//! video-studio 的唯一二进制。
//!
//! **没有任何能改变作品状态的子命令。** 只有 `init`（新建）、`serve`（MCP）、
//! `doctor`（体检）、`emit-assets`（生成随包文档）、`pack` / `unpack`（打包）
//! 和 `e2e report`（汇总端到端留痕）。
//!
//! 这条约束是硬的：状态变更只有 MCP 一个入口，绕过就不存在实现。
//! 前身项目提供了 `cli.py conversation start`，于是 Agent 在卡住的时候
//! 直接跑 CLI 建了 run，绕过了整个协议层。

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};
use studiod::{assets, doctor, e2e, pack};

#[derive(Parser)]
#[command(
    name = "studiod",
    version,
    about = "文档式短视频生产工坊：一个文件夹就是一部作品",
    long_about = "运行本程序的机器不需要 GPU——推理全部经 ComfyUI 的 HTTP API 完成。\n\
                  需要 ffmpeg / ffprobe，但它们不要求在 PATH 中，可在 .env 里指定路径。\n\
                  跑 `studiod doctor` 体检。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 新建一部作品
    Init {
        /// 作品目录，例如 ~/videos/千岛湖.studio
        path: PathBuf,
        /// 作品标题，默认取目录名
        #[arg(long)]
        title: Option<String>,
    },
    /// 以 MCP server 身份运行（由 Codex 拉起，不需要手动执行）
    Serve,
    /// 体检：ffmpeg / ffprobe 与 ComfyUI 节点是否就绪
    Doctor {
        #[arg(long)]
        json: bool,
        /// 修正当前作品 .codex/config.toml 里的程序路径
        #[arg(long)]
        fix: bool,
    },
    /// 生成随包分发的 AGENTS.md、SKILL.md 与 JSON Schema
    EmitAssets {
        #[arg(long, default_value = "assets")]
        out: PathBuf,
        /// 只校验磁盘上的内容是否与代码一致，不写入
        #[arg(long)]
        check: bool,
    },
    /// 打包成单个 .dvs 文件
    Pack {
        bundle: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// 不带媒体，用来做轻量分叉
        #[arg(long)]
        no_media: bool,
    },
    /// 解包成一个作品目录
    Unpack {
        archive: PathBuf,
        #[arg(long)]
        into: PathBuf,
    },
    /// 端到端留痕相关
    #[command(subcommand)]
    E2e(E2eCommand),
}

#[derive(Subcommand)]
enum E2eCommand {
    /// 把作品的调用留痕汇成报告，带回开发环境分析
    Report {
        /// 作品目录，默认当前目录
        #[arg(long)]
        bundle: Option<PathBuf>,
        /// 写到文件（JSON）；不给就打印人读的摘要
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    std::process::exit(code);
}

/// 二进制自己所在的目录——随包分发的 assets、config.toml、.env 都在这里找。
fn program_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

fn program_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "studiod".to_string())
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Init { path, title } => cmd_init(&path, title.as_deref()),
        Command::Serve => cmd_serve(),
        Command::Doctor { json, fix } => cmd_doctor(json, fix),
        Command::EmitAssets { out, check } => cmd_emit(&out, check),
        Command::Pack {
            bundle,
            out,
            no_media,
        } => cmd_pack(&bundle, &out, !no_media),
        Command::Unpack { archive, into } => cmd_unpack(&archive, &into),
        Command::E2e(E2eCommand::Report { bundle, out }) => cmd_e2e(bundle, out),
    }
}

fn cmd_init(path: &Path, title: Option<&str>) -> Result<(), String> {
    let title = title
        .map(String::from)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "未命名作品".to_string());

    let settings = studio_engine::Settings::load(program_dir().as_deref(), None);
    let files = assets::bundle_files(
        &program_path(),
        &title,
        env!("CARGO_PKG_VERSION"),
        &settings.core_model_family(),
    );

    studio_engine::init_project(path, &title, env!("CARGO_PKG_VERSION"), &files)
        .map_err(|e| format!("{e}\n  {}", e.remedy()))?;

    println!("已新建作品：{}", path.display());
    println!("  标题      {title}");
    println!();
    println!("接下来：");
    println!("  cd {}", path.display());
    println!("  codex");
    println!();
    println!("然后直接说你想拍什么。想看环境是否就绪，先跑一次 `studiod doctor`。");
    Ok(())
}

fn cmd_serve() -> Result<(), String> {
    let mut server = studio_mcp::Server::new(&cwd(), program_dir().as_deref());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server
        .serve(stdin.lock(), stdout.lock())
        .map_err(|e| format!("MCP 服务中断：{e}"))
}

fn cmd_doctor(json: bool, fix: bool) -> Result<(), String> {
    let bundle = studio_engine::Bundle::discover(cwd()).ok();
    let bundle_root = bundle.as_ref().map(|b| b.root().to_path_buf());

    if fix {
        let Some(root) = &bundle_root else {
            return Err("--fix 需要在一部作品目录里运行。".to_string());
        };
        doctor::fix_codex_config(root, &program_path())
            .map_err(|e| format!("修正配置失败：{e}"))?;
        println!(
            "已把 {}/.codex/config.toml 指向 {}",
            root.display(),
            program_path()
        );
    }

    let report = doctor::run(program_dir().as_deref(), bundle_root.as_deref());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", doctor::render(&report));
    }
    if report.healthy {
        Ok(())
    } else {
        Err(String::new())
    }
}

fn cmd_emit(out: &Path, check: bool) -> Result<(), String> {
    let files = assets::all_assets();
    if check {
        let mut stale = Vec::new();
        for (rel, content) in &files {
            let p = out.join(rel);
            match std::fs::read_to_string(&p) {
                Ok(disk) if &disk == content => {}
                Ok(_) => stale.push(format!("{} 内容过期", p.display())),
                Err(_) => stale.push(format!("{} 缺失", p.display())),
            }
        }
        if stale.is_empty() {
            println!("assets 与代码一致（{} 个文件）", files.len());
            return Ok(());
        }
        return Err(format!(
            "assets 与代码不一致：\n  {}\n\n跑 `studiod emit-assets` 重新生成。",
            stale.join("\n  ")
        ));
    }
    for (rel, content) in &files {
        let p = out.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
        }
        std::fs::write(&p, content).map_err(|e| format!("写 {} 失败：{e}", p.display()))?;
    }
    println!("已生成 {} 个文件到 {}", files.len(), out.display());
    Ok(())
}

fn cmd_pack(bundle: &Path, out: &Path, include_media: bool) -> Result<(), String> {
    let stats = pack::pack(bundle, out, include_media).map_err(|e| format!("打包失败：{e}"))?;
    println!(
        "已打包 {} 个文件（{:.1} MB）到 {}",
        stats.files,
        stats.bytes as f64 / 1_048_576.0,
        out.display()
    );
    if stats.skipped_media > 0 {
        println!("  跳过 {} 个媒体文件（--no-media）", stats.skipped_media);
    }
    Ok(())
}

fn cmd_unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let n = pack::unpack(archive, into).map_err(|e| format!("解包失败：{e}"))?;
    println!("已解出 {} 个文件到 {}", n, into.display());
    println!("提示：换了机器就跑一次 `studiod doctor --fix`，把程序路径对上。");
    Ok(())
}

fn cmd_e2e(bundle: Option<PathBuf>, out: Option<PathBuf>) -> Result<(), String> {
    let root = match bundle {
        Some(b) => b,
        None => studio_engine::Bundle::discover(cwd())
            .map(|b| b.root().to_path_buf())
            .map_err(|_| "不在作品目录里。用 --bundle 指定，或 cd 进作品目录。".to_string())?,
    };
    let report = e2e::build(&root);
    match out {
        Some(path) => {
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            let mut f = std::fs::File::create(&path).map_err(|e| format!("写报告失败：{e}"))?;
            f.write_all(json.as_bytes())
                .map_err(|e| format!("写报告失败：{e}"))?;
            println!("报告已写入 {}", path.display());
            print!("{}", e2e::render(&report));
        }
        None => print!("{}", e2e::render(&report)),
    }
    if report.passed {
        Ok(())
    } else {
        Err(String::new())
    }
}
