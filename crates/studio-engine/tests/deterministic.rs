//! 确定性阶段由控制面在后台跑完，Agent 只用 status 观察。
//!
//! 这里用一个假执行器，所以不需要 GPU、ComfyUI 或 ffmpeg。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use studio_core::contract::{ProjectStatus, WaitingOn};
use studio_core::{fixtures, Outputs, Result, StageId, StudioError};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_engine::{init_project, Project};

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
        Ok(fixtures::outputs(stage))
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
            p.answer(&q.question_id, "approve").unwrap();
        }
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

#[test]
fn the_control_plane_runs_render_post_review_on_its_own() {
    let (_d, p, calls) = project_at_render(None);

    // 门一通过就轮到控制面。执行器可能已经跑掉几步了，所以只断言
    // 「现在是确定性阶段、等的是控制面」，不去赌具体停在哪一步。
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

    let env = poll_until(&p, 10, |e| e.project.status == ProjectStatus::Completed);
    assert_eq!(
        env.project.status,
        ProjectStatus::Completed,
        "三个确定性阶段应当自动跑完"
    );
    assert_eq!(env.progress.completed, 9);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "render / post / review 各跑一次"
    );

    // 产物落盘且可读
    for s in [StageId::Render, StageId::Post, StageId::Review] {
        let out = p.stage_output(s).unwrap();
        assert!(out.get(s.output_key()).is_some(), "{s} 应当有产物");
        assert!(p.bundle().root().join(format!("stages/{s}.json")).is_file());
    }

    let t = p.timeline(200).unwrap();
    assert_eq!(t.iter().filter(|e| e.kind == "succeeded").count(), 3);
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
