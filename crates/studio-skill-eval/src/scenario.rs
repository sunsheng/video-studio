//! 脚本场景：确定性、不需要任何 LLM，能直接进 `cargo test --workspace`。
//!
//! 跟 Agent 场景（见 ADR-0004）的边界很清楚：这里的每一步调用序列都是
//! 场景脚本自己定的，不是某个 LLM 读了 SKILL.md 之后自己决定的——验证的
//! 是协议层、状态机、门逻辑没有回归，不是 skill 文档措辞好不好。

use crate::harness::Harness;
use crate::judge::{structural, Verdict};
use serde::{Deserialize, Serialize};
use studio_core::{fixtures, StageId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario_id: String,
    pub description: String,
    pub verdicts: Vec<Verdict>,
    pub passed: bool,
}

fn finish(scenario_id: &str, description: &str, verdicts: Vec<Verdict>) -> ScenarioResult {
    let passed = verdicts.iter().all(|v| v.passed);
    ScenarioResult {
        scenario_id: scenario_id.to_string(),
        description: description.to_string(),
        passed,
        verdicts,
    }
}

/// 一个脚本场景的入口函数。
pub type ScenarioFn = fn() -> ScenarioResult;

/// 场景的名字信息，跟"跑一遍"分开——`list` 只需要前者，不该为了报个
/// 名字就把场景真的跑一遍（起子进程、建临时 bundle、走一圈 MCP 调用）。
pub struct ScenarioMeta {
    pub id: &'static str,
    pub description: &'static str,
    pub run: ScenarioFn,
}

/// 全部内置脚本场景。
pub fn all() -> Vec<ScenarioMeta> {
    vec![
        ScenarioMeta {
            id: "golden_six_stage_with_revise",
            description: GOLDEN_SIX_STAGE_DESCRIPTION,
            run: golden_six_stage_with_revise as ScenarioFn,
        },
        ScenarioMeta {
            id: "concurrent_open_reports_busy_with_pid",
            description: CONCURRENT_OPEN_DESCRIPTION,
            run: concurrent_open_reports_busy_with_pid as ScenarioFn,
        },
    ]
}

pub fn run(id: &str) -> Option<ScenarioResult> {
    all().into_iter().find(|m| m.id == id).map(|m| (m.run)())
}

const GOLDEN_SIX_STAGE_DESCRIPTION: &str = "取自 2026-09-03 那次真实事故的重放：\
    提交每镜头 2 秒的剧本 → 用户说「不要固定 2 秒」→ 重新提交智能时长版 → \
    确认，一路走到提示词包。是 scripts/replay-protocol.py 的等价物，\
    区别是这里跑的是真实编译出的 studiod 二进制 + 真实 stdio JSON-RPC，\
    能直接进 cargo test --workspace，不需要额外装 Python。";

/// 原 `scripts/replay-protocol.py` 的等价物：走完提交给 ComfyUI 之前的
/// 六个阶段，中间重放一次「不要固定 2 秒」的修订。
pub fn golden_six_stage_with_revise() -> ScenarioResult {
    let mut h = Harness::fresh();
    h.advance(StageId::Idea);
    h.advance(StageId::Selection);

    // 先交一版每镜头 2 秒的——这正是那次事故翻车的起点。
    let mut even = fixtures::outputs(StageId::Script);
    for (i, beat) in even["script"]["story_arc"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        beat["start"] = serde_json::json!(i as f64 * 2.0);
        beat["end"] = serde_json::json!((i as f64 + 1.0) * 2.0);
        beat["duration_seconds"] = serde_json::json!(2.0);
    }
    let (env, err) = h.call(
        "studio.submit_stage",
        serde_json::json!({
            "outputs": even,
            "summary": "每镜头 2 秒",
            "confirmation": fixtures::confirmation(StageId::Script),
        }),
    );
    if err {
        return finish(
            "golden_six_stage_with_revise",
            GOLDEN_SIX_STAGE_DESCRIPTION,
            vec![Verdict {
                name: "能提交平均时长版剧本".into(),
                passed: false,
                detail: format!("{env}"),
            }],
        );
    }

    // 用户说「不要固定 2 秒」——健康的实现应该是一次 revise 加一次
    // submit，前身项目那次花了 18 次调用。
    let (env, err) = h.call(
        "studio.revise",
        serde_json::json!({ "stage": "script", "message": "不要固定2秒，要根据镜头内容智能分配" }),
    );
    if err {
        return finish(
            "golden_six_stage_with_revise",
            GOLDEN_SIX_STAGE_DESCRIPTION,
            vec![Verdict {
                name: "revise 不该失败".into(),
                passed: false,
                detail: format!("{env}"),
            }],
        );
    }

    h.advance(StageId::Script);
    h.advance(StageId::Storyboard);
    h.advance(StageId::VisualAssets);
    h.advance(StageId::PromptPack);

    let (status, _) = h.call("studio.status", serde_json::json!({}));

    let records = h.trace();
    let verdicts = vec![
        structural::all_blocks_carry_a_remedy(&records),
        structural::no_state_drift(&records),
        structural::revise_round_trips_within(&records, 2),
        structural::stages_reached(
            &records,
            &[
                StageId::Idea,
                StageId::Selection,
                StageId::Script,
                StageId::Storyboard,
                StageId::VisualAssets,
                StageId::PromptPack,
            ],
        ),
        Verdict {
            name: "停在 preview，等控制面自动执行".into(),
            passed: status["project"]["stage"] == "preview" && status["progress"]["completed"] == 6,
            detail: format!(
                "阶段={}，完成={}",
                status["project"]["stage"], status["progress"]["completed"]
            ),
        },
    ];
    finish(
        "golden_six_stage_with_revise",
        GOLDEN_SIX_STAGE_DESCRIPTION,
        verdicts,
    )
}

const CONCURRENT_OPEN_DESCRIPTION: &str = "两个真实的 studiod 子进程打开同一个 \
    bundle：第二个必须拿到 project_busy、附上第一个进程真实的 PID，且这条 \
    remedy 不能点名任何二进制——这是 ADR-0004 记录的那次真实缺陷（remedy \
    文案里直接写着 `studiod init`）的回归防护。";

/// 两个真实进程打开同一个 bundle：第二个必须拿到 `project_busy`，附上
/// 第一个进程真实的 PID，且 remedy 不点名任何二进制。
pub fn concurrent_open_reports_busy_with_pid() -> ScenarioResult {
    let mut first = Harness::fresh();
    let first_pid = first.pid();
    // 拿到第一次回应才能确定 first 已经真正打开并锁住了这个 bundle——
    // spawn() 只是把子进程发出去，它执行到 Project::open() 拿到 flock
    // 是需要时间的，不等这一步会跟 second 的启动产生竞态。
    first.call("studio.status", serde_json::json!({}));
    let mut second = Harness::attach(first.root.clone());

    let (env, err) = second.call("studio.status", serde_json::json!({}));
    let blocked = &env["blocked_by"];

    let verdicts = vec![
        Verdict {
            name: "第二个会话被拒绝".into(),
            passed: err && blocked["code"] == "project_busy",
            detail: format!("code={}", blocked["code"]),
        },
        Verdict {
            name: "附带真实的持有者 PID".into(),
            passed: blocked["message"]
                .as_str()
                .unwrap_or_default()
                .contains(&first_pid.to_string()),
            detail: format!("持有者进程={first_pid}，message={}", blocked["message"]),
        },
        structural::remedy_does_not_name_binaries(&env),
    ];

    drop(first); // 保证它比 second 活得久，不是提前被回收——显式写出来免得读者误会

    finish(
        "concurrent_open_reports_busy_with_pid",
        CONCURRENT_OPEN_DESCRIPTION,
        verdicts,
    )
}
