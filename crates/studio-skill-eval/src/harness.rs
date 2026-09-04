//! 真实 `studiod` 子进程 + stdio JSON-RPC。
//!
//! 脚本场景和后续的 Agent 场景共用同一套连接方式——一个真正编译出的
//! 二进制、一个真正的 stdio 管道，跟生产环境 Codex 连接 `studiod` 的方式
//! 完全一致。这跟 `crates/studio-mcp/tests/protocol.rs` 里 in-process 直接
//! 构造 `Server` 的写法不一样：那种写法验证协议逻辑更快，但验证不了
//! "编译出来的二进制真的能被拉起、cwd 发现真的有效"这类只有真实进程边界
//! 才会暴露的问题——`crates/studiod/tests/smoke.rs` 已经在补这一层，这里
//! 复用同样的连接方式，图的是场景脚本和以后的 Agent driver 能共享。

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use studio_core::StageId;

/// 编译出的 `studiod` 二进制路径。
///
/// `studio-skill-eval` 跟 `studiod` 是平级 crate，不能像 `crates/studiod/
/// tests/smoke.rs` 那样用 `CARGO_BIN_EXE_studiod`——那个环境变量只在
/// "二进制和测试属于同一个包"时由 Cargo 提供。这里改成从当前测试/调用
/// 进程自己的路径反推整个 workspace 共用的 `target/<profile>/` 目录：
/// 只要跑过一次 `cargo build -p studiod`（或者任何一次
/// `cargo test --workspace`，它会先把整个 workspace 构建完），二进制就
/// 已经落在同一个目录里。
pub fn studiod_binary() -> PathBuf {
    let mut dir = std::env::current_exe().expect("current_exe() 应该总能拿到");
    dir.pop(); // 去掉当前二进制自己的文件名
    if dir.ends_with("deps") {
        dir.pop();
    }
    let name = if cfg!(windows) {
        "studiod.exe"
    } else {
        "studiod"
    };
    let path = dir.join(name);
    assert!(
        path.is_file(),
        "找不到 {}——先跑一次 `cargo build -p studiod`（或 `cargo test --workspace`，\
         它会先把整个 workspace 构建完再跑测试）。",
        path.display()
    );
    path
}

/// 一个真实的 `studiod` 子进程，连着一个临时 bundle。
pub struct Harness {
    _dir: Option<tempfile::TempDir>,
    pub root: PathBuf,
    child: Child,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Harness {
    /// 起一个全新的、刚初始化好的 bundle，连一个真实 `studiod` 子进程上去。
    pub fn fresh() -> Harness {
        let dir = tempfile::tempdir().expect("建临时目录失败");
        let root = dir.path().join("场景.studio");
        studio_engine::init_project(
            &root,
            studio_core::fixtures::TITLE,
            env!("CARGO_PKG_VERSION"),
            &[],
        )
        .expect("init_project 失败");
        let mut h = Harness::attach(root);
        h._dir = Some(dir);
        h
    }

    /// 连一个 `studiod` 子进程到某个已存在的 bundle 上——用来测试并发打开
    /// 同一个 bundle 这种需要多个进程、但共用同一份磁盘状态的场景。
    pub fn attach(root: PathBuf) -> Harness {
        let mut child = Command::new(studiod_binary())
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("studiod 启动失败");
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Harness {
            _dir: None,
            root,
            child,
            stdout,
            next_id: 0,
        }
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params
        });
        let mut line = req.to_string();
        line.push('\n');
        self.child
            .stdin
            .as_mut()
            .expect("stdin 应该还开着")
            .write_all(line.as_bytes())
            .expect("写 stdin 失败");
        let mut resp = String::new();
        self.stdout
            .read_line(&mut resp)
            .expect("读不到 studiod 的响应");
        serde_json::from_str(&resp).unwrap_or_else(|e| panic!("响应不是合法 JSON（{e}）：{resp}"))
    }

    /// 调用一个工具，返回 (结构化载荷, 是否报错)。
    pub fn call(&mut self, name: &str, args: Value) -> (Value, bool) {
        let resp = self.rpc(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        );
        let result = &resp["result"];
        (
            result["structuredContent"].clone(),
            result["isError"].as_bool().unwrap_or(false),
        )
    }

    /// 提交某阶段的样例产物；有确认门时自动选 approve。
    pub fn advance(&mut self, stage: StageId) {
        let (env, err) = self.submit(stage);
        assert!(!err, "提交 {stage} 失败：{env}");
        if let Some(q) = env["pending_question"].as_object() {
            let qid = q["question_id"].as_str().unwrap().to_string();
            let (_, err) = self.call(
                "studio.answer",
                serde_json::json!({ "question_id": qid, "answer": "approve" }),
            );
            assert!(!err, "确认 {stage} 失败");
        }
    }

    pub fn submit(&mut self, stage: StageId) -> (Value, bool) {
        let mut args = serde_json::json!({
            "outputs": studio_core::fixtures::outputs(stage),
            "summary": studio_core::fixtures::summary(stage),
        });
        if let Some(c) = studio_core::fixtures::confirmation(stage) {
            args["confirmation"] = serde_json::to_value(c).unwrap();
        }
        self.call("studio.submit_stage", args)
    }

    /// 读取这个 bundle 目前为止的调用留痕——跟生产环境 `studio-cli e2e
    /// report` 读的是同一份 `.studio/trace.jsonl`。
    pub fn trace(&self) -> Vec<studio_mcp::trace::TraceRecord> {
        studio_mcp::trace::Trace::read(&self.root)
    }

    /// 这个子进程真实的操作系统 PID——用来验证 `project_busy` 报的持有者
    /// 是不是这一个进程，而不是随便一个数字。
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
