//! Agent 场景库：跟脚本场景（见 `scenario.rs`）共享同一个 `Harness`/
//! `AgentDriver` 基础设施，区别是这里的调用序列由一个真实 LLM 自己
//! 决定，不是场景脚本写死的。设计与"起步集合"名单见
//! `docs/decisions/ADR-0004-skill-evaluation.md`。
//!
//! 每个场景配一份专属的 [`ScriptedUser`]——门上该说什么是场景叙事的
//! 一部分，不是通用配置，所以跟场景定义绑在一起返回，而不是让调用方
//! 自己现拼。

use crate::driver::AgentScenario;
use crate::judge::{structural, Verdict};
use crate::user_sim::ScriptedUser;
use serde_json::Value;
use std::path::Path;
use studio_core::{DecisionKind, StageId};

/// 全部内置 Agent 场景，配好各自的虚拟用户剧本。
pub fn all() -> Vec<(AgentScenario, ScriptedUser)> {
    vec![
        incident_replay_2026_09_03(),
        ambiguous_user_input_handling(),
        retry_vs_revise_confusion_probe(),
        capability_boundary_probe(),
        declarative_shape_probe(),
        decision_archive_crosses_stages(),
    ]
}

pub fn find(id: &str) -> Option<(AgentScenario, ScriptedUser)> {
    all().into_iter().find(|(s, _)| s.id == id)
}

fn read_stage_json(bundle_root: &Path, stage: StageId) -> Option<Value> {
    let text = std::fs::read_to_string(
        bundle_root
            .join("stages")
            .join(format!("{}.json", stage.as_str())),
    )
    .ok()?;
    serde_json::from_str(&text).ok()
}

const COMMON_BRIEF: &str = "拍一支 15 秒的短视频：一位女生在千岛湖边散步，追随夕阳，随性又放松。";

fn incident_replay_2026_09_03() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "incident_replay_2026_09_03",
            description: "把人工验收自动化：docs/e2e.md 现在手工跑的那个剧本——交一版每镜头固定时长的剧本，用户说「不要固定时长」，应当一次 revise 加一次 submit_stage 收敛，不是像前身项目那次事故一样兜 18 圈。",
            brief: COMMON_BRIEF,
            expected_stage: StageId::PromptPack,
            verdicts: |run| {
                vec![
                    structural::all_blocks_carry_a_remedy(&run.trace),
                    structural::no_state_drift(&run.trace),
                    structural::revise_round_trips_within(&run.trace, 2),
                ]
            },
        },
        ScriptedUser::new(&[(
            StageId::Script,
            "不要把每个镜头的时长都写成一样，要根据镜头内容智能分配",
        )]),
    )
}

fn ambiguous_user_input_handling() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "ambiguous_user_input_handling",
            description: "创意里故意夹一个明显笔误（「20色女性」），检查 Agent 是否按 idea skill 的指示按最合理理解处理并写进 assumptions，而不是卡住反复追问用户「什么是20色」。",
            brief: "拍个视频，主角是一位20色女性，在雨天街头撑伞走过，安静又坚定。",
            expected_stage: StageId::Selection,
            verdicts: |run| {
                let idea = read_stage_json(&run.bundle_root, StageId::Idea);
                let assumptions = idea
                    .as_ref()
                    .and_then(|v| v["brief"]["assumptions"].as_array().cloned())
                    .unwrap_or_default();
                vec![Verdict {
                    name: "笔误按最合理理解处理并写进 assumptions".into(),
                    passed: !assumptions.is_empty(),
                    detail: if idea.is_none() {
                        "没有找到 stages/idea.json——Agent 可能没有提交过 idea 阶段。".into()
                    } else {
                        format!("assumptions = {assumptions:?}")
                    },
                }]
            },
        },
        ScriptedUser::new(&[]),
    )
}

fn retry_vs_revise_confusion_probe() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "retry_vs_revise_confusion_probe",
            description: "开发环境一定没有真实 ComfyUI，走到 preview 会自然撞上 comfy_unavailable——检查 Agent 会不会正确选 studio.retry_stage，而不是误用 studio.revise（comfyui skill 明确写了这条区分）。",
            brief: COMMON_BRIEF,
            expected_stage: StageId::Preview,
            verdicts: |run| {
                let hit_comfy_unavailable = run
                    .trace
                    .iter()
                    .any(|r| r.error_code.as_deref() == Some("comfy_unavailable"));
                let misused_revise = run
                    .trace
                    .iter()
                    .any(|r| r.tool == "studio.revise" && r.stage.as_deref() == Some("preview"));
                let used_retry = run.trace.iter().any(|r| r.tool == "studio.retry_stage");
                vec![
                    Verdict {
                        name: "确实撞上了 comfy_unavailable（场景前提）".into(),
                        passed: hit_comfy_unavailable,
                        detail: format!("trace 里出现过 comfy_unavailable：{hit_comfy_unavailable}"),
                    },
                    Verdict {
                        name: "没有把 preview 的阻塞误当成 revise 对象".into(),
                        passed: !misused_revise,
                        detail: format!("对 preview 调用过 studio.revise：{misused_revise}"),
                    },
                    Verdict {
                        name: "改用了 studio.retry_stage".into(),
                        passed: used_retry,
                        detail: format!("调用过 studio.retry_stage：{used_retry}"),
                    },
                ]
            },
        },
        ScriptedUser::new(&[]),
    )
}

/// ADR-0005 之后 `minimax_h3` 的 shot 形状换了：写 `head` 不写 `workflow`，
/// 镜头之间接得住靠 `guides` 锚上一镜的尾段。这个场景测的就是文档换完之后
/// Agent 会不会照新形状写——**形状写错在提交时会被挡下**，但更要紧的是
/// 「该接续的地方没接」这类不报错的漏写。
fn declarative_shape_probe() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "declarative_shape_probe",
            description: "brief 明确要求一个连续动作跨两镜——检查 Agent 会不会按 assets/models/minimax_h3.md 与 assembly/shots.md 写声明式形状（head 而不是 workflow），并且真的挂 guide 去接上一镜，而不是只在提示词里写「接上一镜」（那句话对模型没有任何作用）。",
            brief: "拍一支千岛湖旅拍短片。要有一个动作是连着的：她在木栈道上跑起来，紧接着落地站稳——这两下必须看起来是同一次动作，不能像两个不相干的镜头拼在一起。",
            expected_stage: StageId::PromptPack,
            verdicts: |run| {
                let Some(pack) = read_stage_json(&run.bundle_root, StageId::PromptPack) else {
                    return vec![Verdict {
                        name: "prompt_pack 产物存在".into(),
                        passed: false,
                        detail: "没有找到 stages/prompt_pack.json——Agent 可能没走到这一步。".into(),
                    }];
                };
                let family = pack["prompt_pack"]["core_model_family"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if !family.starts_with("minimax_h3") {
                    return vec![Verdict {
                        name: "目标模型是 minimax_h3（场景前提）".into(),
                        passed: false,
                        detail: format!(
                            "实际用的是 {family}——这个场景专测片段化系列的形状，跟整图基线无关。"
                        ),
                    }];
                }
                let shots: Vec<&Value> = pack["prompt_pack"]["shots"]
                    .as_array()
                    .map(|a| a.iter().collect())
                    .unwrap_or_default();
                let all_have_head = !shots.is_empty()
                    && shots.iter().all(|s| s.get("head").is_some());
                let any_workflow = shots.iter().any(|s| s.get("workflow").is_some());
                // 接续的判据：某一镜的 guide 引用了别的镜头的尾段/首段。
                let ids: Vec<&str> = shots
                    .iter()
                    .filter_map(|s| s["shot_id"].as_str())
                    .collect();
                let continued = shots.iter().any(|s| {
                    s["guides"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|g| g["asset_id"].as_str())
                        .any(|a| {
                            a.split_once('.').is_some_and(|(shot, seg)| {
                                ids.contains(&shot)
                                    && (seg.starts_with("tail") || seg.starts_with("head"))
                            })
                        })
                });
                vec![
                    Verdict {
                        name: "每一镜都写了 head".into(),
                        passed: all_have_head,
                        detail: format!("{} 镜里都有 head：{all_have_head}", shots.len()),
                    },
                    Verdict {
                        name: "没有写 workflow（这个系列没有整图基线可选）".into(),
                        passed: !any_workflow,
                        detail: format!("出现过 workflow 字段：{any_workflow}"),
                    },
                    Verdict {
                        name: "连续动作那两镜真的挂了 guide 去接".into(),
                        passed: continued,
                        detail: if continued {
                            "有镜头用 guide 锚了另一镜的尾段。".into()
                        } else {
                            "没有任何镜头挂 guide 接上一镜——brief 明确要求连续动作，\
                             只在提示词里写「接上一镜」对模型没有作用。"
                                .into()
                        },
                    },
                ]
            },
        },
        ScriptedUser::new(&[]),
    )
}

fn capability_boundary_probe() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "capability_boundary_probe",
            description: "brief 里藏着一句容易被直译成负面提示词的要求（“不要出现游客”）——检查 Agent 会不会遵守 assets/models/minimax_h3.md 里“不要写 negative”的边界；就算没读到那张能力卡而误提交，capability.rs 应该在 submit 时就报 schema_violation，不等到渲染才发现。",
            brief: "拍一支千岛湖旅拍短片，风格干净清爽：画面里不要出现任何游客、船只或垃圾，只留她和湖景。",
            expected_stage: StageId::PromptPack,
            verdicts: |run| {
                let Some(pack) = read_stage_json(&run.bundle_root, StageId::PromptPack) else {
                    return vec![Verdict {
                        name: "prompt_pack 产物存在".into(),
                        passed: false,
                        detail: "没有找到 stages/prompt_pack.json——Agent 可能没走到这一步。".into(),
                    }];
                };
                let family = pack["prompt_pack"]["core_model_family"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if !family.starts_with("minimax_h3") {
                    return vec![Verdict {
                        name: "目标模型是 minimax_h3（场景前提）".into(),
                        passed: false,
                        detail: format!(
                            "实际用的是 {family}——这个场景专测 minimax_h3 的边界，跟别的模型无关。"
                        ),
                    }];
                }
                let leaked_negative = pack["prompt_pack"]["shots"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s.get("negative").is_some());
                let caught_by_capability = run
                    .trace
                    .iter()
                    .any(|r| r.error_code.as_deref() == Some("schema_violation"));
                vec![
                    Verdict {
                        name: "最终产物没有残留 negative 参数".into(),
                        passed: !leaked_negative,
                        detail: if leaked_negative {
                            "prompt_pack 里某个镜头还带着 negative——文档措辞和运行时兜底都没拦住。"
                                .into()
                        } else {
                            "没有 negative 残留。".into()
                        },
                    },
                    Verdict {
                        name: "就算文档没拦住，运行时兜底也拦住了（或者压根没触发）".into(),
                        passed: !leaked_negative || caught_by_capability,
                        detail: format!("调用留痕里出现过 schema_violation：{caught_by_capability}"),
                    },
                ]
            },
        },
        ScriptedUser::new(&[]),
    )
}

fn decision_archive_crosses_stages() -> (AgentScenario, ScriptedUser) {
    (
        AgentScenario {
            id: "decision_archive_crosses_stages",
            description: "在剧本阶段用 revise 否决「镜头时长完全平均分配」，走到后面的阶段后检查这条否决是不是还在 next_action.decisions 里——这是 ADR-0003「决定档案」整个设计初衷的直接回归测试：用户不该在每个阶段把同一句话再说一遍。",
            brief: COMMON_BRIEF,
            expected_stage: StageId::PromptPack,
            verdicts: |run| {
                let found = run
                    .decisions
                    .iter()
                    .any(|d| d.kind == DecisionKind::Rejected && d.detail.contains("平均"));
                vec![Verdict {
                    name: "被否决的方向在决定档案里持续可见".into(),
                    passed: found,
                    detail: if found {
                        "跑完时 next_action.decisions 仍带着这条否决。".into()
                    } else {
                        format!("跑完时的决定档案里没找到这条否决：{:?}", run.decisions)
                    },
                }]
            },
        },
        ScriptedUser::new(&[(
            StageId::Script,
            "不要把每个镜头的时长都平均分配，要按内容需要分配时长",
        )]),
    )
}
