//! Skill 评估：像测代码一样测 `AGENTS.md` / `SKILL.md`。
//!
//! 只被 `studio-cli` 依赖，不出现在 Codex/Agent 的执行环境里——跟
//! `e2e`/`exec` report 一样是开发者工具。设计见
//! `docs/decisions/ADR-0003-skill-evaluation.md`。
//!
//! 这个 crate 目前只有"脚本场景"：确定性、不需要任何 LLM、能直接进
//! `cargo test --workspace`。"Agent 场景"（真实 LLM 读 skill 文档自己
//! 做决策）是 ADR-0003 里设计好、尚未实现的下一步。

pub mod harness;
pub mod judge;
pub mod scenario;

pub use judge::Verdict;
pub use scenario::{all as all_scenarios, run as run_scenario, ScenarioResult};
