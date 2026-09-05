//! 质量闸。
//!
//! schema 管的是**形状**：字段在不在、类型对不对、枚举合不合法。
//! 形状对了内容仍然可以是空的——`three_facts: ["好看", "很美", "有感觉"]`
//! 完全合规，也完全没用。这个模块管的是内容够不够硬。
//!
//! 三条设计约束：
//!
//! 1. **零 I/O、纯函数**。规则只吃一个阶段的产物，返回一串 [`Finding`]。
//!    同一份产物，在提交时、在 CI 里、在报告里跑，结论必须一样。
//! 2. **只做机械可判的规则**。「钩子够不够强」不在这里——那是人的判断，
//!    写成规则只会制造假阳性，然后大家学会绕过它。这里只放
//!    「字符串相等」「词表命中」「计数」这类没有解释空间的判据。
//! 3. **两档严重度**。[`Severity::Blocking`] 直接挡提交，
//!    [`Severity::Advisory`] 只出现在报告里。挡提交的规则必须
//!    在 doctrine 里有对应的写法说明——挡人却不告诉人怎么写是耍流氓。

use crate::error::{Result, StudioError, Violation};
use crate::lexicon;
use crate::stage::StageId;
use crate::Outputs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 挡住提交。
    Blocking,
    /// 只报告，不挡。
    Advisory,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Blocking => "blocking",
            Severity::Advisory => "advisory",
        }
    }
}

/// 一条质量问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub stage: StageId,
    /// 规则标识，稳定不变，报告按它归类。
    pub rule: String,
    /// 产物内的路径，与 schema 违规同一种写法。
    pub path: String,
    pub message: String,
    pub severity: Severity,
}

impl Finding {
    fn blocking(
        stage: StageId,
        rule: &str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Finding {
            stage,
            rule: rule.to_string(),
            path: path.into(),
            message: message.into(),
            severity: Severity::Blocking,
        }
    }

    fn advisory(
        stage: StageId,
        rule: &str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Finding {
            stage,
            rule: rule.to_string(),
            path: path.into(),
            message: message.into(),
            severity: Severity::Advisory,
        }
    }
}

/// 规则清单。随包文档里的质量闸表由它生成，别在文档里另写一份。
pub const RULES: [(&str, Severity, &str); 7] = [
    (
        "banned_word",
        Severity::Blocking,
        "面向模型的文本里出现 Tier 1 禁用词（cinematic / 电影感 / 唯美这类）。\
         这些词对模型是噪声，对人是废话，占着字数不给信息",
    ),
    (
        "banned_word_upstream",
        Severity::Advisory,
        "创意/剧本阶段的文本里出现禁用词。这一层还没面向模型，不挡提交，\
         但它会一路抄进分镜",
    ),
    (
        "thin_fact",
        Severity::Blocking,
        "物理事实太短，写不下一个可拍的动作。少于 6 个字的多半是形容词",
    ),
    (
        "identity_lock_missing",
        Severity::Blocking,
        "镜头提示词里没有逐字带上身份锁。一致性靠同一个字符串被复制，\
         不靠每次重新描述",
    ),
    (
        "identity_lock_drift",
        Severity::Blocking,
        "身份锁在各阶段之间不是同一个字符串。近义改写等于换了一个人",
    ),
    (
        "thin_prompt",
        Severity::Advisory,
        "镜头提示词过短。低于 40 字通常意味着分镜里写好的东西没编译进来",
    ),
    (
        "no_camera_command",
        Severity::Advisory,
        "提示词里没有出现该系列的运镜指令，分镜定的运镜没有传达给模型",
    ),
];

/// 检查一个阶段的产物。
pub fn check_stage(stage: StageId, outputs: &Outputs) -> Vec<Finding> {
    let mut out = Vec::new();
    banned_words(stage, outputs, &mut out);
    match stage {
        StageId::Storyboard => storyboard(outputs, &mut out),
        StageId::PromptPack => prompt_pack(outputs, &mut out),
        _ => {}
    }
    out
}

/// 挡不挡提交。只看 [`Severity::Blocking`]。
pub fn gate(stage: StageId, outputs: &Outputs) -> Result<()> {
    let findings = check_stage(stage, outputs);
    let blocking: Vec<Violation> = findings
        .iter()
        .filter(|f| f.severity == Severity::Blocking)
        .map(|f| Violation::new(f.path.clone(), format!("[{}] {}", f.rule, f.message)))
        .collect();
    if blocking.is_empty() {
        return Ok(());
    }
    Err(StudioError::QualityViolation {
        stage,
        findings: blocking,
    })
}

/// 跨阶段检查：身份锁在分镜、视觉资产、提示词包里必须是同一个字符串。
///
/// 三处各写一遍、指望它们意思一样，就是身份漂移的来源。
/// 这里比的是字节，不是意思。
pub fn check_across_stages(approved: &[(StageId, Outputs)]) -> Vec<Finding> {
    let find = |stage: StageId| approved.iter().find(|(s, _)| *s == stage).map(|(_, o)| o);
    let mut out = Vec::new();

    let locks: Vec<(StageId, String, &str)> = [
        (
            StageId::Storyboard,
            "storyboard.character_lock.identity_lock",
            find(StageId::Storyboard).and_then(|o| {
                o.get("storyboard")?
                    .get("character_lock")?
                    .get("identity_lock")?
                    .as_str()
                    .map(str::to_string)
            }),
        ),
        (
            StageId::VisualAssets,
            "asset_plan.consistency_lock.character",
            find(StageId::VisualAssets).and_then(|o| {
                o.get("asset_plan")?
                    .get("consistency_lock")?
                    .get("character")?
                    .as_str()
                    .map(str::to_string)
            }),
        ),
        (
            StageId::PromptPack,
            "prompt_pack.identity_lock.character",
            find(StageId::PromptPack).and_then(|o| {
                o.get("prompt_pack")?
                    .get("identity_lock")?
                    .get("character")?
                    .as_str()
                    .map(str::to_string)
            }),
        ),
    ]
    .into_iter()
    .filter_map(|(stage, path, lock)| lock.map(|l| (stage, l, path)))
    .collect();

    let Some((_, first, first_path)) = locks.first() else {
        return out;
    };
    for (stage, lock, path) in locks.iter().skip(1) {
        if lock != first {
            out.push(Finding::blocking(
                *stage,
                "identity_lock_drift",
                *path,
                format!(
                    "身份锁与 {first_path} 不是同一个字符串。\
                     这里写的是「{lock}」，那边写的是「{first}」。\
                     一致性靠逐字复制，不靠近义改写"
                ),
            ));
        }
    }
    out
}

/// 可量化的指标。报告用，不挡任何东西。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub value: String,
    /// 目标值。没有硬目标的写 None。
    pub target: Option<String>,
    pub met: Option<bool>,
}

/// 一个阶段的可量化指标。
pub fn metrics(stage: StageId, outputs: &Outputs) -> Vec<Metric> {
    let mut out = Vec::new();
    match stage {
        StageId::Storyboard => {
            let shots = shots_of(outputs, "storyboard");
            let counts: Vec<usize> = shots
                .iter()
                .map(|s| {
                    s.get("three_facts")
                        .and_then(|f| f.as_array())
                        .map_or(0, Vec::len)
                })
                .collect();
            let min = counts.iter().copied().min().unwrap_or(0);
            out.push(Metric {
                name: "每镜物理事实条数（最少）".to_string(),
                value: min.to_string(),
                target: Some("≥ 3".to_string()),
                met: Some(min >= 3),
            });
            let in_vocab = shots
                .iter()
                .filter(|s| {
                    s.get("camera_motion")
                        .and_then(|m| m.as_str())
                        .is_some_and(|m| lexicon::CAMERA_MOTIONS.contains(&m))
                })
                .count();
            out.push(ratio("运镜落在受控词表内", in_vocab, shots.len(), "100%"));
            let with_audio = shots
                .iter()
                .filter(|s| s.get("audio").is_some_and(|a| !a.is_null()))
                .count();
            out.push(ratio("每镜有声音锚点", with_audio, shots.len(), "100%"));
        }
        StageId::PromptPack => {
            let shots = shots_of(outputs, "prompt_pack");
            let lock = identity_lock(outputs);
            if let Some(lock) = &lock {
                let carried = shots
                    .iter()
                    .filter(|s| {
                        s.get("positive")
                            .and_then(|p| p.as_str())
                            .is_some_and(|p| p.contains(lock.as_str()))
                    })
                    .count();
                out.push(ratio("身份锁逐字出现", carried, shots.len(), "100%"));
            }
            let lens: Vec<usize> = shots
                .iter()
                .map(|s| {
                    s.get("positive")
                        .and_then(|p| p.as_str())
                        .map_or(0, |p| p.chars().count())
                })
                .collect();
            let avg = if lens.is_empty() {
                0
            } else {
                lens.iter().sum::<usize>() / lens.len()
            };
            out.push(Metric {
                name: "提示词平均字数".to_string(),
                value: avg.to_string(),
                target: None,
                met: None,
            });
        }
        _ => {}
    }
    let hits: usize = prose(outputs)
        .iter()
        .map(|(_, s)| lexicon::banned_tier1_hits(s).len())
        .sum();
    out.push(Metric {
        name: "禁用词命中数（Tier 1）".to_string(),
        value: hits.to_string(),
        target: Some("0".to_string()),
        met: Some(hits == 0),
    });
    out
}

fn ratio(name: &str, hit: usize, total: usize, target: &str) -> Metric {
    Metric {
        name: name.to_string(),
        value: if total == 0 {
            "—".to_string()
        } else {
            format!("{hit}/{total}")
        },
        target: Some(target.to_string()),
        met: Some(total > 0 && hit == total),
    }
}

/// 面向模型的文本必须干净；上游阶段只提醒。
///
/// 分成两档不是妥协：`idea` 的 assumptions 里原样引用用户说的
/// 「要唯美一点」是合理的，挡掉它只会逼 Agent 去改写用户的话。
/// 到了分镜和提示词，这些词已经在往模型嘴里送，就得挡。
fn banned_words(stage: StageId, outputs: &Outputs, out: &mut Vec<Finding>) {
    let (rule, severity): (&str, Severity) = match stage {
        StageId::Storyboard | StageId::VisualAssets | StageId::PromptPack => {
            ("banned_word", Severity::Blocking)
        }
        _ => ("banned_word_upstream", Severity::Advisory),
    };
    for (path, text) in prose(outputs) {
        let hits = lexicon::banned_tier1_hits(&text);
        if hits.is_empty() {
            continue;
        }
        let msg = format!(
            "出现禁用词 {}。换成拍得出来的具体描述：光是从哪儿来的、\
             人在做什么动作、画面里有什么东西",
            hits.join("、")
        );
        out.push(match severity {
            Severity::Blocking => Finding::blocking(stage, rule, path, msg),
            Severity::Advisory => Finding::advisory(stage, rule, path, msg),
        });
    }
}

fn storyboard(outputs: &Outputs, out: &mut Vec<Finding>) {
    for (i, shot) in shots_of(outputs, "storyboard").iter().enumerate() {
        let Some(facts) = shot.get("three_facts").and_then(|f| f.as_array()) else {
            continue;
        };
        for (j, fact) in facts.iter().enumerate() {
            let Some(s) = fact.as_str() else { continue };
            if s.chars().count() < 6 {
                out.push(Finding::blocking(
                    StageId::Storyboard,
                    "thin_fact",
                    format!("storyboard.shots[{i}].three_facts[{j}]"),
                    format!(
                        "「{s}」太短，说不出一个可拍的事实。\
                         物理事实要么是一个动作，要么是一个能看见/听见的东西"
                    ),
                ));
            }
        }
    }
}

fn prompt_pack(outputs: &Outputs, out: &mut Vec<Finding>) {
    let lock = identity_lock(outputs);
    for (i, shot) in shots_of(outputs, "prompt_pack").iter().enumerate() {
        let positive = shot.get("positive").and_then(|p| p.as_str()).unwrap_or("");
        let path = format!("prompt_pack.shots[{i}].positive");

        if let Some(lock) = &lock {
            if !lock.is_empty() && !positive.contains(lock.as_str()) {
                out.push(Finding::blocking(
                    StageId::PromptPack,
                    "identity_lock_missing",
                    &path,
                    format!(
                        "没有逐字带上身份锁「{lock}」。\
                         把 prompt_pack.identity_lock.character 复制过来，不要复述"
                    ),
                ));
            }
        }

        if positive.chars().count() < 40 {
            out.push(Finding::advisory(
                StageId::PromptPack,
                "thin_prompt",
                &path,
                format!(
                    "只有 {} 个字。分镜里的三条物理事实、光源、景别、\
                     前中后景是不是没编译进来？",
                    positive.chars().count()
                ),
            ));
        }

        let workflow = shot.get("workflow").and_then(|w| w.as_str()).unwrap_or("");
        if workflow.starts_with("minimax_h3/") {
            let has_command = lexicon::MINIMAX_CAMERA_COMMANDS
                .iter()
                .any(|(_, cmd)| positive.contains(cmd));
            if !has_command {
                out.push(Finding::advisory(
                    StageId::PromptPack,
                    "no_camera_command",
                    &path,
                    "这个系列吃方括号运镜指令（例如 [Push in]），\
                     提示词里一条都没有——分镜定的运镜等于没传达"
                        .to_string(),
                ));
            }
        }
    }
}

fn identity_lock(outputs: &Outputs) -> Option<String> {
    outputs
        .get("prompt_pack")?
        .get("identity_lock")?
        .get("character")?
        .as_str()
        .map(str::to_string)
}

fn shots_of(outputs: &Outputs, key: &str) -> Vec<Value> {
    outputs
        .get(key)
        .and_then(|v| v.get("shots"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 产物里所有自由文本，带路径。
pub fn prose(outputs: &Outputs) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Value, String)> = outputs.iter().map(|(k, v)| (v, k.clone())).collect();
    while let Some((node, path)) = stack.pop() {
        match node {
            Value::String(s) => out.push((path, s.clone())),
            Value::Array(a) => {
                for (i, item) in a.iter().enumerate() {
                    stack.push((item, format!("{path}[{i}]")));
                }
            }
            Value::Object(m) => {
                for (k, item) in m {
                    // `_` 开头是控制面回填的字段，不是 Agent 写的散文。
                    if k.starts_with('_') {
                        continue;
                    }
                    stack.push((item, format!("{path}.{k}")));
                }
            }
            _ => {}
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use serde_json::json;

    /// 黄金样例是随包分发的范文，必须先过自己的闸。
    ///
    /// 这里连 advisory 都不许有：范文只要有一条提醒，抄它的人就会
    /// 原样抄出同一条提醒，然后学会「这条可以不管」。
    #[test]
    fn every_fixture_passes_its_own_gate() {
        for stage in StageId::all() {
            let outputs = fixtures::outputs(stage);
            let findings = check_stage(stage, &outputs);
            assert!(
                findings.is_empty(),
                "{stage} 的黄金样例没过质量闸：{findings:#?}"
            );
        }
    }

    #[test]
    fn fixtures_agree_on_one_identity_lock() {
        let approved: Vec<_> = StageId::all().map(|s| (s, fixtures::outputs(s))).collect();
        assert!(
            check_across_stages(&approved).is_empty(),
            "样例的身份锁跨阶段不一致：{:#?}",
            check_across_stages(&approved)
        );
    }

    #[test]
    fn a_banned_word_in_a_prompt_blocks_but_in_a_brief_only_warns() {
        let mut pack = fixtures::outputs(StageId::PromptPack);
        pack["prompt_pack"]["shots"][0]["positive"] =
            json!(format!("{} 电影感的湖面", fixtures::IDENTITY_LOCK));
        let f = check_stage(StageId::PromptPack, &pack);
        assert!(f
            .iter()
            .any(|f| f.rule == "banned_word" && f.severity == Severity::Blocking));
        assert!(gate(StageId::PromptPack, &pack).is_err());

        let mut idea = fixtures::outputs(StageId::Idea);
        idea["brief"]["theme"] = json!("用户原话：想要唯美一点");
        let f = check_stage(StageId::Idea, &idea);
        assert!(f
            .iter()
            .any(|f| f.rule == "banned_word_upstream" && f.severity == Severity::Advisory));
        assert!(gate(StageId::Idea, &idea).is_ok(), "上游阶段不挡提交");
    }

    #[test]
    fn a_paraphrased_identity_lock_is_caught() {
        let mut pack = fixtures::outputs(StageId::PromptPack);
        pack["prompt_pack"]["shots"][0]["positive"] =
            json!("同一位20岁东亚女性，长黑发，白色连衣裙，站在船头，上午冷白自然光，竖构图");
        let findings = check_stage(StageId::PromptPack, &pack);
        assert!(
            findings.iter().any(|f| f.rule == "identity_lock_missing"),
            "「同一位…」这种复述必须被抓出来：{findings:#?}"
        );
    }

    #[test]
    fn a_lock_that_drifts_between_stages_is_caught() {
        let mut pack = fixtures::outputs(StageId::PromptPack);
        pack["prompt_pack"]["identity_lock"]["character"] = json!("20岁女生，黑长发，白裙子");
        let approved = vec![
            (StageId::Storyboard, fixtures::outputs(StageId::Storyboard)),
            (StageId::PromptPack, pack),
        ];
        let findings = check_across_stages(&approved);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "identity_lock_drift");
    }

    #[test]
    fn a_three_character_fact_is_not_a_fact() {
        let mut sb = fixtures::outputs(StageId::Storyboard);
        sb["storyboard"]["shots"][0]["three_facts"][0] = json!("很好看");
        let findings = check_stage(StageId::Storyboard, &sb);
        assert!(findings.iter().any(|f| f.rule == "thin_fact"));
    }

    /// 挡提交的规则必须写进 doctrine，否则就是「挡了却不说怎么改」。
    #[test]
    fn every_rule_is_documented() {
        for (rule, _, desc) in RULES {
            assert!(!desc.is_empty(), "{rule} 没有说明");
        }
        // 代码里用到的规则名必须都在清单里。
        let mut used = vec![
            "banned_word",
            "banned_word_upstream",
            "thin_fact",
            "identity_lock_missing",
            "identity_lock_drift",
            "thin_prompt",
            "no_camera_command",
        ];
        used.sort();
        let mut listed: Vec<&str> = RULES.iter().map(|(r, _, _)| *r).collect();
        listed.sort();
        assert_eq!(used, listed);
    }

    #[test]
    fn metrics_report_the_fixture_as_meeting_its_targets() {
        for stage in [StageId::Storyboard, StageId::PromptPack] {
            for m in metrics(stage, &fixtures::outputs(stage)) {
                if let Some(met) = m.met {
                    assert!(met, "{stage} 的「{}」没达标：{}", m.name, m.value);
                }
            }
        }
    }
}
