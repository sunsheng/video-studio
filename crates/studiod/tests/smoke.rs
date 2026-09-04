//! 真正编译出的 `studiod` 二进制冒烟测试：起子进程，走一次 MCP 握手。
//!
//! 之前只有 release 工作流的冒烟脚本验证过编译产物真的能被拉起、能完成
//! `initialize` 握手；`cargo test --workspace` 全程只在进程内直接构造
//! `studio_mcp::Server`（见 `crates/studio-mcp/tests/protocol.rs`），从没
//! 跑过真正的二进制、真正的 stdio 管道、真正的 cwd/`current_exe()` 发现
//! 逻辑。换句话说：main.rs 里唯二的两行胶水代码在日常测试里从来没被执行
//! 过，出了问题只能等到打 tag 发版那次冒烟才发现。这里把它补上。

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_studiod")
}

struct Child {
    proc: std::process::Child,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Child {
    fn spawn_in(dir: &std::path::Path) -> Child {
        let mut proc = Command::new(binary())
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("studiod 启动失败");
        let stdout = BufReader::new(proc.stdout.take().unwrap());
        Child { proc, stdout }
    }

    fn rpc(&mut self, body: serde_json::Value) -> serde_json::Value {
        let mut line = body.to_string();
        line.push('\n');
        self.proc
            .stdin
            .as_mut()
            .unwrap()
            .write_all(line.as_bytes())
            .unwrap();
        let mut resp = String::new();
        self.stdout
            .read_line(&mut resp)
            .expect("读不到 studiod 的响应");
        serde_json::from_str(&resp).unwrap_or_else(|e| panic!("响应不是合法 JSON（{e}）：{resp}"))
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.proc.kill();
        let _ = self.proc.wait();
    }
}

#[test]
fn version_flag_reports_a_version_without_starting_the_server() {
    let out = Command::new(binary())
        .arg("--version")
        .output()
        .expect("跑不起来");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("studiod"));
}

/// `studiod` 物理上没有子命令——见 ADR-0002。任何位置参数都该被 clap 拒绝，
/// 而不是被悄悄当成某种隐藏子命令接受。
#[test]
fn an_unknown_positional_argument_is_rejected() {
    let out = Command::new(binary())
        .arg("submit-stage")
        .output()
        .expect("跑不起来");
    assert!(
        !out.status.success(),
        "studiod 不接受任何位置参数，不该有能识别的子命令"
    );
}

#[test]
fn initialize_handshake_succeeds_when_cwd_is_a_real_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    studio_engine::init_project(&root, "冒烟测试", "0.1.0-test", &[]).unwrap();

    let mut child = Child::spawn_in(&root);
    let resp = child.rpc(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18" }
    }));
    assert_eq!(resp["result"]["serverInfo"]["name"], "video-studio");

    let list = child.rpc(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
    }));
    assert_eq!(
        list["result"]["tools"].as_array().unwrap().len(),
        11,
        "编译出的二进制找到的工具面应该跟 studio-mcp::TOOLS 对得上"
    );

    let status = child.rpc(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "studio.status", "arguments": {} }
    }));
    assert_eq!(
        status["result"]["structuredContent"]["project"]["stage"], "idea",
        "cwd 发现逻辑应该找到刚 init 的这个 bundle"
    );
}

/// cwd 不是一部作品时，二进制不该崩，而是照常应答、把 `not_a_project`
/// 结构化地报回来——这条路径此前只在 in-process 的 protocol.rs 里验过，
/// 没验过真正的二进制 + 真正的 `current_dir()`。
#[test]
fn opening_a_directory_that_is_not_a_bundle_still_answers_over_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Child::spawn_in(dir.path());
    let resp = child.rpc(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "studio.status", "arguments": {} }
    }));
    let blocked = &resp["result"]["structuredContent"]["blocked_by"];
    assert_eq!(blocked["code"], "not_a_project");
}
