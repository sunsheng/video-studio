//! Agent 场景驱动：真实 LLM 读 skill 文档、自己决定怎么调用工具面。
//!
//! 跟脚本场景（[`crate::scenario::ScenarioFn`]，固定调用序列）根本
//! 不同：这里由 driver 把一份初始创意交给一个真实 LLM，LLM 自己决定
//! 怎么调 `studio.*`，中途撞上确认门就由 [`crate::user_sim::UserSim`]
//! 扮演用户给出下一句自然语言回复——只有 Agent 自己才能把这句话翻译成
//! 真正的 `studio.answer`/`studio.revise` 调用，这里不代它拼工具调用，
//! 那样就变成脚本自己在做决策，测不出 skill 文档措辞好不好。
//!
//! 跑完之后用调用留痕 + （能拿到就带上的）rollout 交给场景自带的
//! `verdicts` 判定。见 `docs/decisions/ADR-0004-skill-evaluation.md`。

pub mod codex;
pub mod direct_llm;

use crate::harness::Harness;
use crate::judge::Verdict;
use crate::scenario::{finish, ScenarioResult};
use crate::user_sim::UserSim;
use serde_json::Value;
use std::path::PathBuf;
use studio_core::StageId;
use studio_mcp::trace::TraceRecord;

/// 一次 Agent 场景跑完之后，driver 交回来的东西。
pub struct DriverRun {
    /// 只用来保证临时 bundle 活到调用方读完 `stages/*.json` 为止，
    /// 不直接使用——跟 `Harness::_dir` 是同一个理由。
    _dir: Option<tempfile::TempDir>,
    pub bundle_root: PathBuf,
    pub trace: Vec<TraceRecord>,
    pub reached_stage: Option<StageId>,
    pub turns: usize,
    /// 跑完时 `next_action.decisions` 的快照——用来验证决定档案有没有
    /// 真的影响到后面阶段（ADR-0003）。
    pub decisions: Vec<studio_core::Decision>,
    /// token/skills_read/doctrine_read/bypasses——能拿到就填（只有
    /// `CodexDriver` 有真实的 Codex 会话记录），拿不到就是 `None`，
    /// 上层不伪造。
    pub rollout: Option<studio_rollout::Rollout>,
}

/// 一个 Agent 场景：给 LLM 的初始创意、期望走到哪个阶段、场景自己的
/// 判定逻辑。
pub struct AgentScenario {
    pub id: &'static str,
    pub description: &'static str,
    /// 给 Agent 的初始创意/任务描述——就是生产环境里它会收到的那句话，
    /// 不夹带任何测试专用的提示。
    pub brief: &'static str,
    pub expected_stage: StageId,
    /// 场景专属判定：拿 `DriverRun` 判断该验证的事有没有发生。
    pub verdicts: fn(&DriverRun) -> Vec<Verdict>,
}

pub trait AgentDriver {
    fn run(
        &mut self,
        scenario: &AgentScenario,
        user: &mut dyn UserSim,
    ) -> Result<DriverRun, String>;
}

/// 跑一个 Agent 场景：驱动 LLM 到收敛（或达到轮次上限），再叠加
/// "停在期望阶段"这条所有 Agent 场景共用的判定。
pub fn run_agent_scenario(
    scenario: &AgentScenario,
    driver: &mut dyn AgentDriver,
    user: &mut dyn UserSim,
) -> ScenarioResult {
    match driver.run(scenario, user) {
        Ok(run) => {
            let mut verdicts = (scenario.verdicts)(&run);
            verdicts.push(Verdict {
                name: "停在期望阶段".into(),
                passed: run.reached_stage == Some(scenario.expected_stage),
                detail: format!(
                    "期望 {}，实际 {}（{} 轮）",
                    scenario.expected_stage,
                    run.reached_stage
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "未知".into()),
                    run.turns
                ),
            });
            finish(scenario.id, scenario.description, verdicts)
        }
        Err(e) => finish(
            scenario.id,
            scenario.description,
            vec![Verdict {
                name: "driver 正常跑完".into(),
                passed: false,
                detail: e,
            }],
        ),
    }
}

/// 查一次当前状态：在哪个阶段、有没有待答的问题。两个 driver 都要在
/// 每一轮之间问这件事，抽成共用函数，不各写一份。
pub(crate) fn read_gate(h: &mut Harness) -> Result<(StageId, Option<Value>), String> {
    let (env, err) = h.call("studio.status", serde_json::json!({}));
    if err {
        return Err(format!("studio.status 报错：{env}"));
    }
    let stage_str = env["project"]["stage"].as_str().unwrap_or_default();
    let stage = StageId::parse(stage_str).ok_or_else(|| format!("认不出阶段名：{stage_str}"))?;
    let pending = env
        .get("pending_question")
        .filter(|v| !v.is_null())
        .cloned();
    Ok((stage, pending))
}

/// 从 envelope 里取一次 `next_action.decisions` 快照。
pub(crate) fn read_decisions(h: &mut Harness) -> Vec<studio_core::Decision> {
    let (env, err) = h.call("studio.status", serde_json::json!({}));
    if err {
        return Vec::new();
    }
    serde_json::from_value(env["next_action"]["decisions"].clone()).unwrap_or_default()
}
