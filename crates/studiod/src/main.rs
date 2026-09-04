//! video-studio 的 MCP server。
//!
//! **唯一行为是 serve。没有子命令，不接受任何参数**——`--help`/`--version`
//! 是 clap 送的，除此之外一律拒绝。项目管理（`init`/`doctor`/`pack`/
//! `unpack`/`list`）和开发者工具（`emit-assets`/`e2e report`/`exec report`/
//! `workflows check`）都在 `studio-cli` 里，那个二进制不出现在 Codex/Agent
//! 的执行环境里。
//!
//! 这是刻意的：只要这个二进制上挂着任何子命令，就存在"Agent 拿到它、在
//! 沙箱里直接执行子命令绕过 MCP"的路径。子命令列表怎么裁都消不掉这条
//! 路径，只有物理上不存在子命令才行。前身项目的 `cli.py conversation
//! start` 被 Agent 在卡住时直接拿来用，绕过了整个协议层，就是这个模式。
//! 见 `docs/decisions/ADR-0002`。

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "studiod",
    version,
    about = "video-studio 的 MCP server：一个文件夹就是一部作品",
    long_about = "读当前目录找作品，读程序所在目录找随包资源，通过 stdio 提供\n\
                  MCP 协议。由 Codex 根据 .codex/config.toml 自动拉起，不需要\n\
                  手动执行。"
)]
struct Cli;

fn main() {
    let _ = Cli::parse();
    let code = match run() {
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

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn run() -> Result<(), String> {
    // 确定性阶段接上真实实现：门一通过，控制面自己把 render / post / review 跑完。
    let pipeline = std::sync::Arc::new(studio_pipeline::Pipeline::from_program_dir(
        program_dir().as_deref(),
    ));
    let mut server = studio_mcp::Server::with_executor(&cwd(), program_dir().as_deref(), pipeline);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server
        .serve(stdin.lock(), stdout.lock())
        .map_err(|e| format!("MCP 服务中断：{e}"))
}
