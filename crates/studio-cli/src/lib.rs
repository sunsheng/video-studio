//! `studio-cli` 的可测试部分。二进制入口见 `main.rs`。
//!
//! 这个 crate **不出现在 Codex/Agent 的执行环境里**——它是人类操作
//! （建/体检/打包作品）和开发者工具（随包文档生成、留痕报告、
//! workflow 基线校验），见 `docs/decisions/ADR-0002`。

pub mod assets;
pub mod doctor;
pub mod e2e;
pub mod exec_report;
pub mod html;
pub mod list;
pub mod pack;
pub mod quality;

/// 解析 Codex rollout jsonl——实际实现在 `studio-rollout`（跟
/// `studio-skill-eval` 共用一份，见 `docs/decisions/ADR-0004-skill-evaluation.md`），
/// 这里转发是为了不动 `e2e`/`main` 里现有的 `rollout::parse`/`rollout::Rollout`
/// 调用点。
pub use studio_rollout as rollout;
