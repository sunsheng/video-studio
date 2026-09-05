//! 对一部真实作品跑质量闸。
//!
//! 规则本身在 `studio-core::quality` 里，和提交时跑的是同一份代码——
//! 这里只负责把作品的产物读出来喂进去，再把结论排版出来。
//! 两处结论不一致就是 bug，不是「口径不同」。
//!
//! 为什么还要有这个命令：提交闸只在**提交那一刻**跑，且只挡
//! blocking。已经躺在库里的产物（提交时规则还没上线、或者只是
//! advisory）没人回头看。CI 和验收要的是「把整部作品从头到尾过一遍」。

use serde::{Deserialize, Serialize};
use std::path::Path;
use studio_core::quality::{self, Finding, Metric, Severity};
use studio_core::{Outputs, StageId, StageState};
use studio_engine::bundle::DB_FILE;
use studio_store::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageReport {
    pub stage: String,
    pub state: String,
    pub findings: Vec<Finding>,
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub title: String,
    pub bundle: String,
    /// 有没有产物可查。空作品不算失败，只是没数据。
    pub has_data: bool,
    pub passed: bool,
    pub blocking: usize,
    pub advisory: usize,
    pub stages: Vec<StageReport>,
    /// 跨阶段的结论（身份锁一致性）。
    pub cross_stage: Vec<Finding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// 跑一遍。`only` 给定时只看那一个阶段（跨阶段检查照跑，否则查不出漂移）。
pub fn build(root: &Path, only: Option<StageId>) -> Report {
    let title = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());
    let mut report = Report {
        title,
        bundle: root.display().to_string(),
        has_data: false,
        passed: true,
        blocking: 0,
        advisory: 0,
        stages: Vec::new(),
        cross_stage: Vec::new(),
        problem: None,
    };

    let store = match Store::open(&root.join(DB_FILE)) {
        Ok(s) => s,
        Err(e) => {
            report.problem = Some(format!("打不开作品状态库：{}", e.message()));
            report.passed = false;
            return report;
        }
    };
    if let Ok(t) = store.title() {
        report.title = t;
    }

    let mut all: Vec<(StageId, Outputs)> = Vec::new();
    for stage in StageId::all() {
        let Ok(loaded) = store.load_stage(stage) else {
            continue;
        };
        let state = loaded.state();
        let outputs = match loaded {
            studio_core::LoadedStage::Draft(s) => s.outputs().cloned(),
            studio_core::LoadedStage::Awaiting(s) => s.outputs().cloned(),
            studio_core::LoadedStage::Approved(s) => s.outputs().cloned(),
        };
        let Some(outputs) = outputs else { continue };
        report.has_data = true;
        all.push((stage, outputs.clone()));

        if only.is_some_and(|o| o != stage) {
            continue;
        }
        let findings = quality::check_stage(stage, &outputs);
        report.blocking += findings
            .iter()
            .filter(|f| f.severity == Severity::Blocking)
            .count();
        report.advisory += findings
            .iter()
            .filter(|f| f.severity == Severity::Advisory)
            .count();
        report.stages.push(StageReport {
            stage: stage.to_string(),
            state: state_name(state).to_string(),
            findings,
            metrics: quality::metrics(stage, &outputs),
        });
    }

    report.cross_stage = quality::check_across_stages(&all);
    report.blocking += report
        .cross_stage
        .iter()
        .filter(|f| f.severity == Severity::Blocking)
        .count();
    report.passed = report.blocking == 0;
    report
}

fn state_name(s: StageState) -> &'static str {
    match s {
        StageState::Draft => "草稿",
        StageState::AwaitingConfirmation => "等确认",
        StageState::Approved => "已通过",
    }
}

pub fn render(r: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!("质量报告  {}\n", r.title));
    out.push_str(&format!("作品      {}\n\n", r.bundle));

    if let Some(p) = &r.problem {
        out.push_str(&format!("  {p}\n"));
        return out;
    }
    if !r.has_data {
        out.push_str("  还没有任何阶段产物，没什么可查的。\n");
        return out;
    }

    for s in &r.stages {
        out.push_str(&format!("{}  ({})\n", s.stage, s.state));
        for m in &s.metrics {
            let mark = match m.met {
                Some(true) => "达标",
                Some(false) => "未达标",
                None => "    ",
            };
            let target = m
                .target
                .as_ref()
                .map(|t| format!("  目标 {t}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "    {mark}  {} {}{}\n",
                pad(&m.name, 28),
                m.value,
                target
            ));
        }
        for f in &s.findings {
            out.push_str(&format!(
                "    {}  {}\n            {}\n",
                severity_mark(f.severity),
                f.path,
                f.message
            ));
        }
        out.push('\n');
    }

    if !r.cross_stage.is_empty() {
        out.push_str("跨阶段\n");
        for f in &r.cross_stage {
            out.push_str(&format!(
                "    {}  {}\n            {}\n",
                severity_mark(f.severity),
                f.path,
                f.message
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "共 {} 条挡提交、{} 条提醒。{}\n",
        r.blocking,
        r.advisory,
        if r.passed {
            "质量闸通过。"
        } else {
            "没通过——挡提交的那些改完再来。"
        }
    ));
    if !r.passed {
        out.push_str("\n每条方括号里是规则名，对应的写法说明在作品的\n");
        out.push_str("  .agents/doctrine/quality/checklist.md\n");
    }
    out
}

/// 按显示宽度补空格。中日韩字符占两列，`{:<18}` 按字符数补会歪。
fn pad(s: &str, width: usize) -> String {
    let w: usize = s.chars().map(char_width).sum();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

fn char_width(c: char) -> usize {
    // 够用就好：CJK 表意文字、全角标点、假名按两列算，其余按一列。
    match c as u32 {
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

fn severity_mark(s: Severity) -> &'static str {
    match s {
        Severity::Blocking => "挡提交",
        Severity::Advisory => "提醒  ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::fixtures;

    /// 建一部作品、把黄金样例逐阶段灌进去，再跑报告。
    /// 这条同时验证了「提交时的闸」和「事后跑的闸」是同一份结论。
    fn bundle_with_fixtures(dir: &Path) {
        studio_engine::init_project(dir, fixtures::TITLE, "0.0.0-test", &[]).unwrap();
        let project = studio_engine::Project::open(dir, None).unwrap();
        for stage in [
            StageId::Idea,
            StageId::Selection,
            StageId::Script,
            StageId::Storyboard,
            StageId::VisualAssets,
            StageId::PromptPack,
        ] {
            let env = project
                .submit_stage(
                    fixtures::outputs(stage),
                    Some(fixtures::summary(stage)),
                    fixtures::confirmation(stage),
                )
                .unwrap_or_else(|e| panic!("提交 {stage} 失败：{}", e.message()));
            if let Some(q) = env.pending_question {
                let approve = q
                    .options
                    .iter()
                    .find(|o| o.outcome == studio_core::contract::Outcome::Approve)
                    .map(|o| o.id.clone())
                    .unwrap();
                project.answer(&q.question_id, &approve).unwrap();
            }
        }
    }

    #[test]
    fn a_bundle_built_from_the_golden_fixtures_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("作品");
        bundle_with_fixtures(&dir);

        let report = build(&dir, None);
        assert!(report.has_data);
        assert_eq!(report.blocking, 0, "{}", render(&report));
        assert!(report.cross_stage.is_empty());
        assert!(report.passed);
    }

    #[test]
    fn an_empty_bundle_is_not_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("空作品");
        studio_engine::init_project(&dir, "空", "0.0.0-test", &[]).unwrap();

        let report = build(&dir, None);
        assert!(!report.has_data);
        assert!(report.passed);
        assert!(render(&report).contains("没什么可查的"));
    }

    #[test]
    fn a_missing_bundle_says_so_instead_of_passing() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build(&tmp.path().join("不存在"), None);
        assert!(!report.passed);
        assert!(report.problem.is_some());
    }

    #[test]
    fn only_one_stage_still_runs_the_cross_stage_check() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("作品");
        bundle_with_fixtures(&dir);

        let report = build(&dir, Some(StageId::Storyboard));
        assert_eq!(report.stages.len(), 1);
        assert_eq!(report.stages[0].stage, "storyboard");
        // 跨阶段那条不受 --stage 影响：漂移是三处之间的关系，
        // 只看一处永远看不出来。
        assert!(report.cross_stage.is_empty());
    }
}
