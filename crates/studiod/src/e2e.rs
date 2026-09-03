//! 端到端报告。
//!
//! 端到端测试**不在开发环境跑**：它需要一个真实的 Codex 会话驱动真实的
//! MCP server。所以做法是——生产环境跑完，这里把 `.studio/trace.jsonl`
//! 汇成一份机器可读的报告，带回开发环境分析。
//!
//! 报告要能回答验收标准：每个阶段用了几次调用、修订往返是不是一次过、
//! 有没有出现过不带 remedy 的阻塞、有没有 state_drift。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use studio_core::StageId;
use studio_mcp::trace::{Trace, TraceRecord};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: String,
    pub bundle: String,
    pub total_calls: usize,
    pub failed_calls: usize,
    pub calls_by_tool: BTreeMap<String, usize>,
    pub calls_by_stage: BTreeMap<String, usize>,
    pub errors: Vec<ErrorSighting>,
    /// 每次 revise 之后，到下一次成功 submit 之间用了几次调用。
    pub revise_round_trips: Vec<usize>,
    pub stages_reached: Vec<String>,
    pub verdicts: Vec<Verdict>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSighting {
    pub at: String,
    pub tool: String,
    pub code: String,
    pub remedy_present: bool,
}

pub fn build(bundle: &Path) -> Report {
    let records = Trace::read(bundle);
    let mut calls_by_tool: BTreeMap<String, usize> = BTreeMap::new();
    let mut calls_by_stage: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = Vec::new();

    for r in &records {
        *calls_by_tool.entry(r.tool.clone()).or_default() += 1;
        if let Some(s) = &r.stage {
            *calls_by_stage.entry(s.clone()).or_default() += 1;
        }
        if !r.ok {
            errors.push(ErrorSighting {
                at: r.at.clone(),
                tool: r.tool.clone(),
                code: r.error_code.clone().unwrap_or_else(|| "unknown".into()),
                remedy_present: r.remedy_present.unwrap_or(false),
            });
        }
    }

    let revise_round_trips = measure_revise_round_trips(&records);
    let stages_reached: Vec<String> = StageId::all()
        .filter(|s| calls_by_stage.contains_key(s.as_str()))
        .map(|s| s.as_str().to_string())
        .collect();

    let mut verdicts = Vec::new();

    let no_remedyless = errors.iter().all(|e| e.remedy_present);
    verdicts.push(Verdict {
        name: "每条阻塞都带补救路径".into(),
        passed: no_remedyless,
        detail: if no_remedyless {
            format!("{} 次失败调用，全部带 remedy", errors.len())
        } else {
            let bad: Vec<&str> = errors.iter().filter(|e| !e.remedy_present).map(|e| e.code.as_str()).collect();
            format!("这些错误没给出下一步：{}", bad.join("、"))
        },
    });

    let no_drift = !errors.iter().any(|e| e.code == "state_drift");
    verdicts.push(Verdict {
        name: "状态未被外部改动".into(),
        passed: no_drift,
        detail: if no_drift { "没有出现 state_drift".into() } else { "出现了 state_drift，说明有人绕过 MCP 直接改了 .studio/".into() },
    });

    let worst = revise_round_trips.iter().copied().max().unwrap_or(0);
    let revise_ok = revise_round_trips.iter().all(|n| *n <= 2);
    verdicts.push(Verdict {
        name: "修订往返一次过".into(),
        passed: revise_ok,
        detail: if revise_round_trips.is_empty() {
            "本次会话没有发生修订".into()
        } else {
            format!("{} 次修订，最多用了 {worst} 次调用回到提交", revise_round_trips.len())
        },
    });

    let six = [
        StageId::Idea,
        StageId::Selection,
        StageId::Script,
        StageId::Storyboard,
        StageId::VisualAssets,
        StageId::PromptPack,
    ];
    let reached_all = six.iter().all(|s| calls_by_stage.contains_key(s.as_str()));
    verdicts.push(Verdict {
        name: "提交 ComfyUI 前六个阶段全部走到".into(),
        passed: reached_all,
        detail: format!("走到过：{}", stages_reached.join(" → ")),
    });

    let passed = verdicts.iter().all(|v| v.passed);
    Report {
        generated_at: studio_mcp::trace::now(),
        bundle: bundle.display().to_string(),
        total_calls: records.len(),
        failed_calls: errors.len(),
        calls_by_tool,
        calls_by_stage,
        errors,
        revise_round_trips,
        stages_reached,
        verdicts,
        passed,
    }
}

/// 从每次 `studio.revise` 数到下一次成功的 `studio.submit_stage`。
///
/// 前身项目在这里花了 18 次调用；健康的实现应该是 1 次（紧接着就提交）。
fn measure_revise_round_trips(records: &[TraceRecord]) -> Vec<usize> {
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

pub fn render(r: &Report) -> String {
    let mut s = String::new();
    s.push_str(&format!("端到端报告 · {}\n\n", r.generated_at));
    s.push_str(&format!("  作品      {}\n", r.bundle));
    s.push_str(&format!("  调用总数  {}（失败 {}）\n", r.total_calls, r.failed_calls));
    s.push_str(&format!("  走到阶段  {}\n\n", r.stages_reached.join(" → ")));

    s.push_str("  各工具调用次数\n");
    for (tool, n) in &r.calls_by_tool {
        s.push_str(&format!("    {tool:<24} {n}\n"));
    }
    s.push('\n');

    if !r.errors.is_empty() {
        s.push_str("  遇到的阻塞\n");
        for e in &r.errors {
            s.push_str(&format!(
                "    {} {} → {}{}\n",
                e.at,
                e.tool,
                e.code,
                if e.remedy_present { "" } else { "（没有 remedy）" }
            ));
        }
        s.push('\n');
    }

    s.push_str("  验收\n");
    for v in &r.verdicts {
        s.push_str(&format!("    [{}] {}\n         {}\n", if v.passed { "通过" } else { "未过" }, v.name, v.detail));
    }
    s.push('\n');
    s.push_str(if r.passed { "结论：通过。\n" } else { "结论：未通过，见上面「未过」项。\n" });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_mcp::trace::{Trace, TraceRecord};

    fn rec(tool: &str, stage: Option<&str>, ok: bool, code: Option<&str>, remedy: Option<bool>) -> TraceRecord {
        TraceRecord {
            at: "2026-09-03T00:00:00.000Z".into(),
            tool: tool.into(),
            stage: stage.map(String::from),
            ok,
            error_code: code.map(String::from),
            remedy_present: remedy,
            waiting_on: None,
            duration_ms: 1,
        }
    }

    fn bundle_with(records: Vec<TraceRecord>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let t = Trace::at(d.path());
        for r in &records {
            t.append(r);
        }
        d
    }

    #[test]
    fn a_clean_six_stage_run_passes() {
        let mut rs = Vec::new();
        for s in ["idea", "selection", "script", "storyboard", "visual_assets", "prompt_pack"] {
            rs.push(rec("studio.submit_stage", Some(s), true, None, None));
            rs.push(rec("studio.answer", Some(s), true, None, None));
        }
        let d = bundle_with(rs);
        let r = build(d.path());
        assert!(r.passed, "{:?}", r.verdicts);
        assert_eq!(r.stages_reached.len(), 6);
    }

    #[test]
    fn a_blocking_error_without_remedy_fails_the_run() {
        let d = bundle_with(vec![
            rec("studio.submit_stage", Some("idea"), true, None, None),
            rec("studio.submit_stage", None, false, Some("gate_pending"), Some(false)),
        ]);
        let r = build(d.path());
        assert!(!r.passed);
        let v = r.verdicts.iter().find(|v| v.name.contains("补救")).unwrap();
        assert!(!v.passed);
        assert!(v.detail.contains("gate_pending"));
    }

    #[test]
    fn state_drift_fails_the_run() {
        let d = bundle_with(vec![rec("studio.status", None, false, Some("state_drift"), Some(true))]);
        let r = build(d.path());
        let v = r.verdicts.iter().find(|v| v.name.contains("外部改动")).unwrap();
        assert!(!v.passed);
    }

    /// 健康的修订应当是「revise 之后紧接着 submit」。
    #[test]
    fn a_one_pass_revise_is_measured_as_one_call() {
        let d = bundle_with(vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            rec("studio.revise", Some("script"), true, None, None),
            rec("studio.submit_stage", Some("script"), true, None, None),
        ]);
        let r = build(d.path());
        assert_eq!(r.revise_round_trips, vec![1]);
        assert!(r.verdicts.iter().find(|v| v.name.contains("修订")).unwrap().passed);
    }

    /// 前身项目那种「改一次要绕十几步」的形状必须被判为未过。
    #[test]
    fn a_long_detour_after_revise_fails() {
        let mut rs = vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            rec("studio.revise", Some("script"), true, None, None),
        ];
        for _ in 0..5 {
            rs.push(rec("studio.status", None, true, None, None));
        }
        rs.push(rec("studio.submit_stage", Some("script"), true, None, None));
        let d = bundle_with(rs);
        let r = build(d.path());
        assert_eq!(r.revise_round_trips, vec![6]);
        assert!(!r.verdicts.iter().find(|v| v.name.contains("修订")).unwrap().passed);
    }

    #[test]
    fn an_empty_trace_reports_missing_stages_rather_than_crashing() {
        let d = tempfile::tempdir().unwrap();
        let r = build(d.path());
        assert_eq!(r.total_calls, 0);
        assert!(!r.passed);
        assert!(render(&r).contains("未通过"));
    }
}
