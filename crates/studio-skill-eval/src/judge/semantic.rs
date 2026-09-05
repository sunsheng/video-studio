//! LLM 语义裁判：结构化规则覆盖不到的主观质量，交给一个评审 LLM 判。
//!
//! **不能只看 SKILL.md**（ADR-0004「judge/semantic.rs 纳入 doctrine
//! 层」）：提示词架构重构之后大半指导性内容搬进了 `.agents/doctrine/`
//! （运镜语法、故事结构、质量清单等）和 `.agents/models/*.md`（模型
//! 能力卡），SKILL.md 本身只剩"什么时候用、调用形状"。只评审 SKILL.md
//! 会漏评大半 Agent 实际应该参考的文档面，评出虚高的分。
//!
//! 评审 LLM 与被测 driver 的 LLM 允许不是同一个，避免"自己评自己"的
//! 偏置——调用方自己决定传哪个 `model`。

use super::Verdict;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use studio_core::StageId;

/// 按阶段维护的"相关 doctrine 文件"映射表——跟真实 Agent 该"按需加载"
/// 的那几份对应，不是无条件塞全部 doctrine（那样测的就不是"文档指对了
/// 地方没有"，而是"塞了全部上下文之后模型能不能自己挑出重点"，是另一
/// 件事）。
fn doctrine_for_stage(stage: StageId) -> &'static [&'static str] {
    use StageId::*;
    match stage {
        Idea => &["story/concepts.md"],
        Script => &["story/structure.md", "story/voice.md", "story/hook.md"],
        Storyboard => &[
            "camera/grammar.md",
            "camera/blocking.md",
            "camera/lighting.md",
            "exemplars/storyboard.md",
        ],
        VisualAssets => &["consistency/bible.md", "consistency/character-sheet.md"],
        PromptPack => &[
            "quality/checklist.md",
            "quality/banned.md",
            "exemplars/prompt_pack.md",
        ],
        Selection | Preview | Render | Post | Review => &[],
    }
}

pub struct SemanticJudge {
    base_url: String,
    api_key: String,
    model: String,
}

impl SemanticJudge {
    /// 从 `OPENAI_API_KEY`/`OPENAI_BASE_URL` 环境变量构造，跟
    /// [`crate::driver::direct_llm::DirectLlmDriver`] 用的是同一套约定。
    pub fn from_env(model: impl Into<String>) -> Result<SemanticJudge, String> {
        Ok(SemanticJudge {
            base_url: std::env::var("OPENAI_BASE_URL")
                .map_err(|_| "没有配置 OPENAI_BASE_URL".to_string())?,
            api_key: std::env::var("OPENAI_API_KEY")
                .map_err(|_| "没有配置 OPENAI_API_KEY".to_string())?,
            model: model.into(),
        })
    }

    /// 评审某阶段的产物。`skill_duties` 是该阶段 SKILL.md 里"职责"条款
    /// 的原文，`output` 是最终产物 JSON，`doctrine_read` 是 rollout 观测
    /// 到的、Agent 实际读过的方法层文件路径——用来在判定里区分"文档没
    /// 被打开"和"文档写得不好"，这两者需要的修复完全不同。
    pub fn review(
        &self,
        stage: StageId,
        skill_duties: &str,
        output: &Value,
        doctrine_read: &[String],
    ) -> Result<Vec<Verdict>, String> {
        let relevant = doctrine_for_stage(stage);
        let doctrine_text = load_doctrine(relevant)?;
        let prompt = build_prompt(stage, skill_duties, &doctrine_text, output);
        let raw = self.complete(&prompt)?;
        let parsed =
            extract_json(&raw).ok_or_else(|| format!("评审 LLM 没有给出可解析的 JSON：{raw}"))?;
        Ok(to_verdicts(&parsed, relevant, doctrine_read))
    }

    fn complete(&self, prompt: &str) -> Result<String, String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "你是一个严格但公正的内容评审员，只输出 JSON，不要输出别的文字。"},
                {"role": "user", "content": prompt},
            ],
        });
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(120))
            .send_json(body)
            .map_err(|e| format!("调 {url} 失败：{e}"))?;
        let v: Value = resp
            .into_json()
            .map_err(|e| format!("响应不是合法 JSON：{e}"))?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| format!("响应里没有 choices[0].message.content：{v}"))
    }
}

fn load_doctrine(paths: &[&str]) -> Result<String, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/doctrine");
    let mut out = String::new();
    for p in paths {
        let text = std::fs::read_to_string(root.join(p))
            .map_err(|e| format!("读 assets/doctrine/{p} 失败：{e}"))?;
        out.push_str(&format!("\n\n### {p}\n{text}"));
    }
    Ok(out)
}

fn build_prompt(stage: StageId, duties: &str, doctrine: &str, output: &Value) -> String {
    format!(
        "阶段：{stage}\n\n【SKILL.md 职责条款】\n{duties}\n\n【相关方法层文档】{doctrine}\n\n\
         【最终产物】\n{output}\n\n逐条判断产物是否满足职责条款和方法层的质量要求，只输出这样的 \
         JSON，不要输出别的文字：\n{{\"items\":[{{\"criterion\":\"...\",\"met\":true/false,\"reason\":\"...\"}}]}}"
    )
}

/// 从评审 LLM 的回复里挖出 JSON——有的模型会在 JSON 前后加解释文字，
/// 找第一个 `{` 到最后一个 `}` 之间的子串再解析。
fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

fn to_verdicts(
    parsed: &Value,
    relevant_doctrine: &[&str],
    doctrine_read: &[String],
) -> Vec<Verdict> {
    let doctrine_was_read = relevant_doctrine
        .iter()
        .any(|p| doctrine_read.iter().any(|r| r.ends_with(*p)));
    parsed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let met = item["met"].as_bool().unwrap_or(false);
            let reason = item["reason"].as_str().unwrap_or("").to_string();
            let detail = if !met && !relevant_doctrine.is_empty() && !doctrine_was_read {
                format!(
                    "{reason}（且相关方法层文档一份都没有被读到——这是「文档没被打开」，\
                     不是「文档写得不好」）"
                )
            } else {
                reason
            };
            Verdict {
                name: item["criterion"]
                    .as_str()
                    .unwrap_or("未命名条款")
                    .to_string(),
                passed: met,
                detail,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_ignores_surrounding_prose() {
        let text = "这是我的评审结果：\n{\"items\":[{\"criterion\":\"a\",\"met\":true,\"reason\":\"ok\"}]}\n谢谢查阅。";
        let v = extract_json(text).unwrap();
        assert_eq!(v["items"][0]["criterion"], "a");
    }

    #[test]
    fn extract_json_returns_none_for_non_json_text() {
        assert!(extract_json("我不知道怎么评审这个").is_none());
    }

    #[test]
    fn failing_criterion_flags_unread_doctrine_separately_from_bad_writing() {
        let parsed = json!({"items":[{"criterion":"运镜可执行","met":false,"reason":"太模糊"}]});
        let verdicts = to_verdicts(&parsed, &["camera/grammar.md"], &[]);
        assert!(!verdicts[0].passed);
        assert!(verdicts[0].detail.contains("没有被读到"));
    }

    #[test]
    fn failing_criterion_stays_plain_when_doctrine_was_actually_read() {
        let parsed = json!({"items":[{"criterion":"运镜可执行","met":false,"reason":"太模糊"}]});
        let verdicts = to_verdicts(
            &parsed,
            &["camera/grammar.md"],
            &["doctrine/camera/grammar.md".to_string()],
        );
        assert!(!verdicts[0].detail.contains("没有被读到"));
        assert_eq!(verdicts[0].detail, "太模糊");
    }

    #[test]
    fn doctrine_mapping_matches_real_files_on_disk() {
        for stage in StageId::all() {
            for p in doctrine_for_stage(stage) {
                let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/doctrine");
                assert!(
                    root.join(p).is_file(),
                    "{stage} 映射到的 doctrine 文件不存在：{p}"
                );
            }
        }
    }
}
