//! typestate 状态机。
//!
//! 阶段状态编码进类型参数，转换**消耗自身**。这样一类 bug 在编译期就不可表达：
//! 前身项目的 `revise_stage` 把阶段标成 `ready_for_redo` 却不释放任务占用，
//! 紧接着的 `submit_stage` 必然撞上 `task already claimed`——
//! 「用户要求改稿」这条最高频路径因此必然死锁。
//!
//! 这里的做法是：`Stage<AwaitingConfirmation>` 上**根本没有** `submit` 方法，
//! 而 `revise` 消耗掉旧值返回 `Stage<Draft>`，旧状态不可能残留。
//!
//! # 这个 bug 写不出来
//!
//! ```compile_fail
//! use studio_core::state::{AwaitingConfirmation, Stage};
//! fn 不能在门挂着时重新提交(s: Stage<AwaitingConfirmation>) {
//!     // Stage<AwaitingConfirmation> 没有 submit —— 这一行编译不过。
//!     let _ = s.submit(Default::default(), None);
//! }
//! ```
//!
//! ```compile_fail
//! use studio_core::state::{AwaitingConfirmation, Stage};
//! fn 不能在修订后继续用旧状态(s: Stage<AwaitingConfirmation>) {
//!     let _draft = s.revise("不要固定 2 秒");
//!     // s 已经被 move 掉 —— 这一行编译不过。
//!     let _ = s.question();
//! }
//! ```

use crate::contract::{Confirmation, Outcome, Question, SelectionType};
use crate::error::{Result, StudioError, Violation};
use crate::stage::StageId;
use crate::Outputs;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// 草稿：等待 Agent 提交产物。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draft;
/// 已提交，确认门挂起：等待用户应答。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwaitingConfirmation;
/// 已通过：可以推进到下一阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Approved;

/// 可持久化的状态标签。SQLite 里存这个，加载时经 [`LoadedStage`] 还原成 typestate。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    Draft,
    AwaitingConfirmation,
    Approved,
}

impl StageState {
    pub fn as_str(self) -> &'static str {
        match self {
            StageState::Draft => "draft",
            StageState::AwaitingConfirmation => "awaiting_confirmation",
            StageState::Approved => "approved",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stage<S> {
    id: StageId,
    attempt: u32,
    outputs: Option<Outputs>,
    question: Option<Question>,
    _marker: PhantomData<S>,
}

impl<S> Stage<S> {
    pub fn id(&self) -> StageId {
        self.id
    }

    /// 第几次尝试。每次 revise / undo 递增，用于重试上限与审计。
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn outputs(&self) -> Option<&Outputs> {
        self.outputs.as_ref()
    }
}

/// `submit` 的结果：有门则挂起，无门则直接通过。
#[derive(Debug, Clone, PartialEq)]
pub enum Submitted {
    AwaitingConfirmation(Stage<AwaitingConfirmation>),
    Approved(Stage<Approved>),
}

impl Submitted {
    pub fn state(&self) -> StageState {
        match self {
            Submitted::AwaitingConfirmation(_) => StageState::AwaitingConfirmation,
            Submitted::Approved(_) => StageState::Approved,
        }
    }

    pub fn question(&self) -> Option<&Question> {
        match self {
            Submitted::AwaitingConfirmation(s) => Some(s.question()),
            Submitted::Approved(_) => None,
        }
    }

    pub fn outputs(&self) -> Option<&Outputs> {
        match self {
            Submitted::AwaitingConfirmation(s) => s.outputs(),
            Submitted::Approved(s) => s.outputs(),
        }
    }
}

impl Stage<Draft> {
    pub fn new(id: StageId) -> Self {
        Stage {
            id,
            attempt: 1,
            outputs: None,
            question: None,
            _marker: PhantomData,
        }
    }

    /// 从持久化状态还原一个草稿（可能带着上一次尝试留下的产物）。
    pub fn resumed(id: StageId, attempt: u32, outputs: Option<Outputs>) -> Self {
        Stage {
            id,
            attempt,
            outputs,
            question: None,
            _marker: PhantomData,
        }
    }

    /// 提交产物。
    ///
    /// 有确认门的阶段**必须**同时给出 `confirmation`，否则返回
    /// [`StudioError::ConfirmationRequired`]。无门的阶段忽略 `confirmation` 并直接通过。
    ///
    /// 注意签名：消耗 `self`。提交之后就不再存在一个 `Stage<Draft>` 可以重复提交。
    pub fn submit(self, outputs: Outputs, confirmation: Option<Confirmation>) -> Result<Submitted> {
        let gate = self.id.gate();
        match gate {
            Some(gate_id) => {
                let c = confirmation.ok_or(StudioError::ConfirmationRequired {
                    stage: self.id,
                    gate: gate_id,
                })?;
                validate_confirmation(self.id, &c)?;
                let question = Question {
                    question_id: gate_id.to_string(),
                    stage: self.id,
                    prompt: c.prompt,
                    selection_type: c.selection_type,
                    options: c.options,
                };
                Ok(Submitted::AwaitingConfirmation(Stage {
                    id: self.id,
                    attempt: self.attempt,
                    outputs: Some(outputs),
                    question: Some(question),
                    _marker: PhantomData,
                }))
            }
            None => Ok(Submitted::Approved(Stage {
                id: self.id,
                attempt: self.attempt,
                outputs: Some(outputs),
                question: None,
                _marker: PhantomData,
            })),
        }
    }
}

impl Stage<AwaitingConfirmation> {
    /// 还原一个挂起中的门。
    pub fn resumed(id: StageId, attempt: u32, outputs: Outputs, question: Question) -> Self {
        Stage {
            id,
            attempt,
            outputs: Some(outputs),
            question: Some(question),
            _marker: PhantomData,
        }
    }

    pub fn question(&self) -> &Question {
        // 构造函数保证 AwaitingConfirmation 一定带着问题。
        self.question
            .as_ref()
            .expect("AwaitingConfirmation 必然携带 question")
    }

    /// 用户确认通过。
    ///
    /// 选项不在候选里，或选中的是一个「打回重做」选项，都会返回
    /// [`StudioError::UnknownAnswer`]——后者应当走 [`Stage::revise`]。
    pub fn approve(self, answer: &str) -> Result<Stage<Approved>> {
        let q = self.question();
        match q.outcome_of(answer) {
            Some(Outcome::Approve) => {}
            Some(Outcome::Revise) | None => {
                return Err(StudioError::UnknownAnswer {
                    question_id: q.question_id.clone(),
                    given: answer.to_string(),
                    options: q
                        .options
                        .iter()
                        .filter(|o| o.outcome == Outcome::Approve)
                        .map(|o| o.id.clone())
                        .collect(),
                });
            }
        }
        Ok(Stage {
            id: self.id,
            attempt: self.attempt,
            outputs: self.outputs,
            question: None,
            _marker: PhantomData,
        })
    }

    /// 用户要求修订：回到草稿，attempt 递增。
    ///
    /// **不会失败**——修订永远是可行的。占用与门在这次转换里一同消失，
    /// 因为旧的 `Stage<AwaitingConfirmation>` 已经被 move 掉了。
    pub fn revise(self, _message: &str) -> Stage<Draft> {
        Stage {
            id: self.id,
            attempt: self.attempt + 1,
            outputs: self.outputs,
            question: None,
            _marker: PhantomData,
        }
    }
}

impl Stage<Approved> {
    pub fn resumed(id: StageId, attempt: u32, outputs: Option<Outputs>) -> Self {
        Stage {
            id,
            attempt,
            outputs,
            question: None,
            _marker: PhantomData,
        }
    }

    /// 回滚到草稿，重做该阶段。
    pub fn undo(self) -> Stage<Draft> {
        Stage {
            id: self.id,
            attempt: self.attempt + 1,
            outputs: self.outputs,
            question: None,
            _marker: PhantomData,
        }
    }
}

/// 从持久化状态还原成 typestate 的桥。
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedStage {
    Draft(Stage<Draft>),
    Awaiting(Stage<AwaitingConfirmation>),
    Approved(Stage<Approved>),
}

impl LoadedStage {
    pub fn load(
        id: StageId,
        state: StageState,
        attempt: u32,
        outputs: Option<Outputs>,
        question: Option<Question>,
    ) -> Result<LoadedStage> {
        match state {
            StageState::Draft => Ok(LoadedStage::Draft(Stage::<Draft>::resumed(
                id, attempt, outputs,
            ))),
            StageState::Approved => Ok(LoadedStage::Approved(Stage::<Approved>::resumed(
                id, attempt, outputs,
            ))),
            StageState::AwaitingConfirmation => {
                let outputs = outputs.ok_or_else(|| StudioError::StateDrift {
                    detail: format!("阶段 {id} 记为 awaiting_confirmation 却没有产物"),
                })?;
                let question = question.ok_or_else(|| StudioError::StateDrift {
                    detail: format!("阶段 {id} 记为 awaiting_confirmation 却没有确认门"),
                })?;
                Ok(LoadedStage::Awaiting(
                    Stage::<AwaitingConfirmation>::resumed(id, attempt, outputs, question),
                ))
            }
        }
    }

    pub fn state(&self) -> StageState {
        match self {
            LoadedStage::Draft(_) => StageState::Draft,
            LoadedStage::Awaiting(_) => StageState::AwaitingConfirmation,
            LoadedStage::Approved(_) => StageState::Approved,
        }
    }

    pub fn id(&self) -> StageId {
        match self {
            LoadedStage::Draft(s) => s.id(),
            LoadedStage::Awaiting(s) => s.id(),
            LoadedStage::Approved(s) => s.id(),
        }
    }

    pub fn attempt(&self) -> u32 {
        match self {
            LoadedStage::Draft(s) => s.attempt(),
            LoadedStage::Awaiting(s) => s.attempt(),
            LoadedStage::Approved(s) => s.attempt(),
        }
    }

    /// 当前状态下合法的动作，用于 `invalid_transition` 的 remedy。
    pub fn allowed_actions(&self) -> Vec<&'static str> {
        match self {
            LoadedStage::Draft(_) => vec!["studio.submit_stage"],
            LoadedStage::Awaiting(_) => vec!["studio.answer", "studio.revise"],
            LoadedStage::Approved(_) => vec!["studio.undo", "studio.revise"],
        }
    }
}

fn validate_confirmation(stage: StageId, c: &Confirmation) -> Result<()> {
    let mut v = Vec::new();
    if c.prompt.trim().is_empty() {
        v.push(Violation::new(
            "confirmation.prompt",
            "确认问题的正文不能为空",
        ));
    }
    if c.options.is_empty() {
        v.push(Violation::new("confirmation.options", "至少要给出一个选项"));
    }
    if c.selection_type == SelectionType::Single && c.options.len() < 2 {
        v.push(Violation::new(
            "confirmation.options",
            "单选门至少要两个选项，否则用户没得选",
        ));
    }
    if !c.options.iter().any(|o| o.outcome == Outcome::Approve) {
        v.push(Violation::new(
            "confirmation.options",
            "至少要有一个 outcome=approve 的选项，否则这道门永远过不去",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, o) in c.options.iter().enumerate() {
        if o.id.trim().is_empty() {
            v.push(Violation::new(
                format!("confirmation.options[{i}].id"),
                "选项 id 不能为空",
            ));
        }
        if o.label.trim().is_empty() {
            v.push(Violation::new(
                format!("confirmation.options[{i}].label"),
                "选项 label 不能为空",
            ));
        }
        if !seen.insert(o.id.clone()) {
            v.push(Violation::new(
                format!("confirmation.options[{i}].id"),
                format!("选项 id 重复：{}", o.id),
            ));
        }
    }
    if v.is_empty() {
        Ok(())
    } else {
        Err(StudioError::SchemaViolation {
            stage,
            violations: v,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AnswerOption;

    fn conf() -> Confirmation {
        Confirmation {
            prompt: "是否确认这版剧本？".into(),
            selection_type: SelectionType::Single,
            options: vec![
                AnswerOption::new("approve", "确认剧本，进入分镜"),
                AnswerOption::revise("revise", "继续修改"),
            ],
        }
    }

    fn outs(k: &str) -> Outputs {
        let mut m = Outputs::new();
        m.insert(k.into(), serde_json::json!({"ok": true}));
        m
    }

    #[test]
    fn gated_stage_requires_confirmation() {
        let s = Stage::<Draft>::new(StageId::Script);
        let e = s.submit(outs("script"), None).unwrap_err();
        assert_eq!(e.code(), "confirmation_required");
        assert!(e.remedy().contains("confirmation"));
    }

    #[test]
    fn gateless_stage_approves_directly() {
        let s = Stage::<Draft>::new(StageId::Idea);
        let r = s.submit(outs("brief"), None).unwrap();
        assert_eq!(r.state(), StageState::Approved);
        assert!(r.question().is_none());
    }

    #[test]
    fn gated_stage_hangs_the_gate() {
        let s = Stage::<Draft>::new(StageId::Script);
        let r = s.submit(outs("script"), Some(conf())).unwrap();
        assert_eq!(r.state(), StageState::AwaitingConfirmation);
        assert_eq!(r.question().unwrap().question_id, "script.approval");
    }

    #[test]
    fn bad_confirmation_is_a_schema_violation() {
        let mut c = conf();
        c.options.clear();
        let e = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(c))
            .unwrap_err();
        assert_eq!(e.code(), "schema_violation");
    }

    #[test]
    fn single_select_needs_two_options() {
        let mut c = conf();
        c.options.truncate(1);
        let e = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(c))
            .unwrap_err();
        assert_eq!(e.code(), "schema_violation");
    }

    #[test]
    fn duplicate_option_ids_rejected() {
        let mut c = conf();
        c.options[1] = AnswerOption::new("approve", "又一个 approve");
        let e = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(c))
            .unwrap_err();
        assert_eq!(e.code(), "schema_violation");
    }

    #[test]
    fn unknown_answer_is_rejected_with_options() {
        let awaiting = match Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(conf()))
            .unwrap()
        {
            Submitted::AwaitingConfirmation(s) => s,
            _ => panic!("应当挂门"),
        };
        let e = awaiting.approve("怎么都行").unwrap_err();
        assert_eq!(e.code(), "unknown_answer");
        assert!(e.remedy().contains("approve"));
    }

    /// 这就是前身项目那次翻车的完整路径，现在一次走通。
    #[test]
    fn revise_then_resubmit_works_in_one_pass() {
        // 1. 提交每镜头 2 秒的版本，门挂起
        let awaiting = match Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(conf()))
            .unwrap()
        {
            Submitted::AwaitingConfirmation(s) => s,
            _ => panic!("应当挂门"),
        };
        assert_eq!(awaiting.attempt(), 1);

        // 2. 用户说「不要固定 2 秒」——revise 消耗掉挂起状态，无失败可能
        let draft = awaiting.revise("不要固定2秒，要根据镜头内容智能分配");
        assert_eq!(draft.attempt(), 2);

        // 3. 立刻就能重新提交，不存在「task already claimed」
        let again = draft.submit(outs("script"), Some(conf())).unwrap();
        assert_eq!(again.state(), StageState::AwaitingConfirmation);

        // 4. 用户确认，进入下一阶段
        let approved = match again {
            Submitted::AwaitingConfirmation(s) => s.approve("approve").unwrap(),
            _ => panic!("应当挂门"),
        };
        assert_eq!(approved.id().next(), Some(StageId::Storyboard));
    }

    /// 选中「打回重做」的选项不能把阶段标成通过——那正是前身项目的门做过的事。
    #[test]
    fn choosing_a_revise_option_is_not_an_approval() {
        let awaiting = match Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(conf()))
            .unwrap()
        {
            Submitted::AwaitingConfirmation(s) => s,
            _ => panic!("应当挂门"),
        };
        let e = awaiting.approve("revise").unwrap_err();
        assert_eq!(e.code(), "unknown_answer");
        match &e {
            StudioError::UnknownAnswer { options, .. } => {
                assert_eq!(
                    options,
                    &vec!["approve".to_string()],
                    "只应把通过类选项列为候选"
                );
            }
            other => panic!("实际 {other}"),
        }
    }

    #[test]
    fn a_gate_with_no_approving_option_is_rejected() {
        let c = Confirmation {
            prompt: "改还是改？".into(),
            selection_type: SelectionType::Single,
            options: vec![
                crate::contract::AnswerOption::revise("revise_a", "这样改"),
                crate::contract::AnswerOption::revise("revise_b", "那样改"),
            ],
        };
        let e = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(c))
            .unwrap_err();
        assert_eq!(e.code(), "schema_violation");
    }

    #[test]
    fn undo_returns_to_draft_and_bumps_attempt() {
        let approved = match Stage::<Draft>::new(StageId::Idea)
            .submit(outs("brief"), None)
            .unwrap()
        {
            Submitted::Approved(s) => s,
            _ => panic!(),
        };
        let d = approved.undo();
        assert_eq!(d.attempt(), 2);
        assert!(d.outputs().is_some(), "回到草稿时应保留上一版产物供参考");
    }

    #[test]
    fn loaded_stage_rejects_awaiting_without_question() {
        let e = LoadedStage::load(
            StageId::Script,
            StageState::AwaitingConfirmation,
            1,
            Some(outs("script")),
            None,
        )
        .unwrap_err();
        assert_eq!(e.code(), "state_drift");
    }

    #[test]
    fn allowed_actions_match_state() {
        let d = LoadedStage::load(StageId::Script, StageState::Draft, 1, None, None).unwrap();
        assert_eq!(d.allowed_actions(), vec!["studio.submit_stage"]);
        let a = LoadedStage::load(StageId::Idea, StageState::Approved, 1, None, None).unwrap();
        assert!(a.allowed_actions().contains(&"studio.undo"));
    }
}
