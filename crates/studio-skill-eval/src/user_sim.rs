//! 虚拟用户：Agent 场景撞上确认门时，由它扮演用户给出下一句回复。
//!
//! 只生成自然语言——回什么由 [`crate::driver::AgentDriver`] 的实现转交
//! 给真实/模拟的 LLM 自己决定怎么调 `studio.answer`/`studio.revise`，
//! 这里不直接拼工具调用，那样就等于脚本自己在做决策，不是在测 Agent。

use serde_json::Value;
use std::collections::HashMap;
use studio_core::StageId;

/// 门上的当前状态：在哪个阶段、有没有待答的问题（`pending_question`
/// envelope，跟 `harness.rs::advance()` 读的是同一个字段）。
pub struct GateState<'a> {
    pub stage: StageId,
    pub pending_question: Option<&'a Value>,
}

pub trait UserSim {
    /// 生成要说给 Agent 听的下一句话。
    fn reply(&mut self, gate: &GateState) -> String;
}

/// 固定剧本：某个阶段该说什么提前写死，可重放、可回归。没配到的阶段
/// 默认说"通过"，避免因为某个阶段没配脚本就让整个场景卡死在轮次上限。
pub struct ScriptedUser {
    script: HashMap<StageId, &'static str>,
}

impl ScriptedUser {
    pub fn new(script: &[(StageId, &'static str)]) -> ScriptedUser {
        ScriptedUser {
            script: script.iter().copied().collect(),
        }
    }
}

impl UserSim for ScriptedUser {
    fn reply(&mut self, gate: &GateState) -> String {
        self.script
            .get(&gate.stage)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "看起来不错，通过，按你的方案继续。".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_lines_are_used_verbatim() {
        let mut u = ScriptedUser::new(&[(StageId::Script, "不要固定2秒，要根据镜头内容智能分配")]);
        let said = u.reply(&GateState {
            stage: StageId::Script,
            pending_question: None,
        });
        assert_eq!(said, "不要固定2秒，要根据镜头内容智能分配");
    }

    #[test]
    fn unscripted_stages_default_to_approval() {
        let mut u = ScriptedUser::new(&[]);
        let said = u.reply(&GateState {
            stage: StageId::Idea,
            pending_question: None,
        });
        assert!(said.contains("通过"));
    }
}
