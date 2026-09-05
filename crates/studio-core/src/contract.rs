//! 对外契约类型：决策信封、确认问题、事件。
//!
//! 信封是每个变更型工具的统一返回。`blocked_by` 在阻塞时**必须**填满——
//! 前身项目留着这个字段却从不填，Agent 因此看不出自己被挡住了。

use crate::error::StudioError;
use crate::stage::{Capability, StageId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingOn {
    /// 等 Agent 提交阶段产物。
    Agent,
    /// 等用户回答确认门。
    User,
    /// 等控制面跑完 deterministic 阶段。
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Active,
    Blocked,
    Completed,
}

/// 选项被选中之后会发生什么。
///
/// 让确认门自己说清楚「这个选项是通过还是打回」，Agent 就不必靠 id 的字面
/// 意思去猜。前身项目的门里混着 `approve_script` 和 `revise_script`，
/// 两者都只能走同一个 answer 接口，选中「修改」反而把阶段标成了通过。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 通过，进入下一阶段。
    #[default]
    Approve,
    /// 打回重做：阶段回到草稿，等 Agent 重新提交。
    Revise,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnswerOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub outcome: Outcome,
}

impl AnswerOption {
    /// 一个「通过」选项。
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        AnswerOption {
            id: id.into(),
            label: label.into(),
            outcome: Outcome::Approve,
        }
    }

    /// 一个「打回重做」选项。
    pub fn revise(id: impl Into<String>, label: impl Into<String>) -> Self {
        AnswerOption {
            id: id.into(),
            label: label.into(),
            outcome: Outcome::Revise,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SelectionType {
    #[default]
    Single,
    Multi,
}

/// 提交带门阶段时必须附上的确认问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    pub prompt: String,
    #[serde(default)]
    pub selection_type: SelectionType,
    pub options: Vec<AnswerOption>,
}

/// 挂起中的确认门。`question_id` 由阶段的 gate 决定，稳定不变。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question_id: String,
    pub stage: StageId,
    pub prompt: String,
    pub selection_type: SelectionType,
    pub options: Vec<AnswerOption>,
}

impl Question {
    pub fn accepts(&self, answer: &str) -> bool {
        self.options.iter().any(|o| o.id == answer)
    }

    pub fn option_ids(&self) -> Vec<String> {
        self.options.iter().map(|o| o.id.clone()).collect()
    }

    /// 选中这个选项之后该发生什么。
    pub fn outcome_of(&self, answer: &str) -> Option<Outcome> {
        self.options
            .iter()
            .find(|o| o.id == answer)
            .map(|o| o.outcome)
    }
}

/// 阻塞原因。**remedy 非空是硬要求。**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blocked {
    pub code: String,
    pub message: String,
    pub remedy: String,
}

impl From<&StudioError> for Blocked {
    fn from(e: &StudioError) -> Self {
        Blocked {
            code: e.code().to_string(),
            message: e.message(),
            remedy: e.remedy(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    SubmitStage,
    Answer,
    /// 控制面正在执行，Agent 只需观察。
    Await,
    /// 技术验收已出，还差内容自评。见 `studio-core::rubric`。
    SelfReview,
}

/// 用户做过的一个决定。见 `docs/decisions/ADR-0003`。
///
/// **追加即历史**：记下就不改、不删，`studio.undo` 也不回退它。
/// 按时间倒序给出，同一件事上后面的压过前面的。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub at: String,
    pub stage: StageId,
    pub kind: DecisionKind,
    /// 用户的原话（否决）或选中选项的标签（选择）。控制面不做改写。
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    /// 用户否决了当时的产物，detail 是他说的原话。
    Rejected,
    /// 用户在确认门上选了这一项。
    Chose,
}

impl DecisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionKind::Rejected => "rejected",
            DecisionKind::Chose => "chose",
        }
    }

    pub fn parse(s: &str) -> Option<DecisionKind> {
        match s {
            "rejected" => Some(DecisionKind::Rejected),
            "chose" => Some(DecisionKind::Chose),
            _ => None,
        }
    }
}

/// 下一步该做什么。Agent 只看这个字段就够了。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextAction {
    pub kind: ActionKind,
    pub stage: StageId,
    pub capability: Capability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// 上游阶段的产物，作为本阶段输入。
    pub inputs: Value,
    pub required_outputs: Vec<String>,
    /// 传给 `studio.schema` 的阶段名。
    pub schema_ref: String,
    /// 用户此前否决过什么、在门上选过什么，最近的在前。
    /// 不必翻 timeline，也不指望上下文没被压缩掉。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Decision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub title: String,
    pub stage: StageId,
    pub status: ProjectStatus,
}

/// 决策信封。每个变更型工具的统一返回。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub project: ProjectInfo,
    pub waiting_on: WaitingOn,
    pub blocked_by: Option<Blocked>,
    pub pending_question: Option<Question>,
    pub next_action: Option<NextAction>,
    pub progress: Progress,
    /// 控制面此刻在做什么。只有确定性阶段执行中才有，例如「sh03 提交到 9002」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Envelope {
    /// 信封自洽性：阻塞时必须有 remedy，等用户时必须有问题。
    pub fn assert_consistent(&self) -> Result<(), String> {
        if let Some(b) = &self.blocked_by {
            if b.remedy.trim().is_empty() {
                return Err(format!("blocked_by({}) 没有 remedy", b.code));
            }
        }
        match self.waiting_on {
            WaitingOn::User => {
                if self.pending_question.is_none() && self.blocked_by.is_none() {
                    return Err(
                        "waiting_on=user 但既没有 pending_question 也没有 blocked_by".into(),
                    );
                }
            }
            WaitingOn::Agent => {
                if self.next_action.is_none() && self.blocked_by.is_none() {
                    return Err("waiting_on=agent 但既没有 next_action 也没有 blocked_by".into());
                }
            }
            WaitingOn::System => {}
        }
        Ok(())
    }
}

/// 用户可见的历史事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    pub stage: StageId,
    pub kind: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Question {
        Question {
            question_id: "script.approval".into(),
            stage: StageId::Script,
            prompt: "确认剧本？".into(),
            selection_type: SelectionType::Single,
            options: vec![
                AnswerOption::new("approve", "确认"),
                AnswerOption::new("revise", "修改"),
            ],
        }
    }

    #[test]
    fn question_validates_answers() {
        assert!(q().accepts("approve"));
        assert!(!q().accepts("nope"));
        assert_eq!(q().option_ids(), vec!["approve", "revise"]);
    }

    #[test]
    fn blocked_from_error_always_has_remedy() {
        let e = StudioError::ConfirmationRequired {
            stage: StageId::Script,
            gate: "script.approval",
        };
        let b = Blocked::from(&e);
        assert_eq!(b.code, "confirmation_required");
        assert!(!b.remedy.is_empty());
    }

    #[test]
    fn envelope_rejects_waiting_on_user_without_question() {
        let env = Envelope {
            project: ProjectInfo {
                title: "t".into(),
                stage: StageId::Script,
                status: ProjectStatus::Active,
            },
            waiting_on: WaitingOn::User,
            blocked_by: None,
            pending_question: None,
            next_action: None,
            progress: Progress {
                completed: 2,
                total: 9,
            },
            note: None,
        };
        assert!(env.assert_consistent().is_err());
    }

    #[test]
    fn envelope_accepts_waiting_on_user_with_question() {
        let env = Envelope {
            project: ProjectInfo {
                title: "t".into(),
                stage: StageId::Script,
                status: ProjectStatus::Active,
            },
            waiting_on: WaitingOn::User,
            blocked_by: None,
            pending_question: Some(q()),
            next_action: None,
            progress: Progress {
                completed: 2,
                total: 9,
            },
            note: None,
        };
        assert!(env.assert_consistent().is_ok());
    }
}
