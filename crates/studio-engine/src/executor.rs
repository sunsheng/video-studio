//! 确定性阶段的执行。
//!
//! `render` / `post` / `review` 由控制面执行，Agent 只用 `studio.status` 观察。
//! 这就是工具面上没有 `advance` 的原因——多一个工具就多一种被误用的方式。
//!
//! 具体实现（ComfyUI、ffmpeg）在更上层的 crate 里，这里只定义契约：
//! 引擎负责什么时候跑、跑完怎么落状态、失败了怎么让 Agent 看见。

use crate::config::Settings;
use crate::Bundle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use studio_core::{Outputs, Result, StageId};

/// 执行一个确定性阶段所需的一切。
pub struct ExecContext<'a> {
    pub bundle: &'a Bundle,
    pub settings: &'a Settings,
    /// 上游已通过阶段的产物，键是各阶段的 output_key。
    pub inputs: serde_json::Value,
    /// 进度回报。写进去的字符串会出现在 `studio.status` 的信封里。
    pub progress: &'a ProgressNote,
    /// 被要求停止时变 true，长任务应当在安全点检查它。
    pub cancelled: &'a AtomicBool,
}

impl ExecContext<'_> {
    pub fn say(&self, msg: impl Into<String>) {
        self.progress.set(msg);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// 可共享的一行进度文字。
#[derive(Debug, Default)]
pub struct ProgressNote {
    text: Mutex<Option<String>>,
}

impl ProgressNote {
    pub fn set(&self, msg: impl Into<String>) {
        if let Ok(mut g) = self.text.lock() {
            *g = Some(msg.into());
        }
    }

    pub fn get(&self) -> Option<String> {
        self.text.lock().ok().and_then(|g| g.clone())
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.text.lock() {
            *g = None;
        }
    }
}

/// 三个确定性阶段的实现。
///
/// 由 `studio-pipeline` 提供；引擎只持有 trait 对象，因此不需要依赖
/// ComfyUI 客户端或 ffmpeg——分层由 crate 依赖强制。
pub trait StageExecutor: Send + Sync {
    fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs>;

    /// 是否接线。未接线时控制面根本不启动执行——
    /// 「这个构建没接实现」不该表现成「这部作品出问题了」。
    fn is_wired(&self) -> bool {
        true
    }
}

/// 测试与「还没接线」时用的执行器：什么都不做，直接说自己不可用。
pub struct NotWired;

impl StageExecutor for NotWired {
    fn is_wired(&self) -> bool {
        false
    }

    fn execute(&self, stage: StageId, _ctx: &ExecContext<'_>) -> Result<studio_core::Outputs> {
        Err(studio_core::StudioError::internal(format!(
            "阶段 {stage} 的执行器没有接线。这是构建配置问题，不是作品的问题。"
        )))
    }
}

pub type SharedExecutor = Arc<dyn StageExecutor>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_shareable_and_clearable() {
        let p = Arc::new(ProgressNote::default());
        assert!(p.get().is_none());
        let p2 = Arc::clone(&p);
        std::thread::spawn(move || p2.set("sh03 提交到 9002"))
            .join()
            .unwrap();
        assert_eq!(p.get().as_deref(), Some("sh03 提交到 9002"));
        p.clear();
        assert!(p.get().is_none());
    }

    #[test]
    fn the_unwired_executor_explains_itself() {
        let e = NotWired
            .execute(
                StageId::Render,
                &ExecContext {
                    bundle: &Bundle::scaffold(tempfile::tempdir().unwrap().path()).unwrap(),
                    settings: &Settings::load(None, None),
                    inputs: serde_json::Value::Null,
                    progress: &ProgressNote::default(),
                    cancelled: &AtomicBool::new(false),
                },
            )
            .unwrap_err();
        assert_eq!(e.code(), "internal");
        assert!(e.message().contains("render"));
    }
}
