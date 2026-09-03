//! 一部作品的生命周期，走真实的 bundle 与 SQLite。
//!
//! 这些用例不需要 GPU、不需要 ComfyUI、不需要 ffmpeg——
//! 提交给 ComfyUI 之前的每一步都可以在开发环境完整验证。

use studio_core::contract::{ProjectStatus, WaitingOn};
use studio_core::{fixtures, StageId};
use studio_engine::{init_project, Project};

fn new_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    let p = Project::open(&root, None).unwrap();
    (dir, p)
}

/// 提交一个阶段并在有门时确认通过。
fn advance(p: &Project, stage: StageId) {
    let env = p
        .submit_stage(fixtures::outputs(stage), Some(fixtures::summary(stage)), fixtures::confirmation(stage))
        .unwrap_or_else(|e| panic!("提交 {stage} 失败：{e}"));
    if let Some(q) = env.pending_question {
        p.answer(&q.question_id, "approve").unwrap();
    }
}

#[test]
fn a_fresh_project_asks_the_agent_for_the_brief() {
    let (_d, p) = new_project();
    let env = p.status().unwrap();
    env.assert_consistent().unwrap();
    assert_eq!(env.project.stage, StageId::Idea);
    assert_eq!(env.waiting_on, WaitingOn::Agent);
    assert_eq!(env.progress.completed, 0);
    let next = env.next_action.unwrap();
    assert_eq!(next.capability, studio_core::Capability::Idea);
    assert_eq!(next.required_outputs, vec!["brief"]);
    assert_eq!(next.schema_ref, "idea");
    assert!(next.gate.is_none(), "idea 阶段没有确认门");
}

#[test]
fn gateless_stage_advances_without_a_question() {
    let (_d, p) = new_project();
    let env = p.submit_stage(fixtures::outputs(StageId::Idea), Some("brief 完成"), None).unwrap();
    assert!(env.pending_question.is_none());
    assert_eq!(env.project.stage, StageId::Selection);
    assert_eq!(env.progress.completed, 1);
}

#[test]
fn gated_stage_waits_for_the_user() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    let env = p
        .submit_stage(fixtures::outputs(StageId::Selection), None, fixtures::confirmation(StageId::Selection))
        .unwrap();
    env.assert_consistent().unwrap();
    assert_eq!(env.waiting_on, WaitingOn::User);
    let q = env.pending_question.unwrap();
    assert_eq!(q.question_id, "selection.approval");
    assert!(env.next_action.is_none(), "等用户时不该同时给 Agent 下一步");
}

#[test]
fn schema_violation_names_the_exact_field() {
    let (_d, p) = new_project();
    let mut bad = fixtures::outputs(StageId::Idea);
    bad["brief"].as_object_mut().unwrap().remove("aspect_ratio");
    let e = p.submit_stage(bad, None, None).unwrap_err();
    assert_eq!(e.code(), "schema_violation");
    assert!(e.message().contains("brief.aspect_ratio"), "实际：{}", e.message());
    assert!(e.remedy().contains("studio.schema"));
}

#[test]
fn submitting_while_a_gate_is_open_says_exactly_what_to_do() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    p.submit_stage(fixtures::outputs(StageId::Selection), None, fixtures::confirmation(StageId::Selection)).unwrap();

    let e = p
        .submit_stage(fixtures::outputs(StageId::Selection), None, fixtures::confirmation(StageId::Selection))
        .unwrap_err();
    assert_eq!(e.code(), "gate_pending");
    assert!(e.remedy().contains("studio.answer"));
    assert!(e.remedy().contains("studio.revise"));

    // 信封里的 blocked_by 必须带着 remedy——前身项目这个字段永远是 null。
    let env = p.envelope_for_error(&e);
    let b = env.blocked_by.unwrap();
    assert_eq!(b.code, "gate_pending");
    assert!(!b.remedy.is_empty());
}

/// 2026-09-03 那次会话的完整重放：用户在门上说「不要固定 2 秒」。
/// 前身项目在这里花了 10 分钟 18 次调用并绕去写 SQL；这里是三次调用。
#[test]
fn the_revise_round_trip_takes_three_calls() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    advance(&p, StageId::Selection);

    // 1. 提交每镜头 2 秒的版本
    let mut even = fixtures::outputs(StageId::Script);
    {
        let arc = even["script"]["story_arc"].as_array_mut().unwrap();
        for (i, beat) in arc.iter_mut().enumerate() {
            beat["start"] = serde_json::json!(i as f64 * 2.0);
            beat["end"] = serde_json::json!((i as f64 + 1.0) * 2.0);
            beat["duration_seconds"] = serde_json::json!(2.0);
        }
    }
    let env = p.submit_stage(even, Some("每镜头 2 秒"), fixtures::confirmation(StageId::Script)).unwrap();
    assert_eq!(env.waiting_on, WaitingOn::User);

    // 2. 用户说「不要固定2秒，要根据镜头内容智能分配」
    let env = p.revise(StageId::Script, "不要固定2秒，要根据镜头内容智能分配").unwrap();
    assert_eq!(env.waiting_on, WaitingOn::Agent, "修订之后应当立刻回到等 Agent 提交");
    assert!(env.pending_question.is_none(), "修订必须彻底释放确认门");
    assert!(env.blocked_by.is_none(), "修订不该留下任何阻塞");

    // 3. 立刻重新提交智能时长版——不存在 task already claimed
    let env = p
        .submit_stage(fixtures::outputs(StageId::Script), Some("按内容智能分配"), fixtures::confirmation(StageId::Script))
        .unwrap();
    let q = env.pending_question.expect("同一个 question_id 必须能重新挂起");
    assert_eq!(q.question_id, "script.approval");

    // 用户确认，进入分镜
    let env = p.answer(&q.question_id, "approve").unwrap();
    assert_eq!(env.project.stage, StageId::Storyboard);
    assert_eq!(env.progress.completed, 3);
}

#[test]
fn choosing_the_revise_option_sends_the_stage_back_not_forward() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    let env = p
        .submit_stage(fixtures::outputs(StageId::Selection), None, fixtures::confirmation(StageId::Selection))
        .unwrap();
    let q = env.pending_question.unwrap();

    let env = p.answer(&q.question_id, "revise").unwrap();
    assert_eq!(env.project.stage, StageId::Selection, "选『先修改』不该推进阶段");
    assert_eq!(env.waiting_on, WaitingOn::Agent);
    assert!(env.pending_question.is_none());
}

/// 修订只退回被点名的那个阶段，下游一概不动。
///
/// 后果是明确的：改完剧本再确认后，已经通过的分镜不会被自动作废，
/// 流程会直接走到视觉资产。要重做分镜就再显式 revise 一次。
/// 程序不替用户判断什么该作废——版本与备份由用户用 cp -r 或 studiod pack 管理。
#[test]
fn revise_rewinds_only_the_named_stage() {
    let (_d, p) = new_project();
    for s in [StageId::Idea, StageId::Selection, StageId::Script, StageId::Storyboard] {
        advance(&p, s);
    }
    assert_eq!(p.status().unwrap().project.stage, StageId::VisualAssets);

    p.revise(StageId::Script, "把碰杯换成拍照").unwrap();
    let env = p.status().unwrap();
    assert_eq!(env.project.stage, StageId::Script, "当前位置退回剧本");
    assert_eq!(env.progress.completed, 3, "只有剧本退回草稿，分镜仍算已通过");

    // 分镜的产物原封不动
    let sb = p.stage_output(StageId::Storyboard).unwrap();
    assert!(sb["storyboard"]["shots"].is_array());

    // 重新提交剧本并确认后，流程接着往下走（不会重复要求做分镜）
    advance(&p, StageId::Script);
    assert_eq!(p.status().unwrap().project.stage, StageId::VisualAssets);

    // 想重做分镜是显式动作
    p.revise(StageId::Storyboard, "按新剧本重画").unwrap();
    assert_eq!(p.status().unwrap().project.stage, StageId::Storyboard);
}

#[test]
fn undo_only_applies_to_an_approved_stage() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    p.undo(StageId::Idea).unwrap();
    assert_eq!(p.status().unwrap().project.stage, StageId::Idea);

    let e = p.undo(StageId::Idea).unwrap_err();
    assert_eq!(e.code(), "invalid_transition");
    assert!(e.remedy().contains("studio.submit_stage"));
}

#[test]
fn stage_outputs_are_mirrored_as_readable_json() {
    let (_d, p) = new_project();
    advance(&p, StageId::Idea);
    let path = p.bundle().root().join("stages/idea.json");
    assert!(path.is_file(), "阶段产物应当同步成人可读的 stages/<stage>.json");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("千岛湖"));
    assert!(!text.contains(p.bundle().root().to_str().unwrap()), "bundle 内文件不得写入绝对路径");
}

#[test]
fn the_six_stages_before_comfyui_run_end_to_end() {
    let (_d, p) = new_project();
    for s in [
        StageId::Idea,
        StageId::Selection,
        StageId::Script,
        StageId::Storyboard,
        StageId::VisualAssets,
        StageId::PromptPack,
    ] {
        advance(&p, s);
    }
    let env = p.status().unwrap();
    env.assert_consistent().unwrap();
    assert_eq!(env.progress.completed, 6);
    assert_eq!(env.project.stage, StageId::Render, "六个阶段跑完，下一步才轮到 ComfyUI");
    assert_eq!(env.waiting_on, WaitingOn::System, "render 是确定性阶段，由控制面执行");
    assert_eq!(env.project.status, ProjectStatus::Active);

    // 提示词包已经带着可提交给 ComfyUI 的全部参数
    let pack = p.stage_output(StageId::PromptPack).unwrap();
    let shots = pack["prompt_pack"]["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 5);
    assert!(shots.iter().all(|s| s["workflow"].is_string() && s["seed"].is_number()));

    // 时间线记下了每一步
    let t = p.timeline(100).unwrap();
    assert!(t.iter().filter(|e| e.kind == "submitted").count() >= 6);
    assert!(t.iter().filter(|e| e.kind == "approved").count() >= 5);
}

#[test]
fn a_second_session_on_the_same_bundle_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    let _first = Project::open(&root, None).unwrap();
    let e = Project::open(&root, None).unwrap_err();
    assert_eq!(e.code(), "project_busy");
    assert!(e.remedy().contains("studiod init"));
}

#[test]
fn reopening_resumes_from_disk_not_from_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("千岛湖.studio");
    init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
    {
        let p = Project::open(&root, None).unwrap();
        advance(&p, StageId::Idea);
        advance(&p, StageId::Selection);
    }
    // 新会话：上下文不在对话里，在文件夹里。
    let p = Project::open(&root, None).unwrap();
    let env = p.status().unwrap();
    assert_eq!(env.project.stage, StageId::Script);
    assert_eq!(env.progress.completed, 2);
    assert_eq!(env.project.title, fixtures::TITLE);
}

#[test]
fn export_refuses_before_post_is_done() {
    let (_d, p) = new_project();
    let e = p.export().unwrap_err();
    assert_eq!(e.code(), "stage_not_ready");
}
