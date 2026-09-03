//! 执行侧报告：ComfyUI 调度与后期。
//!
//! 跟 `e2e` 那份是**两份独立的报告**，因为读者不同：
//!
//! - `e2e report` 看协作：Agent 走了哪些阶段、修订往返几次、有没有绕过 MCP。
//! - `exec report` 看吞吐：哪个镜头排在哪个节点、GPU 等了多久、后期哪一步慢。
//!
//! 数据来自 `.studio/exec.jsonl`，由控制面的后台执行线程逐步写入。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use studio_engine::{ExecRecord, ExecRecorder};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShotRow {
    pub shot_id: String,
    pub node: Option<String>,
    pub prompt_id: Option<String>,
    /// 选节点。
    pub pick_ms: u64,
    /// 提交 workflow。
    pub submit_ms: u64,
    /// 排队 + GPU 渲染。整条流水线的大头通常在这里。
    pub render_ms: u64,
    /// 下载产物。
    pub download_ms: u64,
    pub total_ms: u64,
    pub ok: bool,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeLoad {
    pub node: String,
    pub shots: usize,
    pub render_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepRow {
    pub stage: String,
    pub step: String,
    pub calls: usize,
    pub total_ms: u64,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: String,
    pub bundle: String,
    pub has_data: bool,
    pub total_ms: u64,
    pub render_ms: u64,
    pub post_ms: u64,
    pub review_ms: u64,
    pub shots: Vec<ShotRow>,
    pub nodes: Vec<NodeLoad>,
    pub steps: Vec<StepRow>,
    pub failures: Vec<ExecRecord>,
    /// 并行度：同一批镜头分散在几个节点上。
    pub nodes_used: usize,
    pub passed: bool,
}

pub fn build(bundle: &Path) -> Report {
    let records = ExecRecorder::read(bundle);
    let mut r = Report {
        generated_at: studio_mcp::trace::now(),
        bundle: bundle.display().to_string(),
        has_data: !records.is_empty(),
        ..Default::default()
    };

    let mut shots: BTreeMap<String, ShotRow> = BTreeMap::new();
    let mut nodes: BTreeMap<String, NodeLoad> = BTreeMap::new();
    let mut steps: BTreeMap<(String, String), StepRow> = BTreeMap::new();

    for rec in &records {
        r.total_ms += rec.duration_ms;
        match rec.stage.as_str() {
            "render" => r.render_ms += rec.duration_ms,
            "post" => r.post_ms += rec.duration_ms,
            "review" => r.review_ms += rec.duration_ms,
            _ => {}
        }

        let key = (rec.stage.clone(), rec.step.clone());
        let entry = steps.entry(key).or_insert_with(|| StepRow {
            stage: rec.stage.clone(),
            step: rec.step.clone(),
            ok: true,
            ..Default::default()
        });
        entry.calls += 1;
        entry.total_ms += rec.duration_ms;
        entry.ok &= rec.ok;
        if entry.detail.is_none() && !rec.extra.is_empty() {
            entry.detail = Some(
                rec.extra
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }

        if let Some(shot_id) = &rec.shot_id {
            let row = shots.entry(shot_id.clone()).or_insert_with(|| ShotRow {
                shot_id: shot_id.clone(),
                ok: true,
                ..Default::default()
            });
            match rec.step.as_str() {
                "pick_node" => row.pick_ms += rec.duration_ms,
                "submit" => row.submit_ms += rec.duration_ms,
                "render" => row.render_ms += rec.duration_ms,
                "download" => row.download_ms += rec.duration_ms,
                _ => {}
            }
            row.total_ms += rec.duration_ms;
            if rec.node.is_some() {
                row.node = rec.node.clone();
            }
            if rec.prompt_id.is_some() {
                row.prompt_id = rec.prompt_id.clone();
            }
            if !rec.ok {
                row.ok = false;
                row.error_code = rec.error_code.clone();
            }
        }

        if let Some(node) = &rec.node {
            let load = nodes.entry(node.clone()).or_insert_with(|| NodeLoad {
                node: node.clone(),
                ..Default::default()
            });
            if rec.step == "render" {
                load.shots += 1;
                load.render_ms += rec.duration_ms;
            }
        }

        if !rec.ok {
            r.failures.push(rec.clone());
        }
    }

    r.shots = shots.into_values().collect();
    r.shots.sort_by(|a, b| a.shot_id.cmp(&b.shot_id));
    r.nodes = nodes.into_values().filter(|n| n.shots > 0).collect();
    r.nodes.sort_by_key(|n| std::cmp::Reverse(n.render_ms));
    r.nodes_used = r.nodes.len();
    r.steps = steps.into_values().collect();
    r.steps.sort_by_key(|s| std::cmp::Reverse(s.total_ms));
    r.passed = r.has_data && r.failures.is_empty();
    r
}

pub fn render(r: &Report) -> String {
    if !r.has_data {
        return format!(
            "执行侧报告 · {}\n\n  作品 {}\n\n  这部作品还没跑过确定性阶段（渲染 / 后期 / 验收）。\n  \
             提示词包确认之后控制面会自动开始，跑完再来看这份报告。\n  \
             Agent 那一侧的报告用 `studiod e2e report`。\n",
            r.generated_at, r.bundle
        );
    }

    let mut s = format!("执行侧报告 · {}\n\n", r.generated_at);
    s.push_str(&format!("  作品      {}\n", r.bundle));
    s.push_str(&format!(
        "  执行耗时  {}（渲染 {} · 后期 {} · 验收 {}）\n",
        ms(r.total_ms),
        ms(r.render_ms),
        ms(r.post_ms),
        ms(r.review_ms)
    ));
    s.push_str(&format!("  节点      用了 {} 个\n\n", r.nodes_used));

    if !r.shots.is_empty() {
        s.push_str("  逐镜头\n");
        s.push_str("    镜头     选节点    提交      渲染        下载      节点\n");
        for x in &r.shots {
            s.push_str(&format!(
                "    {:<8} {:>8} {:>8} {:>10} {:>9}  {}{}\n",
                x.shot_id,
                ms(x.pick_ms),
                ms(x.submit_ms),
                ms(x.render_ms),
                ms(x.download_ms),
                x.node.as_deref().unwrap_or("-"),
                if x.ok {
                    String::new()
                } else {
                    format!("  ← {}", x.error_code.as_deref().unwrap_or("失败"))
                }
            ));
        }
        s.push('\n');
    }

    if !r.nodes.is_empty() {
        s.push_str("  节点负载\n");
        for n in &r.nodes {
            s.push_str(&format!(
                "    {:<28} {} 个镜头 · {}\n",
                n.node,
                n.shots,
                ms(n.render_ms)
            ));
        }
        s.push('\n');
    }

    s.push_str("  各步骤耗时\n");
    for st in &r.steps {
        s.push_str(&format!(
            "    {:<8} {:<14} {:>3} 次 {:>10}{}\n",
            st.stage,
            st.step,
            st.calls,
            ms(st.total_ms),
            st.detail
                .as_ref()
                .map(|d| format!("  {d}"))
                .unwrap_or_default()
        ));
    }
    s.push('\n');

    if !r.failures.is_empty() {
        s.push_str("  失败\n");
        for f in &r.failures {
            s.push_str(&format!(
                "    {} {}/{} {} → {}\n",
                f.at,
                f.stage,
                f.step,
                f.shot_id.as_deref().unwrap_or("-"),
                f.error_code.as_deref().unwrap_or("未知")
            ));
        }
        s.push('\n');
    }

    s.push_str(if r.passed {
        "结论：执行链路全部成功。\n"
    } else {
        "结论：有失败，见上面「失败」。\n"
    });
    s
}

fn ms(v: u64) -> String {
    crate::e2e::human_ms(v as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use studio_engine::ExecRecord;

    fn rec(
        stage: &str,
        step: &str,
        shot: Option<&str>,
        node: Option<&str>,
        ms: u64,
        ok: bool,
    ) -> ExecRecord {
        ExecRecord {
            at: "2026-09-03T00:00:00.000Z".into(),
            stage: stage.into(),
            step: step.into(),
            shot_id: shot.map(String::from),
            node: node.map(String::from),
            prompt_id: shot.map(|s| format!("p-{s}")),
            duration_ms: ms,
            ok,
            error_code: if ok {
                None
            } else {
                Some("comfy_failed".into())
            },
            extra: serde_json::Map::new(),
        }
    }

    fn bundle_with(records: Vec<ExecRecord>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = ExecRecorder::at(d.path());
        for x in &records {
            r.append(x);
        }
        d
    }

    #[test]
    fn a_bundle_without_execution_says_so_instead_of_pretending() {
        let d = tempfile::tempdir().unwrap();
        let r = build(d.path());
        assert!(!r.has_data);
        assert!(!r.passed);
        let text = render(&r);
        assert!(text.contains("还没跑过确定性阶段"));
        assert!(text.contains("studiod e2e report"), "要指明另一份报告在哪");
    }

    #[test]
    fn per_shot_timings_split_into_four_steps() {
        let d = bundle_with(vec![
            rec("render", "pick_node", Some("sh01"), None, 30, true),
            rec(
                "render",
                "submit",
                Some("sh01"),
                Some("http://n1:9001"),
                120,
                true,
            ),
            rec(
                "render",
                "render",
                Some("sh01"),
                Some("http://n1:9001"),
                48_000,
                true,
            ),
            rec(
                "render",
                "download",
                Some("sh01"),
                Some("http://n1:9001"),
                900,
                true,
            ),
        ]);
        let r = build(d.path());
        let s = &r.shots[0];
        assert_eq!(
            (s.pick_ms, s.submit_ms, s.render_ms, s.download_ms),
            (30, 120, 48_000, 900)
        );
        assert_eq!(s.total_ms, 49_050);
        assert_eq!(s.node.as_deref(), Some("http://n1:9001"));
        assert_eq!(s.prompt_id.as_deref(), Some("p-sh01"));
        assert!(s.ok);
    }

    /// GPU 时间是大头，报告要能一眼看出来。
    #[test]
    fn render_dominates_and_is_attributed_per_node() {
        let d = bundle_with(vec![
            rec(
                "render",
                "render",
                Some("sh01"),
                Some("http://n1:9001"),
                40_000,
                true,
            ),
            rec(
                "render",
                "render",
                Some("sh02"),
                Some("http://n2:9002"),
                30_000,
                true,
            ),
            rec(
                "render",
                "render",
                Some("sh03"),
                Some("http://n1:9001"),
                20_000,
                true,
            ),
            rec("post", "concat", None, None, 3_000, true),
        ]);
        let r = build(d.path());
        assert_eq!(r.nodes_used, 2);
        assert_eq!(r.nodes[0].node, "http://n1:9001");
        assert_eq!(r.nodes[0].shots, 2);
        assert_eq!(r.nodes[0].render_ms, 60_000);
        assert_eq!(r.render_ms, 90_000);
        assert_eq!(r.post_ms, 3_000);
        // 步骤按耗时排序，最慢的在最前
        assert_eq!(r.steps[0].step, "render");
    }

    #[test]
    fn a_failed_step_is_listed_and_marks_the_shot() {
        let d = bundle_with(vec![
            rec(
                "render",
                "render",
                Some("sh01"),
                Some("http://n1:9001"),
                5_000,
                true,
            ),
            rec(
                "render",
                "render",
                Some("sh02"),
                Some("http://n2:9002"),
                900,
                false,
            ),
        ]);
        let r = build(d.path());
        assert!(!r.passed);
        assert_eq!(r.failures.len(), 1);
        let sh02 = r.shots.iter().find(|s| s.shot_id == "sh02").unwrap();
        assert!(!sh02.ok);
        assert_eq!(sh02.error_code.as_deref(), Some("comfy_failed"));
        assert!(render(&r).contains("失败"));
    }

    #[test]
    fn post_step_details_ride_along() {
        let mut c = rec("post", "concat", None, None, 2_500, true);
        c.extra.insert("stream_copied".into(), json!(true));
        c.extra.insert("parts".into(), json!(5));
        let d = bundle_with(vec![c]);
        let r = build(d.path());
        let step = r.steps.iter().find(|s| s.step == "concat").unwrap();
        assert!(step
            .detail
            .as_deref()
            .unwrap()
            .contains("stream_copied=true"));
        assert!(render(&r).contains("stream_copied=true"));
    }
}
