//! 内容验收 rubric。
//!
//! 验收阶段原本只有机械指标：时长、画幅、镜头数、有没有音轨——
//! 全部来自 ffprobe 实测。这些都通过，只证明**片子是完整的**，
//! 不证明它好看。从头到尾没有一处在问「这东西好不好看」。
//!
//! 这里补的是另一半。两半的分工是清楚的：
//!
//! - **技术验收**：控制面做，基于实测元数据，通过与否决定 `review.passed`。
//! - **内容验收**：Agent 做，按下面这张固定的 rubric 逐条自评。
//!   **它不决定 `passed`**——片子已经出来了，内容评价改变不了它是否完整。
//!   它决定的是这次交付有没有留下一份「照自己定的标准打了几分」的记录。
//!
//! 为什么要求时间点：没有时间点的自评是「我觉得还行」。
//! 写了 `at_seconds: 0.6` 就得真去看那一帧，说不出具体画面就写不出证据。

use crate::error::{Result, StudioError, Violation};
use crate::stage::StageId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 固定的评价维度。**不可增删**——每次交付评的是同一套东西，
/// 才能横向比较；让 Agent 自选维度等于让它挑自己做得好的那几条。
pub const CRITERIA: [(&str, &str); 5] = [
    (
        "hook",
        "前三秒是否成立：观众在第几秒已经知道这是什么、值不值得看下去",
    ),
    (
        "information_density",
        "每一镜是否都给了新信息。删掉哪一镜观众也不会察觉，就是这一镜没有信息",
    ),
    (
        "pacing",
        "节奏是否跟内容走：动作复杂的镜头有没有被切太短，空镜有没有拖",
    ),
    (
        "consistency",
        "同一个人、同一个地方跨镜有没有变。这是 AI 视频最先露馅的地方",
    ),
    (
        "brief_metrics",
        "brief 里那些**内容型**的 success_metrics 逐条兑现了没有",
    ),
];

/// 一条自评的结论。
pub const VERDICTS: [&str; 3] = ["met", "partially_met", "not_met"];

/// 一条自评。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubricItem {
    pub criterion: String,
    pub verdict: String,
    /// 可指认的时间点。没有时间点的自评是「我觉得还行」。
    pub at_seconds: f64,
    /// 在那个时间点上看见/听见了什么，以及它为什么支持这个结论。
    pub evidence: String,
}

/// 一份完整的内容自评。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfReview {
    pub items: Vec<RubricItem>,
    pub summary: String,
}

impl SelfReview {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 证据的最短长度。比这更短的写不下一个具体画面。
const MIN_EVIDENCE_CHARS: usize = 12;

/// 校验一份自评。`duration_seconds` 是成片实测时长，用来卡时间点。
///
/// 零 I/O：时长由调用方从技术验收结果里取出来传进来。
pub fn validate(review: &SelfReview, duration_seconds: f64) -> Result<()> {
    let mut v = Vec::new();

    for (i, item) in review.items.iter().enumerate() {
        let at = |field: &str| format!("content_review.items[{i}].{field}");

        if !CRITERIA.iter().any(|(c, _)| *c == item.criterion) {
            v.push(Violation::new(
                at("criterion"),
                format!(
                    "「{}」不是评价维度。维度是固定的：{}",
                    item.criterion,
                    CRITERIA
                        .iter()
                        .map(|(c, _)| *c)
                        .collect::<Vec<_>>()
                        .join("、")
                ),
            ));
        }
        if !VERDICTS.contains(&item.verdict.as_str()) {
            v.push(Violation::new(
                at("verdict"),
                format!("结论只能是 {}", VERDICTS.join(" / ")),
            ));
        }
        if item.at_seconds < 0.0 || item.at_seconds > duration_seconds {
            v.push(Violation::new(
                at("at_seconds"),
                format!(
                    "{} 不在成片里（实测总时长 {duration_seconds:.2} 秒）。\
                     时间点要指向片子里真实存在的一帧",
                    item.at_seconds
                ),
            ));
        }
        if item.evidence.chars().count() < MIN_EVIDENCE_CHARS {
            v.push(Violation::new(
                at("evidence"),
                format!(
                    "证据太短（{} 字）。写在那个时间点上**看见/听见**了什么，\
                     不是写「还不错」",
                    item.evidence.chars().count()
                ),
            ));
        }
    }

    for (criterion, desc) in CRITERIA {
        let n = review
            .items
            .iter()
            .filter(|i| i.criterion == criterion)
            .count();
        if n == 0 {
            v.push(Violation::new(
                "content_review.items",
                format!("缺维度 {criterion}：{desc}"),
            ));
        } else if n > 1 {
            v.push(Violation::new(
                "content_review.items",
                format!("维度 {criterion} 写了 {n} 条，每个维度只给一条结论"),
            ));
        }
    }

    if review.summary.chars().count() < MIN_EVIDENCE_CHARS {
        v.push(Violation::new(
            "content_review.summary",
            "总结太短。一句话说清这片子最强的一点和最弱的一点",
        ));
    }

    if v.is_empty() {
        Ok(())
    } else {
        Err(StudioError::SchemaViolation {
            stage: StageId::Review,
            violations: v,
        })
    }
}

/// 从一份自评里数出各档结论的条数，给信封和报告用。
pub fn tally(review: &SelfReview) -> (usize, usize, usize) {
    let count = |w: &str| review.items.iter().filter(|i| i.verdict == w).count();
    (count("met"), count("partially_met"), count("not_met"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> SelfReview {
        SelfReview {
            items: CRITERIA
                .iter()
                .map(|(c, _)| RubricItem {
                    criterion: (*c).to_string(),
                    verdict: "met".to_string(),
                    at_seconds: 0.6,
                    evidence: "0.6 秒她转头笑出来，同一帧里湖面和群岛都在，地点和人一起给到"
                        .to_string(),
                })
                .collect(),
            summary: "钩子成立得很早，最弱的是第三镜的信息密度".to_string(),
        }
    }

    #[test]
    fn a_complete_self_review_passes() {
        assert!(validate(&good(), 10.0).is_ok());
    }

    #[test]
    fn missing_a_criterion_is_rejected() {
        let mut r = good();
        r.items.pop();
        let err = validate(&r, 10.0).unwrap_err();
        assert!(err.message().contains("缺维度"));
    }

    #[test]
    fn a_timecode_outside_the_film_is_rejected() {
        let mut r = good();
        r.items[0].at_seconds = 42.0;
        let err = validate(&r, 10.0).unwrap_err();
        assert!(err.message().contains("不在成片里"));
    }

    /// 「还不错」这类自评正是这一整套东西要挡的。
    #[test]
    fn a_vague_evidence_is_rejected() {
        let mut r = good();
        r.items[2].evidence = "还不错".to_string();
        let err = validate(&r, 10.0).unwrap_err();
        assert!(err.message().contains("证据太短"));
    }

    #[test]
    fn the_same_criterion_twice_is_rejected() {
        let mut r = good();
        let dup = r.items[0].clone();
        r.items.push(dup);
        let err = validate(&r, 10.0).unwrap_err();
        assert!(err.message().contains("只给一条结论"));
    }

    #[test]
    fn tally_counts_each_verdict() {
        let mut r = good();
        r.items[0].verdict = "not_met".to_string();
        r.items[1].verdict = "partially_met".to_string();
        assert_eq!(tally(&r), (3, 1, 1));
    }
}
