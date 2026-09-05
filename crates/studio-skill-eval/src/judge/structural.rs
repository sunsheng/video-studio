//! 结构化裁判：不需要 LLM，纯粹在调用留痕上跑规则。
//!
//! 思路跟 `studio-cli e2e report` 一致（remedy 覆盖、无 state_drift、
//! revise 往返次数、阶段是否都走到），是同一件事的两份独立实现——
//! `studio-skill-eval` 不能反向依赖 `studio-cli`，这里只搬算法，不搬代码。

use super::Verdict;
use studio_core::StageId;
use studio_mcp::trace::TraceRecord;

/// 所有失败调用都必须带 remedy——`blocked_by.remedy` 是硬要求。
pub fn all_blocks_carry_a_remedy(records: &[TraceRecord]) -> Verdict {
    let failures: Vec<&TraceRecord> = records.iter().filter(|r| !r.ok).collect();
    let missing: Vec<&str> = failures
        .iter()
        .filter(|r| !r.remedy_present.unwrap_or(false))
        .map(|r| r.error_code.as_deref().unwrap_or("unknown"))
        .collect();
    Verdict {
        name: "每条阻塞都带补救路径".into(),
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!("{} 次失败调用，全部带 remedy", failures.len())
        } else {
            format!("这些错误没给出下一步：{}", missing.join("、"))
        },
    }
}

/// 不该出现 `state_drift`——那意味着有人绕过 MCP 直接改了 `.studio/`。
pub fn no_state_drift(records: &[TraceRecord]) -> Verdict {
    let drifted = records
        .iter()
        .any(|r| r.error_code.as_deref() == Some("state_drift"));
    Verdict {
        name: "状态未被外部改动".into(),
        passed: !drifted,
        detail: if drifted {
            "出现了 state_drift，说明有人绕过 MCP 直接改了 .studio/".into()
        } else {
            "没有出现 state_drift".into()
        },
    }
}

/// 每次 `revise` 到下一次成功 `submit_stage` 之间用了几次调用——理想值是
/// 1（紧接着就重新提交）。前身项目那次事故是 18。
pub fn revise_round_trips(records: &[TraceRecord]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pending: Option<usize> = None;
    for (i, r) in records.iter().enumerate() {
        if r.tool == "studio.revise" && r.ok {
            pending = Some(i);
        } else if let Some(start) = pending {
            if r.tool == "studio.submit_stage" && r.ok {
                out.push(i - start);
                pending = None;
            }
        }
    }
    out
}

/// 每次修订往返都不超过 `max` 次调用。
pub fn revise_round_trips_within(records: &[TraceRecord], max: usize) -> Verdict {
    let trips = revise_round_trips(records);
    let worst = trips.iter().copied().max().unwrap_or(0);
    let ok = trips.iter().all(|n| *n <= max);
    Verdict {
        name: "修订往返一次过".into(),
        passed: ok,
        detail: if trips.is_empty() {
            "本次场景没有发生修订".into()
        } else {
            format!("{} 次修订，最多用了 {worst} 次调用回到提交", trips.len())
        },
    }
}

/// 给定的这些阶段是否都在留痕里出现过。
pub fn stages_reached(records: &[TraceRecord], expected: &[StageId]) -> Verdict {
    let reached: std::collections::HashSet<&str> =
        records.iter().filter_map(|r| r.stage.as_deref()).collect();
    let missing: Vec<&str> = expected
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !reached.contains(s))
        .collect();
    Verdict {
        name: "预期阶段全部走到".into(),
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!(
                "走到过：{}",
                expected
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            )
        } else {
            format!("没走到：{}", missing.join("、"))
        },
    }
}

/// `blocked_by.remedy` 不点名二进制——它是 Agent 卡住时第一个会读的
/// 通道，比生成的静态文档更容易被当场照着做（ADR-0004 记录的那次真实
/// 缺陷）。`trace.jsonl` 只记调用的形状、不记产物内容，所以这条判断
/// 只能直接在信封（`studio.status`/`tools/call` 的返回值）上做，不是
/// 通过 [`TraceRecord`] 汇总——调用点自己传一个信封 `Value` 进来。
pub fn remedy_does_not_name_binaries(envelope: &serde_json::Value) -> Verdict {
    let remedy = envelope["blocked_by"]["remedy"].as_str().unwrap_or("");
    let leaked: Vec<&str> = ["studiod", "studio-cli"]
        .into_iter()
        .filter(|n| remedy.contains(n))
        .collect();
    Verdict {
        name: "remedy 不点名二进制".into(),
        passed: leaked.is_empty(),
        detail: if leaked.is_empty() {
            format!("remedy 没有提到二进制名：{remedy:?}")
        } else {
            format!("remedy 点了名（{}）：{remedy}", leaked.join("、"))
        },
    }
}
