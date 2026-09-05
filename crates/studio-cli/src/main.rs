//! video-studio 的人类操作 + 开发者工具二进制。
//!
//! **不出现在 Codex/Agent 的执行环境里。** `studiod`（MCP server）是唯一
//! 被 Codex 自动拉起的二进制，没有子命令；这个二进制上的一切——建作品、
//! 体检、打包、生成随包文档、留痕报告——都是给人和 CI 用的。
//! 见 `docs/decisions/ADR-0002`。

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};
use studio_cli::{assets, doctor, e2e, exec_report, html, list, pack, quality, rollout};

#[derive(Parser)]
#[command(
    name = "studio-cli",
    version,
    about = "video-studio 的人类操作 + 开发者工具",
    long_about = "建作品、体检环境、打包分享，以及给开发者用的随包文档生成、\n\
                  留痕报告、workflow 基线校验。跑 `studio-cli doctor` 体检。"
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
    /// 列出作品。目录就是事实源，没有中央注册表
    List {
        /// 在哪些目录下面找，默认当前目录
        paths: Vec<PathBuf>,
        /// 往下找几层，默认 2
        #[arg(long, default_value_t = 2)]
        depth: usize,
        #[arg(long)]
        json: bool,
    },
    /// 打包成单个 .dvs 文件。**默认不带媒体**
    Pack {
        bundle: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// 连媒体一起打包（成片和中间片段，通常很大）
        #[arg(long)]
        media: bool,
    },
    /// 解包成一个作品目录
    Unpack {
        archive: PathBuf,
        #[arg(long)]
        into: PathBuf,
    },
    /// 对一部作品跑质量闸：禁用词、物理事实、身份锁一致性
    Quality {
        /// 作品目录，默认当前目录
        #[arg(long)]
        bundle: Option<PathBuf>,
        /// 只看一个阶段。跨阶段的身份锁检查照跑
        #[arg(long)]
        stage: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Agent 侧的端到端留痕
    #[command(subcommand)]
    E2e(E2eCommand),
    /// 执行侧留痕：ComfyUI 调度与后期
    #[command(subcommand)]
    Exec(ExecCommand),
    /// 已验证 workflow 基线相关
    #[command(subcommand)]
    Workflows(WorkflowCommand),
}

#[derive(Subcommand)]
enum ExecCommand {
    /// 汇总 ComfyUI 调度与后期各步骤的耗时。与 e2e 是两份独立的报告：
    /// 那份看协作，这份看吞吐
    Report {
        /// 作品目录，默认当前目录
        #[arg(long)]
        bundle: Option<PathBuf>,
        /// 写 JSON 报告到文件
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// 同时生成单文件 HTML 报告
        #[arg(long)]
        html: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    /// 逐份检查基线：是不是 API 格式、绑定指向的节点存不存在、有没有核验过
    Check {
        /// 基线目录，默认程序目录下的 assets/workflows
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum E2eCommand {
    /// 把作品的调用留痕汇成报告，带回开发环境分析
    Report {
        /// 作品目录，默认当前目录
        #[arg(long)]
        bundle: Option<PathBuf>,
        /// 写 JSON 报告到文件；不给就打印人读的摘要
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// 同时生成单文件 HTML 报告
        #[arg(long)]
        html: Option<PathBuf>,
        /// 合并 Codex 的会话记录（rollout jsonl），带出 token 用量、
        /// 读过哪些 Skill、有没有绕过 MCP——这些 MCP server 看不见
        #[arg(long)]
        rollout: Option<PathBuf>,
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

/// 二进制自己所在的目录——随包分发的 assets、config.toml、.env，以及
/// 同目录下的 `studiod`，都在这里找。
fn program_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

/// `studiod`（MCP server）的路径——**不是**这个二进制自己的路径。
/// 两个二进制随生产环境一起分发，约定并排放在同一个目录里。
fn studiod_path() -> String {
    let name = if cfg!(windows) {
        "studiod.exe"
    } else {
        "studiod"
    };
    match program_dir() {
        Some(dir) => dir.join(name).display().to_string(),
        None => name.to_string(),
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Init { path, title } => cmd_init(&path, title.as_deref()),
        Command::Doctor { json, fix } => cmd_doctor(json, fix),
        Command::EmitAssets { out, check } => cmd_emit(&out, check),
        Command::List { paths, depth, json } => cmd_list(paths, depth, json),
        Command::Pack { bundle, out, media } => cmd_pack(&bundle, &out, media),
        Command::Unpack { archive, into } => cmd_unpack(&archive, &into),
        Command::Quality {
            bundle,
            stage,
            json,
        } => cmd_quality(bundle, stage, json),
        Command::E2e(E2eCommand::Report {
            bundle,
            out,
            html,
            rollout,
        }) => cmd_e2e(bundle, out, html, rollout),
        Command::Exec(ExecCommand::Report { bundle, out, html }) => cmd_exec(bundle, out, html),
        Command::Workflows(WorkflowCommand::Check { dir }) => cmd_workflows_check(dir),
    }
}

fn cmd_init(path: &Path, title: Option<&str>) -> Result<(), String> {
    let title = title
        .map(String::from)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "未命名作品".to_string());

    let settings = studio_engine::Settings::load(program_dir().as_deref(), None);
    let files = assets::bundle_files(
        &studiod_path(),
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
    println!("然后直接说你想拍什么。想看环境是否就绪，先跑一次 `studio-cli doctor`。");
    Ok(())
}

fn cmd_doctor(json: bool, fix: bool) -> Result<(), String> {
    let bundle = studio_engine::Bundle::discover(cwd()).ok();
    let bundle_root = bundle.as_ref().map(|b| b.root().to_path_buf());

    if fix {
        let Some(root) = &bundle_root else {
            return Err("--fix 需要在一部作品目录里运行。".to_string());
        };
        doctor::fix_codex_config(root, &studiod_path())
            .map_err(|e| format!("修正配置失败：{e}"))?;
        println!(
            "已把 {}/.codex/config.toml 指向 {}",
            root.display(),
            studiod_path()
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
            "assets 与代码不一致：\n  {}\n\n跑 `studio-cli emit-assets` 重新生成。",
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

fn cmd_list(paths: Vec<PathBuf>, depth: usize, json: bool) -> Result<(), String> {
    let roots = if paths.is_empty() { vec![cwd()] } else { paths };
    let entries = list::scan(&roots, depth);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", list::render(&entries));
    }
    Ok(())
}

fn cmd_exec(
    bundle: Option<PathBuf>,
    out: Option<PathBuf>,
    html_out: Option<PathBuf>,
) -> Result<(), String> {
    let root = resolve_bundle(bundle)?;
    let report = exec_report::build(&root);

    if let Some(path) = &html_out {
        std::fs::write(path, html::render_exec(&report))
            .map_err(|e| format!("写 HTML 失败：{e}"))?;
        println!("HTML 报告已写入 {}", path.display());
    }
    if let Some(path) = &out {
        let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("写报告失败：{e}"))?;
        println!("报告已写入 {}", path.display());
    }
    print!("{}", exec_report::render(&report));
    if report.passed || !report.has_data {
        Ok(())
    } else {
        Err(String::new())
    }
}

fn cmd_quality(bundle: Option<PathBuf>, stage: Option<String>, json: bool) -> Result<(), String> {
    let root = resolve_bundle(bundle)?;
    let only =
        match &stage {
            None => None,
            Some(s) => Some(studio_core::StageId::parse(s).ok_or_else(|| {
                format!("没有叫 {s} 的阶段。阶段名见 assets/AGENTS.md 的阶段表。")
            })?),
        };
    let report = quality::build(&root, only);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", quality::render(&report));
    }
    if report.passed {
        Ok(())
    } else {
        Err(String::new())
    }
}

fn resolve_bundle(bundle: Option<PathBuf>) -> Result<PathBuf, String> {
    match bundle {
        Some(b) => Ok(b),
        None => studio_engine::Bundle::discover(cwd())
            .map(|b| b.root().to_path_buf())
            .map_err(|_| "不在作品目录里。用 --bundle 指定，或 cd 进作品目录。".to_string()),
    }
}

fn cmd_workflows_check(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir
        .or_else(|| program_dir().map(|p| p.join("assets/workflows")))
        .unwrap_or_else(|| PathBuf::from("assets/workflows"));
    if !dir.is_dir() {
        return Err(format!(
            "{} 不存在。基线目录见 assets/workflows/README.md。",
            dir.display()
        ));
    }

    let mut checked = 0;
    let mut unverified = Vec::new();
    let mut broken = Vec::new();

    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let rel = p.strip_prefix(&dir).unwrap_or(&p).with_extension("");
            let name = rel.to_string_lossy().replace('\\', "/");
            // FORMAT-EXAMPLE 是格式样例，SOURCE-* 是从前身仓库带过来的参考件，
            // 都不是可提交的基线。
            let base = rel
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if base.starts_with("FORMAT-EXAMPLE") || base.starts_with("SOURCE-") {
                continue;
            }
            checked += 1;
            match studio_pipeline::workflow::Workflow::load(&dir, &name) {
                Err(err) => broken.push(format!("{name}: {}", err.message())),
                Ok(wf) => {
                    if let Err(err) = wf.check() {
                        broken.push(format!("{name}: {}", err.message()));
                    } else if !wf.is_verified() {
                        unverified.push(format!(
                            "{name}\n            {}",
                            wf.unavailable_reason().unwrap_or("原因未记录")
                        ));
                    } else {
                        println!("  可用    {name}  参数：{}", wf.parameters().join("、"));
                    }
                }
            }
        }
    }

    for u in &unverified {
        println!("  不可用  {u}");
    }
    for b in &broken {
        println!("  损坏    {b}");
    }
    println!();
    println!(
        "共 {checked} 份：可用 {}，不可用 {}，损坏 {}",
        checked - unverified.len() - broken.len(),
        unverified.len(),
        broken.len()
    );
    if !unverified.is_empty() {
        println!();
        println!("不可用的基线不会被用来渲染——绑错节点会静默产出错的画面。");
        println!("在有 ComfyUI 的机器上跑通之后，补齐 _studio.bindings 并把");
        println!("bindings_verified 改成 true 即可，不需要改代码。见 docs/TODO.md。");
    }
    if broken.is_empty() {
        Ok(())
    } else {
        Err(String::new())
    }
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
        println!(
            "  跳过 {} 个媒体文件。要连媒体一起打包加 --media",
            stats.skipped_media
        );
    }
    Ok(())
}

fn cmd_unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let n = pack::unpack(archive, into).map_err(|e| format!("解包失败：{e}"))?;
    println!("已解出 {} 个文件到 {}", n, into.display());
    println!("提示：换了机器就跑一次 `studio-cli doctor --fix`，把程序路径对上。");
    Ok(())
}

fn cmd_e2e(
    bundle: Option<PathBuf>,
    out: Option<PathBuf>,
    html_out: Option<PathBuf>,
    rollout_path: Option<PathBuf>,
) -> Result<(), String> {
    let root = resolve_bundle(bundle)?;
    let session = match &rollout_path {
        None => None,
        Some(p) => Some(rollout::parse(p).map_err(|e| format!("读 {} 失败：{e}", p.display()))?),
    };
    let report = e2e::build_with(&root, session);

    if let Some(path) = &html_out {
        std::fs::write(path, html::render(&report)).map_err(|e| format!("写 HTML 失败：{e}"))?;
        println!("HTML 报告已写入 {}", path.display());
    }

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
