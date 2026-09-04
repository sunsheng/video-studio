//! 裁判：判断一次场景跑下来算不算过。
//!
//! `structural` 是纯规则裁判，不需要任何 LLM，操作的只是 `.studio/
//! trace.jsonl` 里的调用留痕——跟 `studio-cli e2e report` 判的是同一类
//! 事情（协议合规），但这里的是场景库自己要用的独立小工具函数，不是
//! 完整的报告结构；`studio-skill-eval` 不能反向依赖 `studio-cli`。
//!
//! 语义层面的裁判（LLM 读 SKILL.md 的职责条款去评产物质量）留给 Agent
//! 场景，见 ADR-0003。

pub mod structural;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}
