//! 一部作品的生命周期。
//!
//! 没有 `run_id`：`Project` 打开的那个目录**就是**当前项目，
//! 因此所有对外方法都不带项目维度的参数。

use crate::bundle::{Bundle, LockGuard};
use crate::config::Settings;
use crate::executor::{ExecContext, NotWired, ProgressNote, SharedExecutor};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use studio_core::contract::{
    ActionKind, Blocked, Envelope, NextAction, Outcome, Progress, ProjectInfo, ProjectStatus,
    WaitingOn,
};
use studio_core::state::{LoadedStage, Stage, StageState, Submitted};
use studio_core::{schema, Confirmation, Event, Outputs, Result, StageId, StageKind, StudioError};
use studio_store::Store;

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Project({})", self.bundle.root().display())
    }
}

pub struct Project {
    bundle: Bundle,
    store: Store,
    settings: Settings,
    executor: SharedExecutor,
    worker: Mutex<Option<Worker>>,
    // 持有期间独占本 bundle；drop 即释放。
    _lock: LockGuard,
}

/// 后台跑确定性阶段的线程。
struct Worker {
    handle: Option<std::thread::JoinHandle<()>>,
    progress: Arc<ProgressNote>,
    cancelled: Arc<AtomicBool>,
}

impl Worker {
    fn finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(true)
    }

    fn stop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        // 会话结束时不要留下还在跑的线程。
        if let Ok(mut g) = self.worker.lock() {
            if let Some(w) = g.as_mut() {
                w.stop();
            }
        }
    }
}

/// 控制面执行阶段时留下的失败记录，供 `status` 变成 `blocked_by`。
const STAGE_ERROR_KEY: &str = "stage_error";

/// `studio.export` 的结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
    pub note: String,
}

impl Project {
    /// 打开当前目录（或其祖先）里的作品。确定性阶段不接线。
    pub fn open(
        root: impl AsRef<std::path::Path>,
        program_dir: Option<&std::path::Path>,
    ) -> Result<Project> {
        Project::open_with(root, program_dir, Arc::new(NotWired))
    }

    /// 打开并接上确定性阶段的执行器。
    pub fn open_with(
        root: impl AsRef<std::path::Path>,
        program_dir: Option<&std::path::Path>,
        executor: SharedExecutor,
    ) -> Result<Project> {
        let bundle = Bundle::discover(root)?;
        let lock = bundle.lock()?;
        let store = Store::open(&bundle.db_path())?;
        let settings = Settings::load(program_dir, Some(bundle.root()));
        Ok(Project {
            bundle,
            store,
            settings,
            executor,
            worker: Mutex::new(None),
            _lock: lock,
        })
    }

    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn store(&self) -> &Store {
        &self.store
    }

    // ---------- 只读 ----------

    /// 第一个尚未通过的阶段。全部通过时返回 None。
    pub fn current_stage(&self) -> Result<Option<StageId>> {
        for stage in StageId::all() {
            if self.store.load_stage(stage)?.state() != StageState::Approved {
                return Ok(Some(stage));
            }
        }
        Ok(None)
    }

    fn completed_count(&self) -> Result<usize> {
        let mut n = 0;
        for stage in StageId::all() {
            if self.store.load_stage(stage)?.state() == StageState::Approved {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 已通过阶段的产物，作为下一阶段的输入。
    fn inputs_for(&self, stage: StageId) -> Result<Value> {
        let mut map = serde_json::Map::new();
        for prev in StageId::all() {
            if prev.index() >= stage.index() {
                break;
            }
            let loaded = self.store.load_stage(prev)?;
            if loaded.state() == StageState::Approved {
                if let Some(o) = loaded_outputs(&loaded) {
                    if let Some(v) = o.get(prev.output_key()) {
                        map.insert(prev.output_key().to_string(), v.clone());
                    }
                }
            }
        }
        Ok(Value::Object(map))
    }

    /// 决策信封。Agent 只看这个就知道该干什么。
    pub fn status(&self) -> Result<Envelope> {
        self.ensure_worker();
        let mut env = self.envelope(None)?;
        if env.blocked_by.is_none() {
            if let Some(b) = self.recorded_error() {
                env.blocked_by = Some(b);
                env.project.status = ProjectStatus::Blocked;
            }
        }
        Ok(env)
    }

    fn envelope(&self, blocked: Option<&StudioError>) -> Result<Envelope> {
        let title = self.store.title()?;
        let pending = self.store.pending_question()?;
        let completed = self.completed_count()?;
        let total = STAGE_TOTAL;
        let current = self.current_stage()?;

        let (stage, status, waiting_on, next_action) = match (&pending, current) {
            (Some(q), _) => (q.stage, ProjectStatus::Active, WaitingOn::User, None),
            (None, None) => (
                StageId::Review,
                ProjectStatus::Completed,
                WaitingOn::System,
                None,
            ),
            (None, Some(s)) => match s.kind() {
                StageKind::Deterministic => (
                    s,
                    ProjectStatus::Active,
                    WaitingOn::System,
                    Some(self.await_action(s)?),
                ),
                _ => (
                    s,
                    ProjectStatus::Active,
                    WaitingOn::Agent,
                    Some(self.submit_action(s)?),
                ),
            },
        };

        let status = if blocked.is_some() {
            ProjectStatus::Blocked
        } else {
            status
        };

        Ok(Envelope {
            project: ProjectInfo {
                title,
                stage,
                status,
            },
            waiting_on,
            blocked_by: blocked.map(Blocked::from),
            pending_question: pending,
            next_action,
            progress: Progress { completed, total },
            note: self.worker_note(),
        })
    }

    fn submit_action(&self, stage: StageId) -> Result<NextAction> {
        Ok(NextAction {
            kind: ActionKind::SubmitStage,
            stage,
            capability: stage.capability(),
            gate: stage.gate().map(|g| g.to_string()),
            inputs: self.inputs_for(stage)?,
            required_outputs: vec![stage.output_key().to_string()],
            schema_ref: stage.to_string(),
        })
    }

    fn await_action(&self, stage: StageId) -> Result<NextAction> {
        Ok(NextAction {
            kind: ActionKind::Await,
            stage,
            capability: stage.capability(),
            gate: None,
            inputs: self.inputs_for(stage)?,
            required_outputs: vec![stage.output_key().to_string()],
            schema_ref: stage.to_string(),
        })
    }

    pub fn schema_of(&self, stage: StageId) -> Value {
        schema::stage_schema_document(stage)
    }

    pub fn stage_output(&self, stage: StageId) -> Result<Value> {
        let loaded = self.store.load_stage(stage)?;
        match loaded_outputs(&loaded) {
            Some(o) => Ok(Value::Object(o.clone())),
            None => Ok(json!({ "stage": stage.as_str(), "outputs": Value::Null,
                               "note": "该阶段还没有产物。调 studio.status() 看当前该做什么。" })),
        }
    }

    pub fn timeline(&self, limit: usize) -> Result<Vec<Event>> {
        self.store.timeline(limit.clamp(1, 500))
    }

    // ---------- 变更 ----------

    /// 提交当前阶段的产物。
    pub fn submit_stage(
        &self,
        outputs: Outputs,
        summary: Option<&str>,
        confirmation: Option<Confirmation>,
    ) -> Result<Envelope> {
        let Some(stage) = self.current_stage()? else {
            return Err(StudioError::InvalidTransition {
                stage: StageId::Review,
                current: "completed",
                attempted: "submit_stage",
                allowed: vec!["studio.export"],
            });
        };

        let loaded = self.store.load_stage(stage)?;
        let draft = match loaded {
            LoadedStage::Draft(d) => d,
            LoadedStage::Awaiting(a) => {
                return Err(StudioError::GatePending {
                    stage,
                    question_id: a.question().question_id.clone(),
                })
            }
            LoadedStage::Approved(_) => {
                return Err(StudioError::InvalidTransition {
                    stage,
                    current: "approved",
                    attempted: "submit_stage",
                    allowed: vec!["studio.undo", "studio.revise"],
                })
            }
        };

        schema::validate(stage, &outputs)?;
        // 先校验再压栈：没通过校验的调用不该占掉一层撤销。
        self.store.take_snapshot(&format!("提交 {stage} 之前"))?;
        let submitted = draft.submit(outputs, confirmation)?;

        let (state, question) = match &submitted {
            Submitted::AwaitingConfirmation(s) => {
                (StageState::AwaitingConfirmation, Some(s.question().clone()))
            }
            Submitted::Approved(_) => (StageState::Approved, None),
        };
        let attempt = match &submitted {
            Submitted::AwaitingConfirmation(s) => s.attempt(),
            Submitted::Approved(s) => s.attempt(),
        };
        let outputs_ref = submitted.outputs();

        self.store.save_stage(
            stage,
            state,
            attempt,
            outputs_ref,
            summary,
            question.as_ref(),
        )?;
        self.mirror_stage_file(stage, outputs_ref)?;

        let desc = summary.unwrap_or("已提交阶段产物");
        self.store.append_event(stage, "submitted", desc, None)?;
        if let Some(q) = &question {
            self.store
                .append_event(stage, "gate_opened", &q.prompt, None)?;
        }
        self.status()
    }

    /// 应答确认门。
    ///
    /// 选项自己声明 outcome：`approve` 通过，`revise` 直接把阶段打回草稿。
    /// Agent 不需要靠 id 的字面意思去猜。
    pub fn answer(&self, question_id: &str, answer: &str) -> Result<Envelope> {
        let Some(q) = self.store.pending_question()? else {
            let stage = self.current_stage()?.unwrap_or(StageId::Review);
            return Err(StudioError::InvalidTransition {
                stage,
                current: "no_pending_gate",
                attempted: "answer",
                allowed: vec!["studio.submit_stage", "studio.status"],
            });
        };
        if q.question_id != question_id {
            return Err(StudioError::GatePending {
                stage: q.stage,
                question_id: q.question_id.clone(),
            });
        }

        match q.outcome_of(answer) {
            None => Err(StudioError::UnknownAnswer {
                question_id: q.question_id.clone(),
                given: answer.to_string(),
                options: q.option_ids(),
            }),
            Some(Outcome::Revise) => {
                let label = q
                    .options
                    .iter()
                    .find(|o| o.id == answer)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| answer.to_string());
                self.store
                    .take_snapshot(&format!("在 {} 的确认门上选择「{label}」之前", q.stage))?;
                self.revise_inner(q.stage, &format!("用户在确认门选择了「{label}」"))
            }
            Some(Outcome::Approve) => {
                self.store
                    .take_snapshot(&format!("确认 {} 之前", q.stage))?;
                let LoadedStage::Awaiting(awaiting) = self.store.load_stage(q.stage)? else {
                    return Err(StudioError::StateDrift {
                        detail: format!("门挂在 {} 上，但该阶段并非等待确认", q.stage),
                    });
                };
                let approved = awaiting.approve(answer)?;
                self.store.save_stage(
                    q.stage,
                    StageState::Approved,
                    approved.attempt(),
                    approved.outputs(),
                    self.store.stage_summary(q.stage)?.as_deref(),
                    None,
                )?;
                self.store.append_event(
                    q.stage,
                    "approved",
                    &format!("已确认 {}", q.question_id),
                    None,
                )?;
                self.status()
            }
        }
    }

    /// 修订某个阶段：回到草稿，等待重新提交。
    ///
    /// **不会失败**是这个设计的要点。前身项目的 revise 只把阶段标成
    /// `ready_for_redo` 却不释放占用，紧接着的 submit 必然撞上
    /// `task already claimed`——用户要求改稿这条路径必然死锁。
    ///
    /// 作品的进度整体退回到这个阶段：它之后的所有阶段一律变回未执行。
    /// 分镜是照旧剧本做的，剧本改了它就不再成立，不该还算通过。
    ///
    /// 修订前会存一份**单槽快照**，[`Project::undo`] 可以整个恢复回来——
    /// 「改完发现还不如原来那版」时用得上。只保留最近一次，再修订一次就覆盖。
    /// 这不是版本管理，是编辑器的 Ctrl+Z：一层，不留历史列表。
    pub fn revise(&self, stage: StageId, message: &str) -> Result<Envelope> {
        self.store.take_snapshot(&format!("修订 {stage} 之前"))?;
        self.clear_recorded_error()?;
        self.revise_inner(stage, message)
    }

    fn revise_inner(&self, stage: StageId, message: &str) -> Result<Envelope> {
        let loaded = self.store.load_stage(stage)?;
        let (attempt, outputs) = match loaded {
            LoadedStage::Awaiting(a) => {
                let d = a.revise(message);
                (d.attempt(), d.outputs().cloned())
            }
            LoadedStage::Approved(a) => {
                let d = a.undo();
                (d.attempt(), d.outputs().cloned())
            }
            LoadedStage::Draft(d) => {
                // 已经是草稿：只记录反馈，状态不变。重复调用是安全的。
                (d.attempt(), d.outputs().cloned())
            }
        };

        self.store.save_stage(
            stage,
            StageState::Draft,
            attempt,
            outputs.as_ref(),
            None,
            None,
        )?;
        self.store.append_event(stage, "revised", message, None)?;
        self.rewind_after(stage)?;
        self.status()
    }

    /// 撤销上一步，就是编辑器的 Ctrl+Z。
    ///
    /// 每个改变状态的操作（提交、确认、修订）在动手前都压了一份快照，
    /// `undo` 弹出最上面那份整个恢复回来。连着调就一步步往回走：
    /// 走到分镜之后连按两次，就退回到剧本确认之前。
    ///
    /// 恢复的是整部作品的状态，不只是某个阶段——所以「改完剧本发现不如原来
    /// 那版」时，旧剧本回来的同时，被退回的下游阶段也恢复已通过。
    pub fn undo(&self) -> Result<Envelope> {
        let label = self.store.restore_snapshot()?;
        self.clear_recorded_error()?;
        let stage = self.current_stage()?.unwrap_or(StageId::Review);
        self.store
            .append_event(stage, "undone", &format!("已撤销：{label}"), None)?;
        self.status()
    }

    /// 栈顶那一步的说明，栈空则为 None。
    pub fn undoable(&self) -> Result<Option<String>> {
        self.store.snapshot_label()
    }

    /// 还能往回走几步。
    pub fn undo_depth(&self) -> Result<usize> {
        self.store.undo_depth()
    }

    // ---------- 确定性阶段的后台执行 ----------

    /// 控制面此刻在做什么。
    fn worker_note(&self) -> Option<String> {
        self.worker.lock().ok()?.as_ref()?.progress.get()
    }

    /// 当前阶段是确定性的、没挂门、也没记着失败时，把执行器跑起来。
    ///
    /// 这就是工具面上没有 `advance` 的原因：门一通过控制面自己往下走，
    /// Agent 只需要用 `studio.status` 观察。
    fn ensure_worker(&self) {
        if !self.executor.is_wired() {
            return;
        }
        let Ok(Some(stage)) = self.current_stage() else {
            return;
        };
        if stage.kind() != StageKind::Deterministic {
            return;
        }
        if matches!(self.store.pending_question(), Ok(Some(_))) {
            return;
        }
        if self.recorded_error().is_some() {
            // 上一次失败还挂着，等 Agent 修订之后再重试，不要闷头重来。
            return;
        }

        let Ok(mut slot) = self.worker.lock() else {
            return;
        };
        if let Some(w) = slot.as_ref() {
            if !w.finished() {
                return;
            }
        }

        let progress = Arc::new(ProgressNote::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let root = self.bundle.root().to_path_buf();
        let db = self.bundle.db_path();
        let settings = self.settings.clone();
        let executor = Arc::clone(&self.executor);
        let p2 = Arc::clone(&progress);
        let c2 = Arc::clone(&cancelled);

        let handle = std::thread::Builder::new()
            .name("studio-deterministic".into())
            .spawn(move || run_deterministic(root, db, settings, executor, p2, c2))
            .ok();

        *slot = Some(Worker {
            handle,
            progress,
            cancelled,
        });
    }

    /// 控制面执行失败时留下的记录。
    fn recorded_error(&self) -> Option<Blocked> {
        let raw = self.store.meta(STAGE_ERROR_KEY).ok().flatten()?;
        serde_json::from_str(&raw).ok()
    }

    fn clear_recorded_error(&self) -> Result<()> {
        self.store.set_meta(STAGE_ERROR_KEY, "")
    }

    /// 把 `from` 之后的所有阶段退回未执行。
    ///
    /// 只改状态，不删产物：旧的 `stages/*.json` 原地留着，Agent 可以
    /// `studio.stage_output` 读到上一版参考着改，重新提交时覆盖。
    fn rewind_after(&self, from: StageId) -> Result<()> {
        let mut rewound = Vec::new();
        for stage in StageId::all() {
            if stage.index() <= from.index() {
                continue;
            }
            let loaded = self.store.load_stage(stage)?;
            if loaded.state() != StageState::Draft {
                let keep = loaded_outputs(&loaded).cloned();
                self.store.save_stage(
                    stage,
                    StageState::Draft,
                    loaded.attempt(),
                    keep.as_ref(),
                    None,
                    None,
                )?;
                rewound.push(stage.as_str());
            }
        }
        if !rewound.is_empty() {
            self.store.append_event(
                from,
                "rewound",
                &format!("后续阶段已退回未执行：{}", rewound.join("、")),
                None,
            )?;
        }
        Ok(())
    }

    /// 把交付物投递到 `output/`。
    pub fn export(&self) -> Result<ExportResult> {
        let post = self.store.load_stage(StageId::Post)?;
        if post.state() != StageState::Approved {
            return Err(StudioError::StageNotReady {
                stage: StageId::Post,
                blocked_on: StageId::Post,
            });
        }
        let mut files = Vec::new();
        for (_, _, rel) in self.store.artifacts(Some(StageId::Post))? {
            let src = self.bundle.resolve(&rel)?;
            if !src.is_file() {
                return Err(StudioError::ArtifactMissing { path: rel.clone() });
            }
            let name = std::path::Path::new(&rel)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "deliverable".to_string());
            let dst_rel = format!("output/{name}");
            let dst = self.bundle.resolve(&dst_rel)?;
            if src != dst {
                std::fs::copy(&src, &dst)
                    .map_err(|e| StudioError::internal(format!("投递 {rel} 失败：{e}")))?;
            }
            files.push(dst_rel);
        }
        self.store.append_event(
            StageId::Post,
            "exported",
            &format!("投递 {} 个交付物", files.len()),
            None,
        )?;
        Ok(ExportResult {
            files,
            note: "交付物已放进 output/。这是唯一对用户可见的产物目录，中间媒体留在 media/。"
                .into(),
        })
    }

    /// 把阶段产物同步成人可读的 `stages/<stage>.json`，方便直接打开看和进 Git。
    fn mirror_stage_file(&self, stage: StageId, outputs: Option<&Outputs>) -> Result<()> {
        let Some(o) = outputs else { return Ok(()) };
        let text =
            serde_json::to_string_pretty(o).map_err(|e| StudioError::internal(e.to_string()))?;
        self.bundle
            .write(&format!("stages/{stage}.json"), &format!("{text}\n"))
    }

    /// 把错误包成信封返回，保证 `blocked_by.remedy` 一定在。
    pub fn envelope_for_error(&self, e: &StudioError) -> Envelope {
        self.envelope(Some(e)).unwrap_or_else(|_| Envelope {
            project: ProjectInfo {
                title: "未知作品".into(),
                stage: StageId::Idea,
                status: ProjectStatus::Blocked,
            },
            waiting_on: WaitingOn::Agent,
            blocked_by: Some(Blocked::from(e)),
            pending_question: None,
            next_action: None,
            progress: Progress {
                completed: 0,
                total: STAGE_TOTAL,
            },
            note: None,
        })
    }
}

const STAGE_TOTAL: usize = 9;

fn loaded_outputs(l: &LoadedStage) -> Option<&Outputs> {
    match l {
        LoadedStage::Draft(s) => s.outputs(),
        LoadedStage::Awaiting(s) => s.outputs(),
        LoadedStage::Approved(s) => s.outputs(),
    }
}

/// 新建一部作品。`extra_files` 是 `(bundle 内相对路径, 内容)`，
/// 由上层把生成好的 AGENTS.md / SKILL.md / .codex/config.toml 传进来。
pub fn init_project(
    root: impl AsRef<std::path::Path>,
    title: &str,
    program_version: &str,
    extra_files: &[(String, String)],
) -> Result<Bundle> {
    let root = root.as_ref();
    if root.join(crate::bundle::DB_FILE).is_file() {
        return Err(StudioError::internal(format!(
            "{} 已经是一部作品了。换个路径，或直接 cd 进去打开 Codex。",
            root.display()
        )));
    }
    let bundle = Bundle::scaffold(root)?;
    Store::create(&bundle.db_path(), title, program_version)?;
    for (rel, content) in extra_files {
        bundle.write(rel, content)?;
    }
    Ok(bundle)
}

/// 直接把一个 `Stage<Draft>` 交给存储层——仅供测试与迁移工具使用。
#[doc(hidden)]
pub fn __draft(stage: StageId) -> Stage<studio_core::state::Draft> {
    Stage::new(stage)
}

/// 后台线程：把当前及其后的确定性阶段一路跑完。
///
/// 用自己的 SQLite 连接写状态——主连接同时在服务 MCP 调用，
/// WAL 加 busy_timeout 足够应付这点并发。
fn run_deterministic(
    root: std::path::PathBuf,
    db: std::path::PathBuf,
    settings: Settings,
    executor: SharedExecutor,
    progress: Arc<ProgressNote>,
    cancelled: Arc<AtomicBool>,
) {
    let Ok(bundle) = Bundle::open(&root) else {
        return;
    };
    let Ok(store) = Store::open(&db) else { return };

    for stage in StageId::all() {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        let Ok(loaded) = store.load_stage(stage) else {
            return;
        };
        if loaded.state() == StageState::Approved {
            continue;
        }
        if stage.kind() != StageKind::Deterministic {
            // 轮到需要 Agent 或用户的阶段了，交回去。
            return;
        }

        progress.set(format!("{stage} 开始"));
        let _ = store.append_event(stage, "started", &format!("控制面开始执行 {stage}"), None);

        let inputs = collect_inputs(&store, stage).unwrap_or(Value::Null);
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs,
            progress: &progress,
            cancelled: &cancelled,
        };

        match executor.execute(stage, &ctx) {
            Ok(outputs) => {
                if schema::validate(stage, &outputs).is_err() {
                    record_failure(
                        &store,
                        stage,
                        &StudioError::internal(format!(
                            "{stage} 的执行结果不符合自身契约，这是实现缺陷"
                        )),
                    );
                    return;
                }
                let _ = store.save_stage(
                    stage,
                    StageState::Approved,
                    loaded.attempt(),
                    Some(&outputs),
                    None,
                    None,
                );
                let text = serde_json::to_string_pretty(&outputs).unwrap_or_default();
                let _ = bundle.write(&format!("stages/{stage}.json"), &format!("{text}\n"));
                let _ = store.append_event(stage, "succeeded", &format!("{stage} 完成"), None);
                progress.clear();
            }
            Err(e) => {
                record_failure(&store, stage, &e);
                return;
            }
        }
    }
    progress.clear();
}

fn record_failure(store: &Store, stage: StageId, e: &StudioError) {
    let blocked = Blocked::from(e);
    let _ = store.append_event(stage, "failed", &e.message(), Some(e.code()));
    if let Ok(json) = serde_json::to_string(&blocked) {
        let _ = store.set_meta(STAGE_ERROR_KEY, &json);
    }
}

fn collect_inputs(store: &Store, stage: StageId) -> Result<Value> {
    let mut map = serde_json::Map::new();
    for prev in StageId::all() {
        if prev.index() >= stage.index() {
            break;
        }
        let loaded = store.load_stage(prev)?;
        if loaded.state() == StageState::Approved {
            if let Some(o) = loaded_outputs(&loaded) {
                if let Some(v) = o.get(prev.output_key()) {
                    map.insert(prev.output_key().to_string(), v.clone());
                }
            }
        }
    }
    Ok(Value::Object(map))
}
