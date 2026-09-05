//! 确定性阶段由控制面在后台跑完，Agent 只用 status 观察。
//!
//! 这里用一个假执行器，所以不需要 GPU、ComfyUI 或 ffmpeg。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use studio_core::contract::Outcome;
use studio_core::contract::{ProjectStatus, WaitingOn};
use studio_core::{fixtures, Outputs, Result, StageId, StudioError};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_engine::{init_project, Project};

/// 挑这道门上第一个「通过」选项。
///
/// 不写死 `"approve"`：选题那道门给的是几个方案各一个通过选项
/// （id 是 concept_id），门上的选项本来就该随阶段而变。
fn first_approve(q: &studio_core::contract::Question) -> String {
    q.options
        .iter()
        .find(|o| o.outcome == Outcome::Approve)
        .unwrap_or_else(|| panic!("{} 的门上没有通过选项", q.stage))
        .id
        .clone()
}

fn submit_through_prompt_pack(p: &Project) {
    for s in [
        StageId::Idea,
        StageId::Selection,
        StageId::Script,
        StageId::Storyboard,
        StageId::VisualAssets,
        StageId::PromptPack,
    ] {
        let env = p
            .submit_stage(
                fixtures::outputs(s),
                Some(fixtures::summary(s)),
                fixtures::confirmation(s),
            )
            .unwrap();
        if let Some(q) = env.pending_question {
            p.answer(&q.question_id, &first_approve(&q)).unwrap();
        }
    }
}

/// 照着 fixtures 产出结果的假执行器。
struct Fake {
    calls: Arc<AtomicUsize>,
    fail_at: Option<StageId>,
}

impl StageExecutor for Fake {
    fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ctx.say(format!("{stage} 处理中"));
        if self.fail_at == Some(stage) {
            return Err(StudioError::ComfyUnavailable {
                tried: vec!["http://127.0.0.1:9001".into()],
            });
        }
        // 上游产物确实传进来了
        assert!(
            ctx.inputs.get("prompt_pack").is_some(),
            "{stage} 应当拿得到提示词包"
        );
        let mut out = fixtures::outputs(stage);
        // 控制面只做技术验收；内容自评是事后由 Agent 用 self_review 补的，
        // 执行器永远不产出它。样例里带着它是因为样例描述的是**做完之后**
        // 的验收产物。
        if stage == StageId::Review {
            if let Some(v) = out.get_mut("review").and_then(|v| v.as_object_mut()) {
                v.remove("content_review");
            }
        }
        Ok(out)
    }
}

fn project_at_render(fail_at: Option<StageId>) -> (tempfile::TempDir, Project, Arc<AtomicUsize>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let exec = Arc::new(Fake {
        calls: Arc::clone(&calls),
        fail_at,
    });
    let p = Project::open_with(&root, None, exec).unwrap();
    submit_through_prompt_pack(&p);
    // preview 也是控制面自动执行的确定性阶段，但带确认门——除非这次故意
    // 让它自己失败，否则替用户把门点过去，好让 render 接着自动跑。
    if fail_at != Some(StageId::Preview) {
        approve_preview_gate(&p);
    }
    (dir, p, calls)
}

/// 轮询 status 直到条件满足或超时——Agent 在真实会话里也是这么做的。
fn poll_until(
    p: &Project,
    secs: u64,
    cond: impl Fn(&studio_core::Envelope) -> bool,
) -> studio_core::Envelope {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let env = p.status().unwrap();
        if cond(&env) || std::time::Instant::now() > deadline {
            return env;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// 等 preview 的确认门弹出来（或者它直接失败挂了），替用户点
/// 「确认，继续」——preview 是唯一「控制面自动执行、但仍带确认门」的
/// 确定性阶段，render 前必须先经过这一步。
fn approve_preview_gate(p: &Project) {
    let env = poll_until(p, 10, |e| {
        e.pending_question.as_ref().map(|q| q.stage) == Some(StageId::Preview)
            || e.blocked_by.is_some()
    });
    if let Some(q) = env.pending_question {
        p.answer(&q.question_id, &first_approve(&q)).unwrap();
    }
}

/// 一份合规的内容自评：五个维度各一条，每条带时间点和证据。
fn self_review() -> studio_core::SelfReview {
    let review = fixtures::outputs(StageId::Review);
    serde_json::from_value(review["review"]["content_review"].clone()).unwrap()
}

#[test]
fn the_control_plane_runs_render_post_review_on_its_own() {
    let (_d, p, calls) = project_at_render(None);

    // preview 的门已经在 project_at_render 里被自动确认过。执行器可能
    // 已经跑掉几步了，所以只断言「现在是确定性阶段、等的是控制面」，
    // 不去赌具体停在哪一步。
    let env = p.status().unwrap();
    assert!(
        matches!(
            env.project.stage,
            StageId::Render | StageId::Post | StageId::Review
        ) || env.project.status == ProjectStatus::Completed,
        "实际停在 {:?}",
        env.project.stage
    );
    if env.project.status != ProjectStatus::Completed {
        assert_eq!(
            env.waiting_on,
            WaitingOn::System,
            "确定性阶段不该回头找 Agent 要东西"
        );
        assert!(env.pending_question.is_none());
    }

    // 十个阶段跑完之后作品还没收尾：技术验收出了，内容自评还没交。
    let env = poll_until(&p, 10, |e| e.progress.completed == 10);
    assert_eq!(env.progress.completed, 10);
    assert_eq!(
        env.next_action.as_ref().map(|a| a.kind),
        Some(studio_core::contract::ActionKind::SelfReview),
        "技术验收只证明片子是完整的，还差「它好不好看」那一半"
    );
    assert_eq!(env.waiting_on, WaitingOn::Agent);

    let env = p.self_review(self_review()).unwrap();
    assert_eq!(
        env.project.status,
        ProjectStatus::Completed,
        "四个确定性阶段应当自动跑完（preview 的门已提前确认）"
    );
    assert_eq!(env.progress.completed, 10);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "preview / render / post / review 各跑一次"
    );

    // 产物落盘且可读
    for s in [
        StageId::Preview,
        StageId::Render,
        StageId::Post,
        StageId::Review,
    ] {
        let out = p.stage_output(s).unwrap();
        assert!(out.get(s.output_key()).is_some(), "{s} 应当有产物");
        assert!(p.bundle().root().join(format!("stages/{s}.json")).is_file());
    }

    let t = p.timeline(200).unwrap();
    assert_eq!(
        t.iter().filter(|e| e.kind == "succeeded").count(),
        3,
        "render / post / review 无门，直接判过；preview 走的是 gate_opened + approved"
    );
    // 5 个创作型确认门（selection/script/storyboard/visual_assets/prompt_pack）
    // 加上 preview 自己那道门，一共 6 次。
    assert_eq!(t.iter().filter(|e| e.kind == "gate_opened").count(), 6);
}

/// 执行失败要变成带 remedy 的阻塞，而不是默默卡住。
#[test]
fn a_failed_stage_becomes_a_blocked_envelope() {
    let (_d, p, _) = project_at_render(Some(StageId::Render));

    let env = poll_until(&p, 10, |e| e.blocked_by.is_some());
    let b = env.blocked_by.expect("失败必须在信封里看得见");
    assert_eq!(b.code, "comfy_unavailable");
    assert!(!b.remedy.is_empty());
    assert!(b.remedy.contains("COMFY_NODES"));
    assert_eq!(env.project.status, ProjectStatus::Blocked);

    // 失败挂着的时候不会闷头重试
    let before = p
        .timeline(200)
        .unwrap()
        .iter()
        .filter(|e| e.kind == "failed")
        .count();
    for _ in 0..5 {
        p.status().unwrap();
    }
    let after = p
        .timeline(200)
        .unwrap()
        .iter()
        .filter(|e| e.kind == "failed")
        .count();
    assert_eq!(before, after, "失败记录还在时不该反复重试");

    // 修订之后阻塞解除
    let env = p
        .revise(StageId::PromptPack, "换一个更省显存的尺寸")
        .unwrap();
    assert!(env.blocked_by.is_none());
    assert_eq!(env.project.stage, StageId::PromptPack);
}

/// 编辑 `.env` 之后不该要求重启整个进程：同一个 `Project` 实例重试
/// 确定性阶段时，应当当场重新读取 `.env`，而不是沿用打开会话那一刻
/// 缓存的配置。这是 `comfy_unavailable` remedy 现在明确要求的行为——
/// 配好 `COMFY_NODES` 后调 `studio.revise` 重试，不需要重开会话。
#[test]
fn retrying_after_editing_env_picks_up_the_new_nodes_without_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    std::fs::write(root.join(".env"), "COMFY_NODES=http://before:9001\n").unwrap();

    let attempts = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    struct RecordingExecutor {
        attempts: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl StageExecutor for RecordingExecutor {
        fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs> {
            // 只关心 render 自己的节点列表和重试次数；preview 顺带跑过去就好。
            if stage != StageId::Render {
                return Ok(fixtures::outputs(stage));
            }
            let nodes = ctx.settings.comfy_nodes();
            let is_first = {
                let mut a = self.attempts.lock().unwrap();
                a.push(nodes.clone());
                a.len() == 1
            };
            if is_first {
                return Err(StudioError::ComfyUnavailable { tried: nodes });
            }
            Ok(fixtures::outputs(stage))
        }
    }
    let exec = Arc::new(RecordingExecutor {
        attempts: Arc::clone(&attempts),
    });

    // Project::open_with 在这一刻缓存的 settings 之后不会再被用来跑 render——
    // 这正是本测试要证明的事。
    let p = Project::open_with(&root, None, exec).unwrap();
    submit_through_prompt_pack(&p);
    approve_preview_gate(&p);

    let env = poll_until(&p, 10, |e| e.blocked_by.is_some());
    assert_eq!(env.blocked_by.unwrap().code, "comfy_unavailable");
    assert_eq!(
        attempts.lock().unwrap().last().unwrap(),
        &vec!["http://before:9001".to_string()],
        "第一次尝试应当看到打开会话时 .env 里的节点"
    );

    // 相当于 Agent 照 remedy 做：编辑 .env，然后调 studio.revise 重试——
    // 全程没有重开 Project，也没有碰 studiod 的任何子命令。
    std::fs::write(root.join(".env"), "COMFY_NODES=http://after:9002\n").unwrap();
    let env = p
        .revise(StageId::Render, "已经把 COMFY_NODES 换成新节点，重试")
        .unwrap();
    assert!(env.blocked_by.is_none(), "revise 之后阻塞应当解除");

    let env = poll_until(&p, 10, |e| e.project.status == ProjectStatus::Completed);
    assert_eq!(env.project.status, ProjectStatus::Completed);
    assert_eq!(
        attempts.lock().unwrap().last().unwrap(),
        &vec!["http://after:9002".to_string()],
        "重试时应当当场重新读取 .env，而不是沿用打开会话时缓存的节点"
    );
}

#[test]
fn progress_shows_up_in_the_envelope_while_running() {
    let (_d, p, _) = project_at_render(None);
    // 执行很快，能抓到 note 或者已经跑完都算正常；关键是字段存在且不乱。
    let env = poll_until(&p, 10, |e| {
        e.note.is_some() || e.project.status == ProjectStatus::Completed
    });
    if let Some(note) = &env.note {
        assert!(
            note.contains("处理中") || note.contains("开始"),
            "实际：{note}"
        );
    }
}

/// 一个跑得很慢、会主动检查取消标志的假执行器——用来证明修订/重试
/// 真的会打断正在跑的 worker，而不是让它悄悄跑完再被状态盖过去。
struct SlowExecutor {
    started: Arc<AtomicBool>,
    saw_cancel: Arc<AtomicBool>,
}

impl StageExecutor for SlowExecutor {
    fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs> {
        if stage != StageId::Render {
            return Ok(fixtures::outputs(stage));
        }
        self.started.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if ctx.is_cancelled() {
                self.saw_cancel.store(true, Ordering::SeqCst);
                return Err(StudioError::internal("渲染被中断"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut out = fixtures::outputs(stage);
        // 控制面只做技术验收；内容自评是事后由 Agent 用 self_review 补的，
        // 执行器永远不产出它。样例里带着它是因为样例描述的是**做完之后**
        // 的验收产物。
        if stage == StageId::Review {
            if let Some(v) = out.get_mut("review").and_then(|v| v.as_object_mut()) {
                v.remove("content_review");
            }
        }
        Ok(out)
    }
}

fn project_stuck_mid_render() -> (tempfile::TempDir, Project, Arc<AtomicBool>, Arc<AtomicBool>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let exec = Arc::new(SlowExecutor {
        started: Arc::clone(&started),
        saw_cancel: Arc::clone(&saw_cancel),
    });
    let p = Project::open_with(&root, None, exec).unwrap();
    submit_through_prompt_pack(&p);
    // SlowExecutor 对 preview 直接返回（只在 render 上慢），门一开就能过。
    approve_preview_gate(&p);

    // 逼 ensure_worker 把它跑起来，再等到执行器真的进了 render 循环。
    p.status().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.load(Ordering::SeqCst), "worker 应当已经进入 render");
    (dir, p, started, saw_cancel)
}

/// 这是 issue 复盘的那个 bug：`revise()` 之前只改状态，不取消正在跑的
/// worker，旧线程跑完会拿着旧状态把新决定覆盖掉。现在 `revise` 必须先
/// 停掉它。
#[test]
fn revise_stops_an_in_flight_worker_before_touching_state() {
    let (_d, p, _started, saw_cancel) = project_stuck_mid_render();

    p.revise(StageId::Render, "换一个更省显存的尺寸").unwrap();

    assert!(
        saw_cancel.load(Ordering::SeqCst),
        "revise 必须先取消掉正在跑的 worker，不能放任它继续跑"
    );
}

/// `retry_stage` 只对确定性阶段有意义——创作型/混合型阶段的重试是
/// `studio.revise` 的事。
#[test]
fn retry_stage_requires_a_deterministic_stage() {
    let (_d, p, _) = project_at_render(None);
    let e = p.retry_stage(StageId::PromptPack).unwrap_err();
    assert_eq!(e.code(), "invalid_transition");
    assert!(e.remedy().contains("studio.revise") || e.remedy().contains("studio.status"));
}

/// `retry_stage` 必须拒绝「传的阶段不是当前真正卡住的那个」——否则会悄悄
/// 重跑当前阶段却留一条写着别的阶段名的时间线记录，跟实际执行对不上。
#[test]
fn retry_stage_rejects_a_stage_that_is_not_the_current_one() {
    let (_d, p, _started, _saw_cancel) = project_stuck_mid_render();

    // 此刻真正卡住的是 render；请求重试一个不同的确定性阶段应当被拒绝，
    // 而不是悄悄把 render 重跑一遍却记成「重试了 post」。
    let e = p.retry_stage(StageId::Post).unwrap_err();
    assert_eq!(e.code(), "retry_stage_mismatch");
    assert!(e.message().contains("post"));
    assert!(e.message().contains("render"));
    assert!(e.remedy().contains("studio.retry_stage(\"render\")"));

    let before = p
        .timeline(200)
        .unwrap()
        .iter()
        .filter(|e| e.kind == "retried")
        .count();
    assert_eq!(before, 0, "被拒绝的请求不该留下 retried 记录");
}

/// `retry_stage` 同样必须先停掉可能还在跑的 worker，才清错误重新触发。
#[test]
fn retry_stage_stops_an_in_flight_worker_before_retrying() {
    let (_d, p, _started, saw_cancel) = project_stuck_mid_render();

    p.retry_stage(StageId::Render).unwrap();

    assert!(
        saw_cancel.load(Ordering::SeqCst),
        "retry_stage 必须先取消掉正在跑的 worker"
    );
}

/// 失败挂着时，`retry_stage` 清掉记录、干净重试，不需要经过 revise
/// 那套「退回草稿、下游全部退回未执行」的逻辑。
#[test]
fn retry_stage_clears_a_recorded_failure_and_reruns_it() {
    let (_d, p, calls) = project_at_render(Some(StageId::Render));
    let env = poll_until(&p, 10, |e| e.blocked_by.is_some());
    assert_eq!(env.blocked_by.unwrap().code, "comfy_unavailable");
    let before = calls.load(Ordering::SeqCst);

    p.retry_stage(StageId::Render).unwrap();

    // 这个假执行器每次都会失败在 Render，所以会再次卡住——这里只
    // 关心「清掉了上一次记录、确实又跑了一次」，不是「这次会成功」。
    //
    // 不断言 retry_stage 返回的那个信封 blocked_by 为空：它清完记录就把
    // worker 拉起来了，而这个执行器是立刻失败的，新的失败完全可能在
    // 信封读取之前就已经记上——那不是 bug，是真的又失败了一次。
    // 可靠的证据是「执行器被再调用了一次」和时间线上的那条 retried。
    poll_until(&p, 10, |_| calls.load(Ordering::SeqCst) > before);
    assert!(
        calls.load(Ordering::SeqCst) > before,
        "retry_stage 应当让执行器再跑一次"
    );
    let retried = p
        .timeline(200)
        .unwrap()
        .iter()
        .filter(|e| e.kind == "retried")
        .count();
    assert_eq!(retried, 1, "retry_stage 应当留一条可审计的时间线记录");
}

/// `studio.comfy.exclude_node` 排除的节点必须真的从执行器看到的
/// `Settings::comfy_nodes()` 里消失——这是选节点时会跳过它的前提。
#[test]
fn excluded_nodes_disappear_from_the_settings_the_executor_sees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    std::fs::write(
        root.join(".env"),
        "COMFY_NODES=http://a:9001,http://b:9002\n",
    )
    .unwrap();

    let seen_nodes = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    struct RecordingExecutor {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl StageExecutor for RecordingExecutor {
        fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs> {
            if stage == StageId::Render {
                self.seen.lock().unwrap().push(ctx.settings.comfy_nodes());
            }
            Ok(fixtures::outputs(stage))
        }
    }
    let exec = Arc::new(RecordingExecutor {
        seen: Arc::clone(&seen_nodes),
    });
    let p = Project::open_with(&root, None, exec).unwrap();
    // 必须在触发 render 的 worker 之前排除——`ensure_worker` 在启动线程那一刻
    // 就把当下的排除名单快照进 Settings，之后再排除对这次已经在跑的执行不生效。
    p.exclude_comfy_node("http://a:9001/").unwrap();
    submit_through_prompt_pack(&p);
    approve_preview_gate(&p);

    let env = poll_until(&p, 10, |e| e.project.status == ProjectStatus::Completed);
    assert_eq!(env.project.status, ProjectStatus::Completed);
    assert_eq!(
        seen_nodes.lock().unwrap().last().unwrap(),
        &vec!["http://b:9002".to_string()],
        "被排除的节点不该出现在执行器看到的节点列表里"
    );
}
