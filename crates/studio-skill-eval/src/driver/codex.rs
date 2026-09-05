//! `codex exec` 子进程驱动：让真实 Codex 读 skill 文档、自己决定怎么调
//! `studio.*`。
//!
//! 跟 [`super::direct_llm::DirectLlmDriver`] 的角色分工完全不同：那边
//! 我们自己的代码就是 MCP 客户端，直接拿 `Harness` 发调用；这里 Codex
//! 才是 MCP 客户端，它自己拉起 `studiod` 子进程——我们的代码只负责
//! 编排 `codex` 进程、在两轮之间临时挂上去看一眼状态（这时候 Codex 那
//! 一轮已经结束，它自己的 `studiod` 子进程已经退出，锁是空的，挂上去
//! 不会撞见 `project_busy`），事后再解析调用留痕和 Codex 的会话记录。
//!
//! ## 这个 Codex 版本的两个真实约束（CLAUDE.md 已经踩过）
//!
//! 1. 不会自动读 bundle 里的 `.codex/config.toml`——要用 `codex mcp add`
//!    全局注册，见 [`ensure_mcp_registered`]。所以这里不往 bundle 里写
//!    那个文件（写了也没用，还容易让人误以为是它在生效）。
//! 2. rollout jsonl 落在 `$CODEX_HOME/sessions/<Y>/<M>/<D>/
//!    rollout-<时间戳>-<thread_id>.jsonl`，用 `--json` 拿到的
//!    `thread.started` 事件里的 `thread_id` 去找，不能从 `codex exec`
//!    本身的返回值直接拿路径。

use super::{read_decisions, read_gate, AgentDriver, AgentScenario, DriverRun};
use crate::harness::Harness;
use crate::user_sim::{GateState, UserSim};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct CodexDriver {
    /// 只有这次运行自己注册的才在 drop 时移除——如果全局本来就有一份
    /// 指对了地方的注册（比如人手动 `codex mcp add` 过），不该被我们
    /// 顺手删掉。
    owns_registration: bool,
    max_turns: usize,
}

impl CodexDriver {
    /// 24 轮：六个创造性阶段，每个阶段大致"提交一次 + 应一次门"两轮，
    /// 留出余量给修订往返和确定性阶段自动执行后的应对（比如
    /// `retry_vs_revise_confusion_probe` 撞上 `comfy_unavailable` 之后
    /// 还要再走一轮）。
    pub fn new() -> Result<CodexDriver, String> {
        CodexDriver::with_max_turns(24)
    }

    pub fn with_max_turns(max_turns: usize) -> Result<CodexDriver, String> {
        let owns_registration = ensure_mcp_registered(&crate::harness::studiod_binary())?;
        Ok(CodexDriver {
            owns_registration,
            max_turns,
        })
    }
}

impl Drop for CodexDriver {
    fn drop(&mut self) {
        if self.owns_registration {
            let _ = Command::new("codex")
                .args(["mcp", "remove", "video-studio"])
                .output();
        }
    }
}

/// 确保全局注册了名为 `video-studio` 的 MCP server，指向这次要用的
/// `studiod` 二进制。返回值是"这次是不是我们自己注册的"，决定 drop 时
/// 要不要清掉。
fn ensure_mcp_registered(studiod_path: &Path) -> Result<bool, String> {
    let list = Command::new("codex")
        .args(["mcp", "list", "--json"])
        .output()
        .map_err(|e| format!("`codex mcp list` 跑不起来：{e}"))?;
    if !list.status.success() {
        return Err(format!(
            "`codex mcp list` 失败：{}",
            String::from_utf8_lossy(&list.stderr)
        ));
    }
    let servers: Value = serde_json::from_slice(&list.stdout)
        .map_err(|e| format!("`codex mcp list --json` 输出不是合法 JSON：{e}"))?;
    let existing = servers
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["name"] == "video-studio");
    if let Some(existing) = existing {
        let cmd = existing["transport"]["command"]
            .as_str()
            .unwrap_or_default();
        if cmd == studiod_path.to_string_lossy() {
            return Ok(false);
        }
        return Err(format!(
            "全局已经注册了名为 video-studio 的 MCP server，但指向 {cmd}，跟这次要用的 {} \
             不一致——先用 `codex mcp remove video-studio` 清掉再跑。",
            studiod_path.display()
        ));
    }
    let add = Command::new("codex")
        .args([
            "mcp",
            "add",
            "video-studio",
            "--",
            &studiod_path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("`codex mcp add` 跑不起来：{e}"))?;
    if !add.status.success() {
        return Err(format!(
            "`codex mcp add` 失败：{}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }
    Ok(true)
}

/// 把已经生成好的随包文档复制进临时 bundle，让 Codex 能读到跟生产环境
/// 一样的 AGENTS.md/SKILL.md/doctrine/模型能力卡。
///
/// 不调用 `studio-cli::assets` 里的生成函数——那些生成逻辑只该活在
/// `studio-cli`（ADR-0002：`studio-skill-eval` 不能反向依赖它）。
/// `emit-assets --check` 已经保证仓库里 `assets/` 目录跟生成器输出一致，
/// 这里直接复制这份已生成产物，不重新生成一遍。
fn copy_generated_docs_into(bundle_root: &Path) -> Result<(), String> {
    let repo_assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let agents_md = std::fs::read_to_string(repo_assets.join("AGENTS.md")).map_err(|e| {
        format!("读 assets/AGENTS.md 失败：{e}（是不是没在 workspace checkout 里跑？）")
    })?;
    std::fs::write(bundle_root.join("AGENTS.md"), agents_md)
        .map_err(|e| format!("写 AGENTS.md 失败：{e}"))?;

    for sub in ["skills", "doctrine", "models"] {
        copy_tree(
            &repo_assets.join(sub),
            &bundle_root.join(".agents").join(sub),
        )?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(from)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(from)
            .expect("walkdir 给的路径一定在 from 之下");
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
        }
        std::fs::copy(entry.path(), &dest)
            .map_err(|e| format!("复制 {} 失败：{e}", entry.path().display()))?;
    }
    Ok(())
}

fn codex_home() -> PathBuf {
    if let Ok(p) = std::env::var("CODEX_HOME") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".codex")
}

/// 在 `$CODEX_HOME/sessions/` 里找这个 `thread_id` 对应的 rollout 文件。
fn find_rollout_file(thread_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{thread_id}.jsonl");
    walkdir::WalkDir::new(codex_home().join("sessions"))
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with(&suffix))
        .map(|e| e.path().to_path_buf())
}

/// `--json` 流里第一条 `thread.started` 事件的 `thread_id`。
fn extract_thread_id(stdout: &[u8]) -> Option<String> {
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] == "thread.started" {
            return v["thread_id"].as_str().map(String::from);
        }
    }
    None
}

/// 跑一轮 `codex exec`（`resume_id` 为 `None`）或续上一轮
/// （`Some(thread_id)`）。cwd 设成 bundle 目录——这也是它自己拉起的
/// `studiod` 子进程会继承的 cwd，跟真实用户在 bundle 目录里敲 `codex`
/// 是同一件事。
///
/// 带 `--dangerously-bypass-approvals-and-sandbox`：这个判断只在"外层
/// 已经有沙箱"的机器上成立（这里是隔离的远程执行容器），不是通用结论，
/// 见 CLAUDE.md「本地配置 Codex」一节——挪到没有外层沙箱的机器上运行
/// 这份 driver 之前，先重新评估这个假设是否仍然成立。
fn spawn_turn(
    bundle_root: &Path,
    resume_id: Option<&str>,
    prompt: &str,
) -> Result<std::process::Output, String> {
    let mut args: Vec<String> = vec!["exec".into()];
    if resume_id.is_some() {
        args.push("resume".into());
    }
    args.push("--json".into());
    args.push("--skip-git-repo-check".into());
    args.push("--dangerously-bypass-approvals-and-sandbox".into());
    if let Some(id) = resume_id {
        args.push(id.to_string());
    }
    args.push(prompt.to_string());

    Command::new("codex")
        .current_dir(bundle_root)
        .args(&args)
        .output()
        .map_err(|e| format!("`codex {}` 跑不起来：{e}", args.join(" ")))
}

impl AgentDriver for CodexDriver {
    fn run(
        &mut self,
        scenario: &AgentScenario,
        user: &mut dyn UserSim,
    ) -> Result<DriverRun, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("建临时目录失败：{e}"))?;
        let bundle_root = dir.path().join("场景.studio");
        studio_engine::init_project(&bundle_root, scenario.id, env!("CARGO_PKG_VERSION"), &[])
            .map_err(|e| format!("init_project 失败：{e}"))?;
        copy_generated_docs_into(&bundle_root)?;

        let mut thread_id: Option<String> = None;
        let mut prompt = scenario.brief.to_string();
        let mut turns = 0usize;
        let mut reached_stage = None;

        for _ in 0..self.max_turns {
            turns += 1;
            let output = spawn_turn(&bundle_root, thread_id.as_deref(), &prompt)?;
            if let Some(tid) = extract_thread_id(&output.stdout) {
                thread_id = Some(tid);
            }
            if !output.status.success() {
                return Err(format!(
                    "codex exec 第 {turns} 轮失败：{}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }

            let mut h = Harness::attach(bundle_root.clone());
            let (stage, pending) = read_gate(&mut h)?;
            reached_stage = Some(stage);
            if pending.is_none() && stage == scenario.expected_stage {
                break;
            }
            prompt = match &pending {
                Some(q) => user.reply(&GateState {
                    stage,
                    pending_question: Some(q),
                }),
                None => "继续往下推进这部作品。".to_string(),
            };
            drop(h); // 显式落一下：下一轮 codex 要自己拉起 studiod，锁必须先空出来。
        }

        let mut h = Harness::attach(bundle_root.clone());
        let decisions = read_decisions(&mut h);
        drop(h);

        let rollout = thread_id
            .as_deref()
            .and_then(find_rollout_file)
            .and_then(|p| studio_rollout::parse(&p).ok());

        Ok(DriverRun {
            trace: studio_mcp::trace::Trace::read(&bundle_root),
            bundle_root,
            reached_stage,
            turns,
            decisions,
            rollout,
            _dir: Some(dir),
        })
    }
}
