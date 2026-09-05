//! 确定性阶段的执行。
//!
//! `render` / `post` / `review` 由控制面执行，Agent 只用 `studio.status` 观察。
//! 这就是工具面上没有 `advance` 的原因——多一个工具就多一种被误用的方式。
//!
//! 具体实现（ComfyUI、ffmpeg）在更上层的 crate 里，这里只定义契约：
//! 引擎负责什么时候跑、跑完怎么落状态、失败了怎么让 Agent 看见。

use crate::config::Settings;
use crate::Bundle;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use studio_core::{Outputs, Result, StageId, StudioError};

/// 执行面的一步留痕。
///
/// MCP 那一侧的留痕记的是「Agent 调了什么」；这一份记的是「控制面做了什么」——
/// 哪个镜头提交到了哪个节点、排队渲染等了多久、下载多大、后期哪一步慢。
/// 两者分开是因为读者不同：前者看协作，后者看吞吐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRecord {
    pub at: String,
    pub stage: String,
    /// 这一步干了什么，例如 `pick_node` / `submit` / `render` / `download` / `concat`。
    pub step: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    pub duration_ms: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 这一步特有的信息，例如下载字节数、是否直接复制流。
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

pub const EXEC_TRACE_FILE: &str = ".studio/exec.jsonl";

/// 往 `.studio/exec.jsonl` 追加执行留痕。写失败不影响主流程。
#[derive(Debug)]
pub struct ExecRecorder {
    path: std::path::PathBuf,
    stage: Mutex<String>,
    /// 并发渲染时多个 worker 线程会同时 `append`——串行化整个「开文件+写一行」
    /// 过程，避免两行 JSON 交错拼在一起，写坏 `exec.jsonl`。
    write_lock: Mutex<()>,
}

impl ExecRecorder {
    pub fn at(bundle_root: &std::path::Path) -> ExecRecorder {
        ExecRecorder {
            path: bundle_root.join(EXEC_TRACE_FILE),
            stage: Mutex::new(String::new()),
            write_lock: Mutex::new(()),
        }
    }

    pub fn set_stage(&self, stage: StageId) {
        if let Ok(mut g) = self.stage.lock() {
            *g = stage.as_str().to_string();
        }
    }

    pub fn append(&self, rec: &ExecRecord) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(line) = serde_json::to_string(rec) else {
            return;
        };
        let bytes = format!("{line}\n");
        let _guard = self.write_lock.lock();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(bytes.as_bytes());
        }
    }

    pub fn read(bundle_root: &std::path::Path) -> Vec<ExecRecord> {
        let Ok(text) = std::fs::read_to_string(bundle_root.join(EXEC_TRACE_FILE)) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn stage_name(&self) -> String {
        self.stage.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// 一步的计时器。`done` / `ok` / `fail` 落一条留痕；忘了调就在 drop 时记成中断。
pub struct Step<'a> {
    rec: &'a ExecRecorder,
    step: String,
    shot_id: Option<String>,
    node: Option<String>,
    prompt_id: Option<String>,
    extra: Map<String, Value>,
    started: Instant,
    finished: bool,
}

impl Step<'_> {
    pub fn shot(mut self, id: impl Into<String>) -> Self {
        self.shot_id = Some(id.into());
        self
    }
    pub fn node(mut self, url: impl Into<String>) -> Self {
        self.node = Some(url.into());
        self
    }
    pub fn prompt(mut self, id: impl Into<String>) -> Self {
        self.prompt_id = Some(id.into());
        self
    }
    pub fn with(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.to_string(), value);
        self
    }

    /// 记一条成功，并把值原样传回去。
    pub fn ok<T>(mut self, value: T) -> T {
        self.write(true, None);
        value
    }

    /// 记一条失败，并把错误原样传回去。
    pub fn fail(mut self, e: StudioError) -> StudioError {
        self.write(false, Some(e.code().to_string()));
        e
    }

    /// 按 `Result` 自动记，最常用。
    pub fn done<T>(self, r: Result<T>) -> Result<T> {
        match r {
            Ok(v) => Ok(self.ok(v)),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn write(&mut self, ok: bool, error_code: Option<String>) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.rec.append(&ExecRecord {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            stage: self.rec.stage_name(),
            step: self.step.clone(),
            shot_id: self.shot_id.clone(),
            node: self.node.clone(),
            prompt_id: self.prompt_id.clone(),
            duration_ms: self.started.elapsed().as_millis() as u64,
            ok,
            error_code,
            extra: std::mem::take(&mut self.extra),
        });
    }
}

impl Drop for Step<'_> {
    fn drop(&mut self) {
        // 没显式结束就当中断——比悄悄丢掉一步好。
        self.write(false, Some("interrupted".into()));
    }
}

/// 执行一个确定性阶段所需的一切。
pub struct ExecContext<'a> {
    pub bundle: &'a Bundle,
    pub settings: &'a Settings,
    /// 上游已通过阶段的产物，键是各阶段的 output_key。
    pub inputs: serde_json::Value,
    /// 进度回报。写进去的字符串会出现在 `studio.status` 的信封里。
    pub progress: &'a ProgressNote,
    /// 执行留痕。逐步计时落进 `.studio/exec.jsonl`。
    pub recorder: &'a ExecRecorder,
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

    /// 开始计时一步。用 `.done(结果)` 或 `.ok(值)` / `.fail(错误)` 结束。
    pub fn step(&self, name: &str) -> Step<'_> {
        Step {
            rec: self.recorder,
            step: name.to_string(),
            shot_id: None,
            node: None,
            prompt_id: None,
            extra: Map::new(),
            started: Instant::now(),
            finished: false,
        }
    }

    /// 说一句进度，同时它也会成为下一步留痕的上下文。
    pub fn progress_and_step(&self, msg: impl Into<String>, step: &str) -> Step<'_> {
        let msg = msg.into();
        self.say(msg);
        self.step(step)
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

    /// 确定性阶段执行成功、且该阶段声明了确认门时，用这份文案把它挂起等待
    /// 确认，而不是直接判过。多数确定性阶段没有门（`stage.gate()` 是
    /// `None`），这个方法根本不会被调用。返回 `None` 时引擎会退回一份
    /// 通用文案——覆盖它只是为了给出更贴合场景的措辞（比如 preview）。
    fn gate_confirmation(&self, _stage: StageId) -> Option<studio_core::Confirmation> {
        None
    }

    /// 这个 Hybrid 阶段的产物还需不需要控制面执行。
    ///
    /// Hybrid 的意思是「Agent 定内容，控制面执行生成」——Agent 提交的是一份
    /// **计划**（`visual_assets` 那份里每个视图 `status: planned`、没有 `path`），
    /// 控制面照着计划真的把东西生成出来，再回填。
    ///
    /// **执行完才上确认门**，不是先上门再执行：门要人确认的是「卡片长得对不对」，
    /// 按后一种顺序人是在批准一份自己没见过的 JSON。旁边就有先例——`preview`
    /// 也是先执行、再在门上让人看 480p。
    ///
    /// 判据交给执行器而不是引擎，是因为「执行过没有」只有产物自己说得清
    /// （`visual_assets` 看的是还有没有 `planned` 的视图），引擎不该知道
    /// `asset_plan` 内部长什么样。返回 `false` 时 Hybrid 退化成 Creative：
    /// 提交完直接上门，跟以前一样。
    fn needs_execution(&self, _stage: StageId, _outputs: &Outputs) -> bool {
        false
    }

    /// 这台机器上可用基线的能力面。
    ///
    /// 引擎拿它在**提交** `prompt_pack` 时做双向对账：写了基线不吃的参数
    /// 会被静默丢弃，少写的参数会让基线用自己的默认值——两种都要在花 GPU
    /// 时间之前挡下来。返回 `None` 表示这个构建没有基线可查（例如测试里的
    /// [`NotWired`]），此时不做这层校验。
    fn capabilities(&self) -> Option<studio_core::CapabilitySet> {
        None
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
    fn steps_are_timed_and_appended() {
        let d = tempfile::tempdir().unwrap();
        let rec = ExecRecorder::at(d.path());
        rec.set_stage(StageId::Render);
        let bundle = Bundle::scaffold(d.path()).unwrap();
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &Settings::load(None, None),
            inputs: serde_json::Value::Null,
            progress: &ProgressNote::default(),
            recorder: &rec,
            cancelled: &AtomicBool::new(false),
        };

        ctx.step("submit")
            .shot("sh01")
            .node("http://127.0.0.1:9001")
            .prompt("abc-123")
            .ok(());
        let e = ctx
            .step("download")
            .shot("sh02")
            .fail(StudioError::ComfyUnavailable { tried: vec![] });
        assert_eq!(e.code(), "comfy_unavailable");

        let records = ExecRecorder::read(d.path());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].stage, "render");
        assert_eq!(records[0].step, "submit");
        assert_eq!(records[0].prompt_id.as_deref(), Some("abc-123"));
        assert!(records[0].ok);
        assert!(!records[1].ok);
        assert_eq!(records[1].error_code.as_deref(), Some("comfy_unavailable"));
    }

    /// 忘了结束的一步不该悄悄消失。
    #[test]
    fn an_abandoned_step_is_recorded_as_interrupted() {
        let d = tempfile::tempdir().unwrap();
        let rec = ExecRecorder::at(d.path());
        rec.set_stage(StageId::Post);
        {
            let bundle = Bundle::scaffold(d.path()).unwrap();
            let ctx = ExecContext {
                bundle: &bundle,
                settings: &Settings::load(None, None),
                inputs: serde_json::Value::Null,
                progress: &ProgressNote::default(),
                recorder: &rec,
                cancelled: &AtomicBool::new(false),
            };
            let _ = ctx.step("concat");
        }
        let records = ExecRecorder::read(d.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].error_code.as_deref(), Some("interrupted"));
    }

    #[test]
    fn extra_fields_ride_along() {
        let d = tempfile::tempdir().unwrap();
        let rec = ExecRecorder::at(d.path());
        let bundle = Bundle::scaffold(d.path()).unwrap();
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &Settings::load(None, None),
            inputs: serde_json::Value::Null,
            progress: &ProgressNote::default(),
            recorder: &rec,
            cancelled: &AtomicBool::new(false),
        };
        ctx.step("concat")
            .with("stream_copied", serde_json::json!(true))
            .with("parts", serde_json::json!(5))
            .ok(());
        let r = &ExecRecorder::read(d.path())[0];
        assert_eq!(r.extra["stream_copied"], serde_json::json!(true));
        assert_eq!(r.extra["parts"], serde_json::json!(5));
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
                    recorder: &ExecRecorder::at(std::path::Path::new("/tmp/studio-unwired")),
                    cancelled: &AtomicBool::new(false),
                },
            )
            .unwrap_err();
        assert_eq!(e.code(), "internal");
        assert!(e.message().contains("render"));
    }
}
