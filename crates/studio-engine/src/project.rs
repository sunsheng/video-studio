//! 一部作品的生命周期。
//!
//! 没有 `run_id`：`Project` 打开的那个目录**就是**当前项目，
//! 因此所有对外方法都不带项目维度的参数。

use crate::bundle::{Bundle, LockGuard};
use crate::config::Settings;
use serde_json::{json, Value};
use studio_core::contract::{
    ActionKind, Blocked, Envelope, NextAction, Outcome, Progress, ProjectInfo, ProjectStatus, WaitingOn,
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
    // 持有期间独占本 bundle；drop 即释放。
    _lock: LockGuard,
}

/// `studio.export` 的结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
    pub note: String,
}

impl Project {
    /// 打开当前目录（或其祖先）里的作品。
    pub fn open(root: impl AsRef<std::path::Path>, program_dir: Option<&std::path::Path>) -> Result<Project> {
        let bundle = Bundle::discover(root)?;
        let lock = bundle.lock()?;
        let store = Store::open(&bundle.db_path())?;
        let settings = Settings::load(program_dir, Some(bundle.root()));
        Ok(Project { bundle, store, settings, _lock: lock })
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
        self.envelope(None)
    }

    fn envelope(&self, blocked: Option<&StudioError>) -> Result<Envelope> {
        let title = self.store.title()?;
        let pending = self.store.pending_question()?;
        let completed = self.completed_count()?;
        let total = STAGE_TOTAL;
        let current = self.current_stage()?;

        let (stage, status, waiting_on, next_action) = match (&pending, current) {
            (Some(q), _) => (q.stage, ProjectStatus::Active, WaitingOn::User, None),
            (None, None) => (StageId::Review, ProjectStatus::Completed, WaitingOn::System, None),
            (None, Some(s)) => match s.kind() {
                StageKind::Deterministic => (s, ProjectStatus::Active, WaitingOn::System, Some(self.await_action(s)?)),
                _ => (s, ProjectStatus::Active, WaitingOn::Agent, Some(self.submit_action(s)?)),
            },
        };

        let status = if blocked.is_some() { ProjectStatus::Blocked } else { status };

        Ok(Envelope {
            project: ProjectInfo { title, stage, status },
            waiting_on,
            blocked_by: blocked.map(Blocked::from),
            pending_question: pending,
            next_action,
            progress: Progress { completed, total },
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
        let submitted = draft.submit(outputs, confirmation)?;

        let (state, question) = match &submitted {
            Submitted::AwaitingConfirmation(s) => (StageState::AwaitingConfirmation, Some(s.question().clone())),
            Submitted::Approved(_) => (StageState::Approved, None),
        };
        let attempt = match &submitted {
            Submitted::AwaitingConfirmation(s) => s.attempt(),
            Submitted::Approved(s) => s.attempt(),
        };
        let outputs_ref = submitted.outputs();

        self.store.save_stage(stage, state, attempt, outputs_ref, summary, question.as_ref())?;
        self.mirror_stage_file(stage, outputs_ref)?;

        let desc = summary.unwrap_or("已提交阶段产物");
        self.store.append_event(stage, "submitted", desc, None)?;
        if let Some(q) = &question {
            self.store.append_event(stage, "gate_opened", &q.prompt, None)?;
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
            return Err(StudioError::GatePending { stage: q.stage, question_id: q.question_id.clone() });
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
                self.revise(q.stage, &format!("用户在确认门选择了「{label}」"))
            }
            Some(Outcome::Approve) => {
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
                self.store.append_event(q.stage, "approved", &format!("已确认 {}", q.question_id), None)?;
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
        self.store.take_snapshot(&format!("修订 {stage} 之前：{message}"))?;
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

        self.store.save_stage(stage, StageState::Draft, attempt, outputs.as_ref(), None, None)?;
        self.store.append_event(stage, "revised", message, None)?;
        self.rewind_after(stage)?;
        self.status()
    }

    /// 撤销上一次修订，把作品整个恢复到那次 `revise` 之前。
    ///
    /// 场景：改完剧本走到分镜，又觉得还不如原来那版——`undo` 之后旧剧本回来，
    /// 分镜恢复已通过，下一步接着是视觉资产。
    ///
    /// 只有一层。恢复之后快照即被消耗，不能连着撤销两次。
    pub fn undo(&self) -> Result<Envelope> {
        let label = self.store.restore_snapshot()?;
        let stage = self.current_stage()?.unwrap_or(StageId::Review);
        self.store.append_event(stage, "undone", &format!("已撤销：{label}"), None)?;
        self.status()
    }

    /// 当前是否有可撤销的修订。
    pub fn undoable(&self) -> Result<Option<String>> {
        self.store.snapshot_label()
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
                self.store.save_stage(stage, StageState::Draft, loaded.attempt(), keep.as_ref(), None, None)?;
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
            return Err(StudioError::StageNotReady { stage: StageId::Post, blocked_on: StageId::Post });
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
                std::fs::copy(&src, &dst).map_err(|e| StudioError::internal(format!("投递 {rel} 失败：{e}")))?;
            }
            files.push(dst_rel);
        }
        self.store.append_event(StageId::Post, "exported", &format!("投递 {} 个交付物", files.len()), None)?;
        Ok(ExportResult {
            files,
            note: "交付物已放进 output/。这是唯一对用户可见的产物目录，中间媒体留在 media/。".into(),
        })
    }

    /// 把阶段产物同步成人可读的 `stages/<stage>.json`，方便直接打开看和进 Git。
    fn mirror_stage_file(&self, stage: StageId, outputs: Option<&Outputs>) -> Result<()> {
        let Some(o) = outputs else { return Ok(()) };
        let text = serde_json::to_string_pretty(o).map_err(|e| StudioError::internal(e.to_string()))?;
        self.bundle.write(&format!("stages/{stage}.json"), &format!("{text}\n"))
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
            progress: Progress { completed: 0, total: STAGE_TOTAL },
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
