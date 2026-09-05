//! Skill 评估：像测代码一样测 `AGENTS.md` / `SKILL.md`。
//!
//! 只被 `studio-cli` 依赖，不出现在 Codex/Agent 的执行环境里——跟
//! `e2e`/`exec` report 一样是开发者工具。设计见
//! `docs/decisions/ADR-0004-skill-evaluation.md`。
//!
//! 两类场景：
//! - "脚本场景"（[`scenario`]）：确定性、不需要任何 LLM、能直接进
//!   `cargo test --workspace`。
//! - "Agent 场景"（[`agent_scenarios`] + [`driver`] + [`user_sim`]）：
//!   真实 LLM 读 skill 文档自己做决策，不进 CI，本机按需跑。

pub mod agent_scenarios;
pub mod driver;
pub mod harness;
pub mod judge;
pub mod scenario;
pub mod user_sim;

pub use driver::{run_agent_scenario, AgentDriver, AgentScenario, DriverRun};
pub use judge::Verdict;
pub use scenario::{all as all_scenarios, run as run_scenario, ScenarioResult};
pub use user_sim::{GateState, ScriptedUser, UserSim};
