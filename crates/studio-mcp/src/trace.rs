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
    /// 调用时的阶段（尽力而为，来自返回信封）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 出错时是否带上了可执行的补救路径。端到端报告会核对这一列。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    pub duration_ms: u64,
}

pub struct Trace {
    path: std::path::PathBuf,
}

impl Trace {
    pub fn at(bundle_root: &Path) -> Trace {
        Trace { path: bundle_root.join(TRACE_FILE) }
    }

    /// 写失败不影响主流程——留痕是辅助，不是契约。
    pub fn append(&self, rec: &TraceRecord) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(line) = serde_json::to_string(rec) {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    pub fn read(bundle_root: &Path) -> Vec<TraceRecord> {
        let path = bundle_root.join(TRACE_FILE);
        let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
        text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
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
            ok: true,
            error_code: None,
            remedy_present: None,
            waiting_on: Some("user".into()),
            duration_ms: 7,
        });
        t.append(&TraceRecord {
            at: now(),
            tool: "studio.submit_stage".into(),
            stage: Some("script".into()),
            ok: false,
            error_code: Some("gate_pending".into()),
            remedy_present: Some(true),
            waiting_on: None,
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
