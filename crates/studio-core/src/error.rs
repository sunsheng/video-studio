//! 错误契约。
//!
//! **每个错误都必须给出 remedy**——一句「接下来调什么」。
//! [`StudioError::remedy`] 是穷尽 match，不允许 `_ =>` 兜底：
//! 新增一个变体而忘了写补救路径，编译就过不去。
//!
//! 这条规则的由来：前身项目返回过一句
//! `-32602: task already claimed: stage.script.v1`，
//! 不含任何下一步提示，直接导致 Agent 放弃协议、改去写 SQL。

use crate::stage::StageId;
use serde::{Deserialize, Serialize};
use std::fmt;

pub type Result<T> = std::result::Result<T, StudioError>;

/// schema 校验的单条违规。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    /// JSON 指针式路径，例如 `script.story_arc[2].duration_seconds`。
    pub path: String,
    pub message: String,
}

impl Violation {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Violation {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StudioError {
    /// 产物不符合阶段 schema。
    SchemaViolation {
        stage: StageId,
        violations: Vec<Violation>,
    },
    /// 状态机不允许这个动作。
    InvalidTransition {
        stage: StageId,
        current: &'static str,
        attempted: &'static str,
        allowed: Vec<&'static str>,
    },
    /// 有门的阶段提交时没带确认问题。
    ConfirmationRequired { stage: StageId, gate: &'static str },
    /// 门还挂着，不能推进。
    GatePending { stage: StageId, question_id: String },
    /// 应答的选项不在候选里。
    UnknownAnswer {
        question_id: String,
        given: String,
        options: Vec<String>,
    },
    /// 前置阶段还没通过。
    StageNotReady { stage: StageId, blocked_on: StageId },
    /// `.studio/` 被外部改动，完整性校验失败。
    StateDrift { detail: String },
    /// 另一个进程持有本 bundle。
    ProjectBusy {
        pid: Option<u32>,
        since: Option<String>,
    },
    /// 当前目录不是一个 bundle。
    NotAProject { path: String },
    /// 没有可用的 ComfyUI 节点。
    ComfyUnavailable { tried: Vec<String> },
    /// ComfyUI 侧执行失败。
    ComfyFailed { node: String, detail: String },
    /// 固定模型缺失或校验失败——停止，不静默替换。
    ModelContractViolation { detail: String },
    /// 登记过的产物在磁盘上不存在。
    ArtifactMissing { path: String },
    /// 外部程序找不到（ffmpeg / ffprobe）。
    ToolUnavailable {
        tool: String,
        looked_in: Vec<String>,
    },
    /// 阶段重试到顶。
    RetryLimitExceeded { stage: StageId, limit: u32 },
    /// 请求重试的阶段不是当前真正卡住/待执行的那个确定性阶段。
    RetryStageMismatch {
        requested: StageId,
        current: Option<StageId>,
    },
    /// 内部错误：I/O、序列化等。
    Internal { detail: String },
}

impl StudioError {
    /// 稳定的机器可读错误码。文档里的错误表由它生成。
    pub fn code(&self) -> &'static str {
        match self {
            StudioError::SchemaViolation { .. } => "schema_violation",
            StudioError::InvalidTransition { .. } => "invalid_transition",
            StudioError::ConfirmationRequired { .. } => "confirmation_required",
            StudioError::GatePending { .. } => "gate_pending",
            StudioError::UnknownAnswer { .. } => "unknown_answer",
            StudioError::StageNotReady { .. } => "stage_not_ready",
            StudioError::StateDrift { .. } => "state_drift",
            StudioError::ProjectBusy { .. } => "project_busy",
            StudioError::NotAProject { .. } => "not_a_project",
            StudioError::ComfyUnavailable { .. } => "comfy_unavailable",
            StudioError::ComfyFailed { .. } => "comfy_failed",
            StudioError::ModelContractViolation { .. } => "model_contract_violation",
            StudioError::ArtifactMissing { .. } => "artifact_missing",
            StudioError::ToolUnavailable { .. } => "tool_unavailable",
            StudioError::RetryLimitExceeded { .. } => "retry_limit_exceeded",
            StudioError::RetryStageMismatch { .. } => "retry_stage_mismatch",
            StudioError::Internal { .. } => "internal",
        }
    }

    /// 下一步能做什么。**穷尽 match，不允许通配**。
    pub fn remedy(&self) -> String {
        match self {
            StudioError::SchemaViolation { stage, .. } => format!(
                "调 studio.schema(\"{stage}\") 取回该阶段的输出契约，按上面列出的路径修正后重新 studio.submit_stage。"
            ),
            StudioError::InvalidTransition { allowed, .. } => {
                if allowed.is_empty() {
                    "调 studio.status() 看当前该谁行动。".to_string()
                } else {
                    format!("当前状态只允许：{}。先调 studio.status() 确认，再选其中一个。", allowed.join("、"))
                }
            }
            StudioError::ConfirmationRequired { stage, gate } => format!(
                "阶段 {stage} 有确认门 {gate}，提交时必须同时给出 confirmation：\
                 {{prompt, selection_type, options[]}}。补上后重新 studio.submit_stage。"
            ),
            StudioError::GatePending { question_id, .. } => format!(
                "确认门 {question_id} 还挂着。要用户确认就调 studio.answer(\"{question_id}\", <选项 id>)；\
                 要改内容就调 studio.revise(stage, message)——revise 会释放门并回到可提交状态。"
            ),
            StudioError::UnknownAnswer { question_id, options, .. } => format!(
                "选项必须是 {} 之一。重新调 studio.answer(\"{question_id}\", <其中一个>)。",
                options.join(" / ")
            ),
            StudioError::StageNotReady { blocked_on, .. } => format!(
                "前置阶段 {blocked_on} 还没通过。调 studio.status() 拿到 next_action，按它指的阶段先做。"
            ),
            StudioError::StateDrift { detail } => format!(
                ".studio/ 的状态被外部改动过（{detail}）。这个目录是服务端私有的，不要直接读写。\
                 用 studio.timeline() 核对事件历史；确实需要回退就调 studio.undo(stage)。"
            ),
            StudioError::ProjectBusy { pid, .. } => {
                let who = match pid {
                    Some(p) => format!("进程 {p}"),
                    None => "另一个进程".to_string(),
                };
                format!(
                    "本作品已被{who}打开。一个 bundle 同时只能有一个会话：\
                     关掉那个会话后重试，或用 `studiod init <另一个路径>` 新开一部作品。"
                )
            }
            StudioError::NotAProject { path } => format!(
                "{path} 不是一部作品。用 `studiod init <路径>` 新建一部，然后在那个目录里打开 Codex。"
            ),
            StudioError::ComfyUnavailable { tried } => format!(
                "没有健康的 ComfyUI 节点（试过 {}）。在 .env 里配好 COMFY_NODES 后，\
                 调 studio.status() 确认当前卡在 preview 还是 render，再对它调 \
                 studio.retry_stage(<该阶段>) 让控制面重新尝试——它会先停掉可能还在跑的 \
                 worker，也会当场重新读取 `.env`，不需要重启 MCP 进程，也不需要在这部作品 \
                 之外手动跑 `studiod` 子命令；节点恢复前不要降级换模型。",
                if tried.is_empty() { "无".to_string() } else { tried.join("、") }
            ),
            StudioError::ComfyFailed { node, detail } => format!(
                "节点 {node} 执行失败（{detail}）。用 studio.timeline() 看这一镜的历史；\
                 内容本身没问题、只是这次执行失败了（节点抖动、超时）就调 studio.status() \
                 确认当前卡在 preview 还是 render，再对它调 studio.retry_stage(<该阶段>)——\
                 它会先停掉可能还在跑的 worker 再干净重试。怀疑是这个节点本身有问题，可以先调 \
                 studio.comfy.exclude_node 把它排除掉。只有内容/提示词本身要改才用 \
                 studio.revise。"
            ),
            StudioError::ModelContractViolation { detail } => format!(
                "固定模型契约不满足（{detail}）。这是硬停止，不允许静默替换成 pruned/量化变体。\
                 跑 `studiod doctor` 看清缺哪个文件、该放到哪个节点上；补齐后调 \
                 studio.revise(\"render\", <说明>) 重做。用户明确批准降级前不要换系列。"
            ),
            StudioError::ArtifactMissing { path } => format!(
                "登记过的产物 {path} 不在磁盘上。调 studio.revise(stage, <原因>) 重做产出该文件的阶段。"
            ),
            StudioError::ToolUnavailable { tool, looked_in } => format!(
                "找不到 {tool}（找过：{}）。在 bundle 或程序目录的 .env 里配 {}_PATH 指向可执行文件，\
                 或把它放进 PATH；配好后跑 `studiod doctor` 验证。",
                if looked_in.is_empty() { "PATH".to_string() } else { looked_in.join("、") },
                tool.to_uppercase()
            ),
            StudioError::RetryLimitExceeded { stage, limit } => format!(
                "阶段 {stage} 已重试 {limit} 次。先用 studio.timeline() 看清失败原因，\
                 改掉输入后再 studio.revise(\"{stage}\", <说明>)；不要继续盲目重试。"
            ),
            StudioError::RetryStageMismatch { requested: _, current } => match current {
                Some(c) => format!(
                    "调 studio.status() 确认当前卡在哪个阶段，再对准确的阶段调 \
                     studio.retry_stage(\"{c}\")——传的阶段必须是当前正卡着的那个，\
                     不能是别的阶段，否则实际重跑的是当前阶段，跟传的名字对不上。"
                ),
                None => "作品所有阶段都已通过，没有正在等待或失败的确定性阶段可以重试。\
                         调 studio.status() 确认。"
                    .to_string(),
            },
            StudioError::Internal { detail } => format!(
                "内部错误（{detail}）。调 studio.status() 确认状态是否完好；\
                 若状态可用则重试该操作，否则把 .studio/logs/studiod.log 提给维护者。"
            ),
        }
    }

    /// 面向人的一句话描述。
    pub fn message(&self) -> String {
        match self {
            StudioError::SchemaViolation { stage, violations } => {
                let list = violations
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("阶段 {stage} 的产物不符合契约：{list}")
            }
            StudioError::InvalidTransition {
                stage,
                current,
                attempted,
                ..
            } => format!("阶段 {stage} 当前处于 {current}，不能执行 {attempted}"),
            StudioError::ConfirmationRequired { stage, gate } => {
                format!("阶段 {stage} 的确认门 {gate} 需要确认问题")
            }
            StudioError::GatePending { stage, question_id } => {
                format!("阶段 {stage} 正在等待确认（{question_id}）")
            }
            StudioError::UnknownAnswer { given, .. } => format!("无效的选项：{given}"),
            StudioError::StageNotReady { stage, blocked_on } => {
                format!("阶段 {stage} 依赖尚未通过的 {blocked_on}")
            }
            StudioError::StateDrift { detail } => format!("状态完整性校验失败：{detail}"),
            StudioError::ProjectBusy { pid, since } => match (pid, since) {
                (Some(p), Some(s)) => format!("作品已被进程 {p} 打开（自 {s}）"),
                (Some(p), None) => format!("作品已被进程 {p} 打开"),
                _ => "作品已被另一个进程打开".to_string(),
            },
            StudioError::NotAProject { path } => format!("{path} 不是一部作品"),
            StudioError::ComfyUnavailable { .. } => "没有可用的 ComfyUI 节点".to_string(),
            StudioError::ComfyFailed { node, detail } => {
                format!("ComfyUI 节点 {node} 执行失败：{detail}")
            }
            StudioError::ModelContractViolation { detail } => format!("模型契约不满足：{detail}"),
            StudioError::ArtifactMissing { path } => format!("产物缺失：{path}"),
            StudioError::ToolUnavailable { tool, .. } => format!("找不到外部程序：{tool}"),
            StudioError::RetryLimitExceeded { stage, limit } => {
                format!("阶段 {stage} 超过重试上限 {limit}")
            }
            StudioError::RetryStageMismatch { requested, current } => match current {
                Some(c) => format!("请求重试 {requested}，但当前阶段是 {c}"),
                None => format!("请求重试 {requested}，但作品已经全部完成"),
            },
            StudioError::Internal { detail } => format!("内部错误：{detail}"),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        StudioError::Internal {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code(), self.message())
    }
}

impl std::error::Error for StudioError {}

/// 全部错误码 —— `emit-assets` 用它生成文档里的错误表。
pub const ERROR_CODES: [(&str, &str); 17] = [
    ("schema_violation", "产物不符合阶段 schema，附字段路径"),
    ("invalid_transition", "状态机不允许，附当前状态与合法动作"),
    ("confirmation_required", "有门的阶段提交时没带确认问题"),
    ("gate_pending", "确认门还挂着，不能推进"),
    ("unknown_answer", "应答的选项不在候选里"),
    ("stage_not_ready", "前置阶段还没通过"),
    ("state_drift", ".studio/ 被外部改动，完整性校验失败"),
    ("project_busy", "另一进程持有本 bundle，附 PID"),
    ("not_a_project", "当前目录不是一部作品"),
    ("comfy_unavailable", "无健康 ComfyUI 节点，结构化阻塞不降级"),
    ("comfy_failed", "ComfyUI 侧执行失败"),
    (
        "model_contract_violation",
        "固定模型缺失或校验失败，停止不静默替换",
    ),
    ("artifact_missing", "登记的产物在磁盘上不存在"),
    ("tool_unavailable", "找不到 ffmpeg / ffprobe 等外部程序"),
    ("retry_limit_exceeded", "阶段重试到顶"),
    (
        "retry_stage_mismatch",
        "请求重试的阶段不是当前真正卡住的那个确定性阶段",
    ),
    ("internal", "I/O、序列化等内部错误"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn one_of_each() -> Vec<StudioError> {
        vec![
            StudioError::SchemaViolation {
                stage: StageId::Script,
                violations: vec![Violation::new("script.title", "缺失")],
            },
            StudioError::InvalidTransition {
                stage: StageId::Script,
                current: "awaiting_confirmation",
                attempted: "submit",
                allowed: vec!["answer", "revise"],
            },
            StudioError::ConfirmationRequired {
                stage: StageId::Script,
                gate: "script.approval",
            },
            StudioError::GatePending {
                stage: StageId::Script,
                question_id: "script.approval".into(),
            },
            StudioError::UnknownAnswer {
                question_id: "script.approval".into(),
                given: "x".into(),
                options: vec!["approve".into()],
            },
            StudioError::StageNotReady {
                stage: StageId::Render,
                blocked_on: StageId::PromptPack,
            },
            StudioError::StateDrift {
                detail: "digest 不匹配".into(),
            },
            StudioError::ProjectBusy {
                pid: Some(42),
                since: Some("2026-09-03T00:00:00Z".into()),
            },
            StudioError::ProjectBusy {
                pid: None,
                since: None,
            },
            StudioError::NotAProject {
                path: "/tmp/x".into(),
            },
            StudioError::ComfyUnavailable {
                tried: vec!["127.0.0.1:9001".into()],
            },
            StudioError::ComfyUnavailable { tried: vec![] },
            StudioError::ComfyFailed {
                node: "9001".into(),
                detail: "OOM".into(),
            },
            StudioError::ModelContractViolation {
                detail: "缺 vae".into(),
            },
            StudioError::ArtifactMissing {
                path: "media/sh01.mp4".into(),
            },
            StudioError::ToolUnavailable {
                tool: "ffmpeg".into(),
                looked_in: vec![".env".into()],
            },
            StudioError::ToolUnavailable {
                tool: "ffprobe".into(),
                looked_in: vec![],
            },
            StudioError::RetryLimitExceeded {
                stage: StageId::Render,
                limit: 3,
            },
            StudioError::RetryStageMismatch {
                requested: StageId::Render,
                current: Some(StageId::Preview),
            },
            StudioError::RetryStageMismatch {
                requested: StageId::Render,
                current: None,
            },
            StudioError::Internal {
                detail: "boom".into(),
            },
        ]
    }

    /// 硬规则：没有 remedy 的错误视为实现缺陷。
    #[test]
    fn every_error_carries_a_remedy() {
        for e in one_of_each() {
            let r = e.remedy();
            assert!(r.len() > 10, "{} 的 remedy 太短，等于没说：{r:?}", e.code());
            assert!(!e.message().is_empty(), "{} 缺少 message", e.code());
        }
    }

    /// remedy 必须指向一个能调的工具，而不是泛泛而谈。
    #[test]
    fn remedy_points_at_a_tool_or_command() {
        for e in one_of_each() {
            let r = e.remedy();
            let actionable = r.contains("studio.") || r.contains("studiod ") || r.contains(".env");
            assert!(
                actionable,
                "{} 的 remedy 没给出可执行的下一步：{r}",
                e.code()
            );
        }
    }

    #[test]
    fn codes_are_unique_and_documented() {
        let mut seen = std::collections::HashSet::new();
        for (code, desc) in ERROR_CODES {
            assert!(seen.insert(code), "错误码重复：{code}");
            assert!(!desc.is_empty());
        }
        for e in one_of_each() {
            assert!(seen.contains(e.code()), "{} 未登记进 ERROR_CODES", e.code());
        }
    }
}
