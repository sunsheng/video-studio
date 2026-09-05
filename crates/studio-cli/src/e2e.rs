//! 端到端报告。
//!
//! 端到端测试**不在开发环境跑**：它需要一个真实的 Codex 会话驱动真实的
//! MCP server。所以做法是——生产环境跑完，这里把 `.studio/trace.jsonl`
//! 汇成一份机器可读的报告，带回开发环境分析。
//!
//! 报告要能回答验收标准：每个阶段用了几次调用、修订往返是不是一次过、
//! 有没有出现过不带 remedy 的阻塞、有没有 state_drift。

use crate::rollout::Rollout;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use studio_core::StageId;
use studio_mcp::trace::{Trace, TraceRecord};

/// 全流程耗时拆解。**等待用户确认不算进有效耗时**——那是人在想，不是系统在跑。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    /// 从第一次调用开始到最后一次调用结束的墙上时间。
    pub wall_ms: i64,
    /// 扣掉等待用户之后的时间，衡量系统与 Agent 的实际开销。
    pub effective_ms: i64,
    /// 控制面自己处理的时间。
    pub server_ms: i64,
    /// 两次调用之间、且上一次不是在等用户——Agent 在想和在写。
    pub agent_ms: i64,
    /// 挂在确认门上等人的时间。
    pub waiting_user_ms: i64,
}

/// 按 Skill（能力）汇总。这是能观测到的 skill 维度。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillStat {
    pub capability: String,
    pub stages: Vec<String>,
    pub calls: usize,
    pub server_ms: i64,
    pub agent_ms: i64,
    pub waiting_user_ms: i64,
}

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
    pub timing: Timing,
    pub skills: Vec<SkillStat>,
    /// 合并进来的 Codex 会话记录。没给 --rollout 时为 None，
    /// 报告里相应把 token 与绕行两列标成不可观测。
    pub rollout: Option<Rollout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSighting {
    pub at: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub code: String,
    pub remedy_present: bool,
}

pub fn build(bundle: &Path) -> Report {
    build_with(bundle, None)
}

pub fn build_with(bundle: &Path, rollout: Option<Rollout>) -> Report {
    // Agent 会先照着自己的猜测去 cat 一个不存在的路径，再去列目录纠正。
    // 猜错的那几次不算「读过」，留着会让报告以为方法层比实际更被用上。
    let rollout = rollout.map(|mut r| {
        r.doctrine_read
            .retain(|p| bundle.join(".agents").join(p).is_file());
        r
    });
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
                stage: r.stage.clone(),
                code: r.error_code.clone().unwrap_or_else(|| "unknown".into()),
                remedy_present: r.remedy_present.unwrap_or(false),
            });
        }
    }

    let revise_round_trips = studio_mcp::trace::revise_round_trips(&records);
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
            let bad: Vec<&str> = errors
                .iter()
                .filter(|e| !e.remedy_present)
                .map(|e| e.code.as_str())
                .collect();
            format!("这些错误没给出下一步：{}", bad.join("、"))
        },
    });

    let no_drift = !errors.iter().any(|e| e.code == "state_drift");
    verdicts.push(Verdict {
        name: "状态未被外部改动".into(),
        passed: no_drift,
        detail: if no_drift {
            "没有出现 state_drift".into()
        } else {
            "出现了 state_drift，说明有人绕过 MCP 直接改了 .studio/".into()
        },
    });

    let worst = revise_round_trips.iter().copied().max().unwrap_or(0);
    let revise_ok = revise_round_trips.iter().all(|n| *n <= 2);
    verdicts.push(Verdict {
        name: "修订往返一次过".into(),
        passed: revise_ok,
        detail: if revise_round_trips.is_empty() {
            "本次会话没有发生修订".into()
        } else {
            format!(
                "{} 次修订，最多用了 {worst} 次调用回到提交",
                revise_round_trips.len()
            )
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

    let (timing, skills) = measure(&records);

    // 有 rollout 就能多守一条：整个会话不该出现绕过 MCP 的动作。
    if let Some(r) = &rollout {
        verdicts.push(Verdict {
            name: "全程没有绕过 MCP".into(),
            passed: r.bypasses.is_empty(),
            detail: if r.bypasses.is_empty() {
                format!(
                    "Codex 侧 {} 次本地命令，没有一次碰状态库或试图用 CLI 推进阶段",
                    r.calls.shell
                )
            } else {
                format!("发现绕行：{}", r.bypasses.join("；"))
            },
        });
    }

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
        timing,
        skills,
        rollout,
    }
}

/// 从留痕还原耗时。
///
/// 每条记录的 `at` 是调用**结束**的时刻，`duration_ms` 是控制面处理时长，
/// 所以调用开始 ≈ `at - duration`。两次调用之间的空档归给谁，取决于上一次
/// 调用之后轮到谁：`waiting_on == user` 就是人在看，否则是 Agent 在想。
fn measure(records: &[TraceRecord]) -> (Timing, Vec<SkillStat>) {
    let mut t = Timing::default();
    let mut by_cap: BTreeMap<String, SkillStat> = BTreeMap::new();

    let parsed: Vec<(i64, &TraceRecord)> = records
        .iter()
        .filter_map(|r| epoch_ms(&r.at).map(|ms| (ms, r)))
        .collect();

    for (i, (end, r)) in parsed.iter().enumerate() {
        let cap = r.capability.clone().unwrap_or_else(|| "（未知）".into());
        let entry = by_cap.entry(cap.clone()).or_insert_with(|| SkillStat {
            capability: cap,
            ..Default::default()
        });
        entry.calls += 1;
        entry.server_ms += r.duration_ms as i64;
        if let Some(s) = &r.stage {
            if !entry.stages.contains(s) {
                entry.stages.push(s.clone());
            }
        }
        t.server_ms += r.duration_ms as i64;

        if let Some((next_end, next)) = parsed.get(i + 1) {
            let next_start = next_end - next.duration_ms as i64;
            let gap = (next_start - end).max(0);
            if r.waiting_on.as_deref() == Some("user") {
                t.waiting_user_ms += gap;
                entry.waiting_user_ms += gap;
            } else {
                t.agent_ms += gap;
                entry.agent_ms += gap;
            }
        }
    }

    if let (Some((first_end, first)), Some((last_end, _))) = (parsed.first(), parsed.last()) {
        t.wall_ms = (last_end - (first_end - first.duration_ms as i64)).max(0);
        t.effective_ms = (t.wall_ms - t.waiting_user_ms).max(0);
    }

    let mut skills: Vec<SkillStat> = by_cap.into_values().collect();
    skills.sort_by_key(|s| std::cmp::Reverse(s.server_ms + s.agent_ms));
    (t, skills)
}

/// RFC3339 → 毫秒。解析不了就跳过这条，不因为一行坏数据毁掉整份报告。
fn epoch_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

pub fn human_ms(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms} 毫秒")
    } else if ms < 60_000 {
        format!("{:.1} 秒", ms as f64 / 1000.0)
    } else {
        format!("{} 分 {} 秒", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// 从每次 `studio.revise` 数到下一次成功的 `studio.submit_stage`。
///
/// 前身项目在这里花了 18 次调用；健康的实现应该是 1 次（紧接着就提交）。
pub fn render(r: &Report) -> String {
    let mut s = String::new();
    s.push_str(&format!("端到端报告 · {}\n\n", r.generated_at));
    s.push_str(&format!("  作品      {}\n", r.bundle));
    s.push_str(&format!(
        "  调用总数  {}（失败 {}）\n",
        r.total_calls, r.failed_calls
    ));
    s.push_str(&format!("  走到阶段  {}\n", r.stages_reached.join(" → ")));
    s.push_str(&format!(
        "  耗时      有效 {}（墙上 {}，其中等用户 {}）\n",
        human_ms(r.timing.effective_ms),
        human_ms(r.timing.wall_ms),
        human_ms(r.timing.waiting_user_ms)
    ));
    if let Some(ro) = &r.rollout {
        s.push_str(&format!(
            "  token     输入 {} / 输出 {}（推理 {}，命中缓存 {}）\n",
            ro.tokens.input, ro.tokens.output, ro.tokens.reasoning_output, ro.tokens.cached_input
        ));
        s.push_str(&format!(
            "  读过 Skill {}\n",
            if ro.skills_read.is_empty() {
                "（会话里没有读取记录）".to_string()
            } else {
                ro.skills_read.join("、")
            }
        ));
        // 方法层是按需加载的：一份都没读，产出干巴就不能赖到文档头上。
        s.push_str(&format!(
            "  读过方法 {}\n",
            if ro.doctrine_read.is_empty() {
                "（一份都没读——方法层没被用上）".to_string()
            } else {
                ro.doctrine_read.join("、")
            }
        ));
    }
    s.push('\n');

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
                if e.remedy_present {
                    ""
                } else {
                    "（没有 remedy）"
                }
            ));
        }
        s.push('\n');
    }

    s.push_str("  验收\n");
    for v in &r.verdicts {
        s.push_str(&format!(
            "    [{}] {}\n         {}\n",
            if v.passed { "通过" } else { "未过" },
            v.name,
            v.detail
        ));
    }
    s.push('\n');
    s.push_str(if r.passed {
        "结论：通过。\n"
    } else {
        "结论：未通过，见上面「未过」项。\n"
    });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_mcp::trace::{Trace, TraceRecord};

    fn rec(
        tool: &str,
        stage: Option<&str>,
        ok: bool,
        code: Option<&str>,
        remedy: Option<bool>,
    ) -> TraceRecord {
        rec_revised(tool, stage, ok, code, remedy, None)
    }

    /// 带上「这次调用把阶段打回了草稿没有」。修订的判据是它，不是工具名。
    fn rec_revised(
        tool: &str,
        stage: Option<&str>,
        ok: bool,
        code: Option<&str>,
        remedy: Option<bool>,
        revised: Option<bool>,
    ) -> TraceRecord {
        TraceRecord {
            at: "2026-09-03T00:00:00.000Z".into(),
            tool: tool.into(),
            stage: stage.map(String::from),
            capability: stage
                .and_then(studio_core::StageId::parse)
                .map(|s| s.capability().as_str().to_string()),
            ok,
            error_code: code.map(String::from),
            remedy_present: remedy,
            waiting_on: None,
            revised,
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
        for s in [
            "idea",
            "selection",
            "script",
            "storyboard",
            "visual_assets",
            "prompt_pack",
        ] {
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
            rec(
                "studio.submit_stage",
                None,
                false,
                Some("gate_pending"),
                Some(false),
            ),
        ]);
        let r = build(d.path());
        assert!(!r.passed);
        let v = r.verdicts.iter().find(|v| v.name.contains("补救")).unwrap();
        assert!(!v.passed);
        assert!(v.detail.contains("gate_pending"));
    }

    #[test]
    fn state_drift_fails_the_run() {
        let d = bundle_with(vec![rec(
            "studio.status",
            None,
            false,
            Some("state_drift"),
            Some(true),
        )]);
        let r = build(d.path());
        let v = r
            .verdicts
            .iter()
            .find(|v| v.name.contains("外部改动"))
            .unwrap();
        assert!(!v.passed);
    }

    /// 健康的修订应当是「revise 之后紧接着 submit」。
    #[test]
    fn a_one_pass_revise_is_measured_as_one_call() {
        let d = bundle_with(vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            rec_revised(
                "studio.revise",
                Some("script"),
                true,
                None,
                None,
                Some(true),
            ),
            rec("studio.submit_stage", Some("script"), true, None, None),
        ]);
        let r = build(d.path());
        assert_eq!(r.revise_round_trips, vec![1]);
        assert!(
            r.verdicts
                .iter()
                .find(|v| v.name.contains("修订"))
                .unwrap()
                .passed
        );
    }

    /// **issue #17**：门上点 revise 走的是 `studio.answer`，不是 `studio.revise`。
    /// 以前只认工具名，于是这条更常用的路径被系统性漏报——「修订往返一次过」
    /// 那一栏永远显示通过，因为它压根没看见修订。
    #[test]
    fn a_revise_chosen_at_the_gate_is_counted_too() {
        let d = bundle_with(vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            // 门上选了 revise：工具名是 answer，但阶段被打回了草稿
            rec_revised(
                "studio.answer",
                Some("script"),
                true,
                None,
                None,
                Some(true),
            ),
            rec("studio.submit_stage", Some("script"), true, None, None),
            // 这次确认通过，没有打回
            rec_revised(
                "studio.answer",
                Some("script"),
                true,
                None,
                None,
                Some(false),
            ),
        ]);
        let r = build(d.path());
        assert_eq!(r.revise_round_trips, vec![1], "门上那次修订要被数进来");
    }

    /// 反过来：确认通过的 answer 不是修订，不能误报。
    #[test]
    fn an_approving_answer_is_not_a_revision() {
        let d = bundle_with(vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            rec_revised(
                "studio.answer",
                Some("script"),
                true,
                None,
                None,
                Some(false),
            ),
            rec("studio.submit_stage", Some("storyboard"), true, None, None),
        ]);
        let r = build(d.path());
        assert!(r.revise_round_trips.is_empty(), "确认通过不是修订");
    }

    /// 前身项目那种「改一次要绕十几步」的形状必须被判为未过。
    #[test]
    fn a_long_detour_after_revise_fails() {
        let mut rs = vec![
            rec("studio.submit_stage", Some("script"), true, None, None),
            rec_revised(
                "studio.revise",
                Some("script"),
                true,
                None,
                None,
                Some(true),
            ),
        ];
        for _ in 0..5 {
            rs.push(rec("studio.status", None, true, None, None));
        }
        rs.push(rec("studio.submit_stage", Some("script"), true, None, None));
        let d = bundle_with(rs);
        let r = build(d.path());
        assert_eq!(r.revise_round_trips, vec![6]);
        assert!(
            !r.verdicts
                .iter()
                .find(|v| v.name.contains("修订"))
                .unwrap()
                .passed
        );
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
