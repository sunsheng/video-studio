//! 调用留痕。
//!
//! 每次工具调用追加一行到 `.studio/trace.jsonl`。生产环境的端到端验收
//! 靠它出报告：哪个阶段用了几次调用、修订往返是不是一次过、有没有出现过
//! 不带 remedy 的阻塞。开发环境不跑端到端，只在这里定义格式。
//!
//! 只记调用的形状，不记产物内容——产物本身在 `stages/*.json` 里。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

pub const TRACE_FILE: &str = ".studio/trace.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub at: String,
    pub tool: String,
    /// 这次调用作用在哪个阶段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// 该阶段对应的能力，也就是 Skill 名。报告按它汇总。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 出错时是否带上了可执行的补救路径。端到端报告会核对这一列。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    /// 这次调用有没有把 `stage` 打回草稿，也就是**触发了一次修订**。
    ///
    /// 修订有两条路：在确认门上选 revise 类选项（走 `studio.answer`），
    /// 或者用自然语言提意见（走 `studio.revise`）。**前一条更常用**，
    /// 而只按工具名认的话它完全看不见——见 issue #17。
    ///
    /// 所以这一列由控制面记下事实，不让报告去猜：调用前后各看一眼该阶段的
    /// 状态，从非草稿变成草稿就是一次修订。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised: Option<bool>,
    pub duration_ms: u64,
}

/// 每次修订到下一次成功 `submit_stage` 之间用了几次调用——理想值是 1
/// （紧接着就重新提交）。前身项目那次事故是 18。
///
/// **判据是 [`TraceRecord::revised`]，不是工具名。** 按工具名认会漏掉门上
/// 点 revise 那条路，而那条是更常用的——漏报的后果是「修订往返一次过」
/// 这一栏永远显示通过，因为它压根没看见修订（issue #17）。
///
/// 放在这里是因为端到端报告（`studio-cli`）和 Skill 评估
/// （`studio-skill-eval`）都要用它。各写一份的话，某一天有人改了其中一份，
/// 两边的结论就会不一致而没人发现。
pub fn revise_round_trips(records: &[TraceRecord]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pending: Option<usize> = None;
    for (i, r) in records.iter().enumerate() {
        if r.revised == Some(true) {
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

pub struct Trace {
    path: std::path::PathBuf,
}

impl Trace {
    pub fn at(bundle_root: &Path) -> Trace {
        Trace {
            path: bundle_root.join(TRACE_FILE),
        }
    }

    /// 写失败不影响主流程——留痕是辅助，不是契约。
    pub fn append(&self, rec: &TraceRecord) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(line) = serde_json::to_string(rec) {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    pub fn read(bundle_root: &Path) -> Vec<TraceRecord> {
        let path = bundle_root.join(TRACE_FILE);
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_through_jsonl() {
        let d = tempfile::tempdir().unwrap();
        let t = Trace::at(d.path());
        t.append(&TraceRecord {
            at: now(),
            tool: "studio.submit_stage".into(),
            stage: Some("script".into()),
            capability: Some("script".into()),
            ok: true,
            error_code: None,
            remedy_present: None,
            waiting_on: Some("user".into()),
            revised: Some(false),
            duration_ms: 7,
        });
        t.append(&TraceRecord {
            at: now(),
            tool: "studio.submit_stage".into(),
            stage: Some("script".into()),
            capability: Some("script".into()),
            ok: false,
            error_code: Some("gate_pending".into()),
            remedy_present: Some(true),
            waiting_on: None,
            revised: None,
            duration_ms: 1,
        });
        let back = Trace::read(d.path());
        assert_eq!(back.len(), 2);
        assert!(back[0].ok);
        assert_eq!(back[1].error_code.as_deref(), Some("gate_pending"));
        assert_eq!(back[1].remedy_present, Some(true));
    }

    #[test]
    fn reading_a_bundle_without_trace_is_empty_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(Trace::read(d.path()).is_empty());
    }
}
