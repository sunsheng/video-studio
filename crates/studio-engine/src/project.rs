//! 一部作品的生命周期。
//!
//! 没有 `run_id`：`Project` 打开的那个目录**就是**当前项目，
//! 因此所有对外方法都不带项目维度的参数。

use crate::bundle::{Bundle, LockGuard};
use crate::config::Settings;
use crate::executor::{ExecContext, ExecRecorder, NotWired, ProgressNote, SharedExecutor};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use studio_core::contract::{
    ActionKind, AnswerOption, Blocked, Envelope, NextAction, Outcome, Progress, ProjectInfo,
    ProjectStatus, Question, SelectionType, WaitingOn,
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
    /// 只用来在每次重试确定性阶段前重新读一遍 `.env`——不落盘、不进数据库。
    /// bundle 本身要能被随意 `mv` / `cp -r` 到别的机器，机器相关的配置
    /// （比如这台机器上的 ComfyUI 集群地址）永远只能来自当次进程能看到的
    /// 文件系统，不能变成作品状态的一部分。
    program_dir: Option<std::path::PathBuf>,
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
            program_dir: program_dir.map(|p| p.to_path_buf()),
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

    /// 已通过阶段数与第一个未通过阶段，**一次扫描**内一起算出。
    ///
    /// 确定性阶段跑在后台线程里，用自己的连接写状态；如果这里分两次扫描
    /// （先数完通过数，再单独找第一个未通过阶段），两次扫描之间足够让
    /// 后台线程把最后一个阶段提交为 Approved——这时 completed 还停在
    /// 扫描时的旧值，current 却已经是最新的 None（全部通过），信封里就会
    /// 出现「completed=9 但 status=Completed」这种自相矛盾的组合。
    /// 合并成一次扫描后两者必然来自同一批读取，不会再对不上。
    fn progress_and_current(&self) -> Result<(usize, Option<StageId>)> {
        let mut completed = 0;
        let mut current = None;
        for stage in StageId::all() {
            if self.store.load_stage(stage)?.state() == StageState::Approved {
                completed += 1;
            } else if current.is_none() {
                current = Some(stage);
            }
        }
        Ok((completed, current))
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
        let total = STAGE_TOTAL;
        let (completed, current) = self.progress_and_current()?;

        let (stage, status, waiting_on, next_action) = match (&pending, current) {
            (Some(q), _) => (q.stage, ProjectStatus::Active, WaitingOn::User, None),
            // 十个阶段都通过了，但验收只做了技术那一半：片子是完整的，
            // 没人说过它好不好。差这份记录就不算收尾。
            (None, None) if self.content_review_missing()? => (
                StageId::Review,
                ProjectStatus::Active,
                WaitingOn::Agent,
                Some(self.self_review_action()?),
            ),
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
            decisions: self.store.decisions(DECISION_LIMIT)?,
        })
    }

    /// 内容自评要照着技术验收的实测结果写，所以输入里得带上 review 自己
    /// 的产物——`inputs_for` 只给前置阶段，这里补一格。
    fn self_review_action(&self) -> Result<NextAction> {
        let mut inputs = self.inputs_for(StageId::Review)?;
        let loaded = self.store.load_stage(StageId::Review)?;
        if let (Some(map), Some(o)) = (inputs.as_object_mut(), loaded_outputs(&loaded)) {
            if let Some(v) = o.get("review") {
                map.insert("review".to_string(), v.clone());
            }
        }
        Ok(NextAction {
            kind: ActionKind::SelfReview,
            stage: StageId::Review,
            capability: StageId::Review.capability(),
            gate: None,
            inputs,
            required_outputs: vec!["content_review".to_string()],
            schema_ref: "review".to_string(),
            decisions: self.store.decisions(DECISION_LIMIT)?,
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
            // 等控制面跑的时候 Agent 不写东西，档案给了也没用。
            decisions: Vec::new(),
        })
    }

    pub fn schema_of(&self, stage: StageId) -> Value {
        let mut doc = schema::stage_schema_document(stage);
        // 提示词包的 workflow 取值随机器而变：只给出这台机器上真正能跑的
        // 那几条，而不是让 Agent 写完一整包才在提交时被告知基线没核验。
        if stage == StageId::PromptPack {
            if let Some(caps) = self.executor.capabilities() {
                caps.narrow_schema(&mut doc);
            }
        }
        doc
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
        // 形状对了还不够：还要对得上这台机器上基线的能力面。写了基线没有
        // 绑定的参数会被静默跳过——不报错、不留痕，只让画面莫名其妙地不对。
        // 这道关必须在 prompt_pack 那道门之前，因为门一过就开始烧 GPU。
        if stage == StageId::PromptPack {
            if let Some(caps) = self.executor.capabilities() {
                caps.check_prompt_pack(&outputs)?;
            }
        }
        // 形状对、参数对，内容仍然可以是空的：`three_facts: ["好看","很美","有感觉"]`
        // 完全合规。质量闸挡的是这一类——只挡机械可判的，人的判断留给确认门。
        studio_core::quality::gate(stage, &outputs)?;
        // 身份锁要跨阶段比对，得先把已通过的阶段捞出来。
        self.check_identity_lock_across_stages(stage, &outputs)?;
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

    /// 身份锁必须在分镜、视觉资产、提示词包三处逐字相同。
    ///
    /// 单看一个阶段是查不出漂移的——三处各写各的，每一处单独看都合规。
    /// 所以这一条要把已通过的阶段捞出来一起比。
    fn check_identity_lock_across_stages(&self, stage: StageId, outputs: &Outputs) -> Result<()> {
        const LOCKED: [StageId; 3] = [
            StageId::Storyboard,
            StageId::VisualAssets,
            StageId::PromptPack,
        ];
        if !LOCKED.contains(&stage) {
            return Ok(());
        }
        let mut all = Vec::new();
        for s in LOCKED {
            if s == stage {
                all.push((s, outputs.clone()));
                continue;
            }
            let loaded = self.store.load_stage(s)?;
            if loaded.state() != StageState::Approved {
                continue;
            }
            if let Some(o) = loaded_outputs(&loaded) {
                all.push((s, o.clone()));
            }
        }
        let findings = studio_core::quality::check_across_stages(&all);
        let blocking: Vec<studio_core::Violation> = findings
            .iter()
            .filter(|f| f.severity == studio_core::Severity::Blocking)
            .map(|f| {
                studio_core::Violation::new(f.path.clone(), format!("[{}] {}", f.rule, f.message))
            })
            .collect();
        if blocking.is_empty() {
            return Ok(());
        }
        Err(StudioError::QualityViolation {
            stage,
            findings: blocking,
        })
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
                // preview 没有独立内容，它的门选「有问题」时要退回 prompt_pack，
                // 不是退回 preview 自己——revise_target() 是这条重定向规则的唯一事实源。
                self.revise_inner(
                    q.stage.revise_target(),
                    &format!("用户在确认门选择了「{label}」"),
                )
            }
            Some(Outcome::Approve) => {
                self.store
                    .take_snapshot(&format!("确认 {} 之前", q.stage))?;
                let LoadedStage::Awaiting(awaiting) = self.store.load_stage(q.stage)? else {
                    return Err(StudioError::StateDrift {
                        detail: format!("门挂在 {} 上，但该阶段并非等待确认", q.stage),
                    });
                };
                let label = q
                    .options
                    .iter()
                    .find(|o| o.id == answer)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| answer.to_string());
                let approved = awaiting.approve(answer)?;
                // 门上选了哪一项**是作品状态的一部分**，不只是一条日志：
                // 一道门给出多个通过选项时（例如让用户从几个方案里挑一个），
                // 下游必须知道用户挑的是哪个。写进产物，`next_action.inputs`
                // 就会自动带给下游阶段，不必让 Agent 回头翻 timeline。
                let outputs = approved.outputs().map(|o| {
                    let mut o = o.clone();
                    if let Some(v) = o.get_mut(q.stage.output_key()) {
                        if let Some(obj) = v.as_object_mut() {
                            obj.insert(
                                "_gate_choice".to_string(),
                                serde_json::json!({
                                    "option_id": answer,
                                    "label": label,
                                    "question_id": q.question_id,
                                }),
                            );
                        }
                    }
                    o
                });
                self.store.save_stage(
                    q.stage,
                    StageState::Approved,
                    approved.attempt(),
                    outputs.as_ref(),
                    self.store.stage_summary(q.stage)?.as_deref(),
                    None,
                )?;
                // 产物刚被改过（多了 _gate_choice），人可读的那份也要跟上，
                // 否则 stages/<stage>.json 里看不到用户到底选了哪个方案——
                // bundle 是文档即事实，两份对不上就等于文档在撒谎。
                self.mirror_stage_file(q.stage, outputs.as_ref())?;
                self.store.append_event(
                    q.stage,
                    "approved",
                    &format!("已确认 {}：选择了「{label}」", q.question_id),
                    None,
                )?;
                // 门上选了哪一项也是「用户要什么」的一部分，进决定档案。
                self.store
                    .record_decision(q.stage, studio_core::DecisionKind::Chose, &label)?;
                self.status()
            }
        }
    }

    /// 内容自评：验收的另一半。
    ///
    /// 技术验收由控制面做（时长、画幅、镜头数、音轨，全部 ffprobe 实测），
    /// 它证明片子是**完整的**，不证明它**好看**。这个方法收的是后者。
    ///
    /// 它**不改 `review.passed`**：片子已经出来了，内容评价改变不了它是否
    /// 完整。它改变的是这次交付有没有留下一份「照自己定的标准打了几分」
    /// 的记录——没有这份记录，作品就不算收尾。
    pub fn self_review(&self, review: studio_core::SelfReview) -> Result<Envelope> {
        let loaded = self.store.load_stage(StageId::Review)?;
        if loaded.state() != StageState::Approved {
            return Err(StudioError::StageNotReady {
                stage: StageId::Review,
                blocked_on: self.current_stage()?.unwrap_or(StageId::Review),
            });
        }
        let attempt = match &loaded {
            LoadedStage::Approved(a) => a.attempt(),
            _ => 1,
        };
        let mut outputs = loaded_outputs(&loaded)
            .cloned()
            .ok_or_else(|| StudioError::internal("验收阶段没有产物"))?;

        // 时间点要落在成片里，所以得先拿到实测时长。
        let post = self.store.load_stage(StageId::Post)?;
        let duration = loaded_outputs(&post)
            .and_then(|o| o["post"]["duration_seconds"].as_f64())
            .ok_or_else(|| StudioError::internal("后期结果里没有实测时长，无法校验自评的时间点"))?;
        studio_core::rubric::validate(&review, duration)?;

        let (met, partial, not_met) = studio_core::rubric::tally(&review);
        if let Some(v) = outputs.get_mut("review") {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("content_review".to_string(), review.to_json());
            }
        }
        // attempt 照旧：这次写的是内容自评，不是重跑一遍验收。
        // 写死 1 会抹掉重试过几次的事实，后面的 retry / undo 还可能撞上
        // 一个已经用过的编号。
        self.store.save_stage(
            StageId::Review,
            StageState::Approved,
            attempt,
            Some(&outputs),
            self.store.stage_summary(StageId::Review)?.as_deref(),
            None,
        )?;
        // 人可读的那份也要跟上，否则打包出去的 bundle 里 stages/review.json
        // 仍然没有 content_review，看的人会以为这份自评根本没交。
        self.mirror_stage_file(StageId::Review, Some(&outputs))?;
        self.store.append_event(
            StageId::Review,
            "content_reviewed",
            &format!(
                "内容自评：{met} 条达成、{partial} 条部分达成、{not_met} 条未达成。{}",
                review.summary
            ),
            None,
        )?;
        self.status()
    }

    /// 内容自评还没交。作品因此还没收尾。
    fn content_review_missing(&self) -> Result<bool> {
        let loaded = self.store.load_stage(StageId::Review)?;
        if loaded.state() != StageState::Approved {
            return Ok(false);
        }
        Ok(loaded_outputs(&loaded)
            .map(|o| o["review"].get("content_review").is_none())
            .unwrap_or(false))
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
        // 要修订的阶段如果正好是一个确定性阶段的 worker 还在跑着，必须先停掉它——
        // 否则旧线程跑完之后会拿着旧的产物覆盖掉这次修订，状态和实际执行就此脱节。
        self.stop_worker();
        self.store.take_snapshot(&format!("修订 {stage} 之前"))?;
        self.clear_recorded_error()?;
        // preview 自己不产出独立内容，修订它统一重定向到 prompt_pack。
        self.revise_inner(stage.revise_target(), message)
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
        // 用户的原话逐字进档案：他在这个阶段说过的话，到下游阶段仍然有效。
        // 见 docs/decisions/ADR-0003。
        self.store
            .record_decision(stage, studio_core::DecisionKind::Rejected, message)?;
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

    /// 干净地重试一个卡住的确定性阶段：先停掉可能还在跑的 worker 线程，
    /// 清掉上一次失败的记录，再让 [`Project::ensure_worker`] 重新把它跑起来。
    ///
    /// 和 [`Project::revise`] 的分工不同：`revise` 是「内容不对，退回草稿
    /// 等 Agent 重新交」，面向 Agent 产出的创作型阶段；`retry_stage` 是
    /// 「内容没问题，只是这次执行失败了（节点抖动、超时），原样再跑一次」，
    /// 只对确定性阶段有意义，不会像 `revise` 那样递增 attempt、
    /// 把下游阶段退回未执行。
    ///
    /// 这就是 issue 里那次故障的根治：`revise()` 只改状态，不取消正在跑的
    /// worker，旧线程跑完照样会覆盖新状态。`retry_stage` 在清错误之前先
    /// `Worker::stop()`，状态和实际执行不会再脱节。
    pub fn retry_stage(&self, stage: StageId) -> Result<Envelope> {
        if stage.kind() != StageKind::Deterministic {
            return Err(StudioError::InvalidTransition {
                stage,
                current: "not_deterministic",
                attempted: "retry_stage",
                allowed: vec!["studio.revise"],
            });
        }
        // 传的阶段必须真的是当前卡住/待执行的那个——`ensure_worker` 之后
        // 重新跑的是 `current_stage()`，不是调用方传的值，两者对不上时
        // 实际重跑的会是另一个阶段，还留一条写着错误阶段名的时间线记录。
        let current = self.current_stage()?;
        if current != Some(stage) {
            return Err(StudioError::RetryStageMismatch {
                requested: stage,
                current,
            });
        }
        self.stop_worker();
        self.clear_recorded_error()?;
        self.store
            .append_event(stage, "retried", &format!("重新尝试 {stage}"), None)?;
        self.status()
    }

    /// 停掉当前挂着的 worker（如果有一个还在跑）。修订、重试确定性阶段
    /// 之前必须先做这件事——旧线程跑完会拿着旧状态覆盖掉新决定。
    fn stop_worker(&self) {
        if let Ok(mut g) = self.worker.lock() {
            if let Some(w) = g.as_mut() {
                w.stop();
            }
        }
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
        // 每次真正要跑（或重试）确定性阶段，都当场重新读一遍 `.env`——
        // 不用打开会话时缓存的那份。这样改完 COMFY_NODE 之后只需要让
        // 控制面再跑一次这个阶段（比如 `studio.revise` 清掉上一次的失败），
        // 不需要重启整个 `studiod serve` 进程。
        let settings = Settings::load(self.program_dir.as_deref(), Some(self.bundle.root()));
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

const STAGE_TOTAL: usize = 10;

/// 回给 Agent 的决定档案条数上限。默认注入的东西必须有硬上限——
/// 整套架构的原则是渐进披露，见 `docs/decisions/ADR-0003`。
const DECISION_LIMIT: usize = 20;

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
    let recorder = ExecRecorder::at(&root);
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
        match loaded.state() {
            StageState::Approved => continue,
            // 门还挂着（比如 preview 执行完但用户/Agent 还没确认），
            // 控制面不该继续往下跑，交回去等应答。
            StageState::AwaitingConfirmation => return,
            StageState::Draft => {}
        }
        if stage.kind() != StageKind::Deterministic {
            // 轮到需要 Agent 或用户的阶段了，交回去。
            return;
        }

        recorder.set_stage(stage);
        progress.set(format!("{stage} 开始"));
        let _ = store.append_event(stage, "started", &format!("控制面开始执行 {stage}"), None);

        let inputs = collect_inputs(&store, stage).unwrap_or(Value::Null);
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs,
            progress: &progress,
            recorder: &recorder,
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
                let text = serde_json::to_string_pretty(&outputs).unwrap_or_default();
                let _ = bundle.write(&format!("stages/{stage}.json"), &format!("{text}\n"));

                match stage.gate() {
                    None => {
                        let _ = store.save_stage(
                            stage,
                            StageState::Approved,
                            loaded.attempt(),
                            Some(&outputs),
                            None,
                            None,
                        );
                        let _ =
                            store.append_event(stage, "succeeded", &format!("{stage} 完成"), None);
                        progress.clear();
                    }
                    Some(gate_id) => {
                        // 确定性阶段也能带门（目前只有 preview）：执行完不直接判过，
                        // 挂起等确认——用跟「Agent 提交带门阶段」完全一样的
                        // AwaitingConfirmation 状态，answer/revise 两条路径都不需要
                        // 知道这次挂起是控制面自己执行出来的还是 Agent 提交出来的。
                        let confirmation = executor
                            .gate_confirmation(stage)
                            .unwrap_or_else(|| generic_gate_confirmation(stage, gate_id));
                        let question = Question {
                            question_id: gate_id.to_string(),
                            stage,
                            prompt: confirmation.prompt,
                            selection_type: confirmation.selection_type,
                            options: confirmation.options,
                        };
                        let _ = store.save_stage(
                            stage,
                            StageState::AwaitingConfirmation,
                            loaded.attempt(),
                            Some(&outputs),
                            None,
                            Some(&question),
                        );
                        let _ = store.append_event(stage, "gate_opened", &question.prompt, None);
                        progress.clear();
                        return;
                    }
                }
            }
            Err(e) => {
                record_failure(&store, stage, &e);
                return;
            }
        }
    }
    progress.clear();
}

/// 通用确认门文案，供没有覆盖 [`StageExecutor::gate_confirmation`] 的执行器
/// 兜底——保证「确定性阶段带门」这件事不会因为某个执行器（尤其是测试用的
/// 假执行器）没接线具体文案就悄悄失效。
fn generic_gate_confirmation(stage: StageId, gate: &str) -> Confirmation {
    Confirmation {
        prompt: format!("{stage} 已由控制面执行完成，确认门 {gate} 等待确认。"),
        selection_type: SelectionType::Single,
        options: vec![
            AnswerOption::new("approve", "确认，继续下一阶段"),
            AnswerOption::revise("revise", "有问题，退回修改"),
        ],
    }
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
