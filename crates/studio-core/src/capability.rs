//! 基线能力面：**这条基线到底吃哪些参数。**
//!
//! 这是为了消灭本项目最危险的一种失败：写了基线没有绑定的参数，
//! 它会被静默跳过——不报错、不留痕，只是让画面莫名其妙地不对。
//! 最常见的两例：
//!
//! - `minimax_h3` 没有 `negative` 绑定，而提示词阶段习惯性地写负向提示词；
//! - `ltx2_5` 按秒收时长（`duration_seconds`），而 schema 的必填项是
//!   `length_frames`，换到这条基线时长就完全不受控。
//!
//! 这里只有**判断**，没有 I/O：能力面从哪来（读基线文件、还是测试里手写）
//! 由上层决定，[`CapabilitySet`] 只负责拿它去对账。这样「写了会被丢弃」
//! 这条规则可以在没有 GPU、没有 ComfyUI 的机器上完整单测。

use crate::error::{Result, StudioError, Violation};
use crate::stage::StageId;
use crate::Outputs;

/// 「写了但基线不吃」要报错的参数名。
///
/// 逐镜头产物里还有 `shot_id`、`workflow`、`audio` 这些不进节点图的字段，
/// 它们不在这里，也就不受这套校验约束。
///
/// **`references` 有意不在这里**，尽管它也会被 `apply()` 跳过。区别在写它的
/// 意图：写 `negative` 是想控制渲染参数，被丢弃等于控制失效，必须当场报错；
/// 写 `references` 是声明「这一镜用到哪些资产」，即使当前进不了渲染请求，
/// 这个声明本身可审计，而且基线补上图片输入绑定之后会自动生效。
/// 所以它的规则是**允许提前写，但基线一旦支持就必须写**——后半句由下面
/// 「少写」那个方向自动覆盖，因为那个方向遍历的是基线实际绑定的键。
pub const INJECTABLE_PARAMS: [&str; 8] = [
    "positive",
    "negative",
    "width",
    "height",
    "length_frames",
    "duration_seconds",
    "fps",
    "seed",
];

/// 一条已验证基线的能力面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCapability {
    /// 形如 `minimax_h3/t2v`。
    pub name: String,
    /// 可注入参数，取自基线的 `_studio.bindings` 键集合。
    pub params: Vec<String>,
    /// 是否已在真机上核验过绑定。未核验的不许用来渲染。
    pub verified: bool,
    /// 未核验时的原因，用在错误消息里。
    pub unavailable_reason: Option<String>,
}

impl WorkflowCapability {
    pub fn accepts(&self, param: &str) -> bool {
        self.params.iter().any(|p| p == param)
    }
}

/// 当前这台机器上全部可用基线。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    workflows: Vec<WorkflowCapability>,
}

impl CapabilitySet {
    pub fn new(workflows: Vec<WorkflowCapability>) -> Self {
        Self { workflows }
    }

    pub fn get(&self, name: &str) -> Option<&WorkflowCapability> {
        self.workflows.iter().find(|w| w.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.workflows.is_empty()
    }

    /// 已核验、可以写进提示词包的基线名，排序后返回。
    ///
    /// 用来把 `studio.schema("prompt_pack")` 里 `workflow` 字段的取值收窄到
    /// 这台机器真正能跑的那些——未核验的基线不该等到渲染时才报错。
    pub fn verified_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .workflows
            .iter()
            .filter(|w| w.verified)
            .map(|w| w.name.clone())
            .collect();
        names.sort();
        names
    }

    /// 把 `prompt_pack` 的 schema 文档收窄到这台机器真正能跑的基线。
    ///
    /// Agent 提交前会先调 `studio.schema` —— 与其让它写完一整包提示词、
    /// 提交时才被告知「这条基线没核验」，不如在它看 schema 的那一刻就只给
    /// 能用的那几条。没有已核验基线时不动 schema：那是部署问题，
    /// 该在渲染时以 `model_contract_violation` 报出来，不是靠一个空 enum
    /// 把 Agent 卡在原地。
    pub fn narrow_schema(&self, doc: &mut serde_json::Value) {
        let names = self.verified_names();
        if names.is_empty() {
            return;
        }
        let Some(field) =
            doc.pointer_mut("/properties/prompt_pack/properties/shots/items/properties/workflow")
        else {
            return;
        };
        let Some(obj) = field.as_object_mut() else {
            return;
        };
        obj.insert("enum".into(), serde_json::json!(names));
        obj.insert(
            "description".into(),
            serde_json::json!(
                "使用的已验证 workflow 名。这台机器上可用的就是这几条，\
                 未核验的基线不在其中。每条吃的参数不同——写了它不吃的参数\
                 会被静默丢弃，提交时会被挡下；写之前先看这个系列的能力卡"
            ),
        );
    }

    /// 对提示词包做双向对账。
    ///
    /// 两个方向都要查，因为两种错法的后果一样严重：
    /// **多写**的参数会被静默丢弃，**少写**的参数会让基线用自己的默认值。
    pub fn check_prompt_pack(&self, outputs: &Outputs) -> Result<()> {
        let mut violations = Vec::new();
        let Some(shots) = outputs
            .get("prompt_pack")
            .and_then(|p| p.get("shots"))
            .and_then(|s| s.as_array())
        else {
            // 形状本身不对，交给 schema 校验去报——这里不重复报一遍。
            return Ok(());
        };

        for (i, shot) in shots.iter().enumerate() {
            let at = |field: &str| format!("prompt_pack.shots[{i}].{field}");
            let Some(name) = shot.get("workflow").and_then(|w| w.as_str()) else {
                continue; // 同上，缺 workflow 是 schema 的事。
            };

            let Some(cap) = self.get(name) else {
                let mut available = self.verified_names();
                available.sort();
                violations.push(Violation::new(
                    at("workflow"),
                    format!(
                        "没有名为 {name} 的基线。可用的是：{}",
                        if available.is_empty() {
                            "（这台机器上一条都没有）".to_string()
                        } else {
                            available.join("、")
                        }
                    ),
                ));
                continue;
            };

            if !cap.verified {
                violations.push(Violation::new(
                    at("workflow"),
                    format!(
                        "基线 {name} 尚未真机核验（{}），不能用来渲染——\
                         绑错节点会静默产出错的画面，比直接报错难查得多。\
                         换一条已核验的基线：{}",
                        cap.unavailable_reason.as_deref().unwrap_or("原因未记录"),
                        self.verified_names().join("、")
                    ),
                ));
                continue;
            }

            let Some(obj) = shot.as_object() else {
                continue;
            };

            // 方向一：写了这条基线不吃的参数——写了会被静默丢弃。
            for param in INJECTABLE_PARAMS {
                if !obj.contains_key(param) || cap.accepts(param) {
                    continue;
                }
                violations.push(Violation::new(at(param), dropped_hint(param, name)));
            }

            // 方向二：这条基线要的参数没写——它会用自己的默认值。
            for param in &cap.params {
                if obj.contains_key(param.as_str()) {
                    continue;
                }
                violations.push(Violation::new(
                    at(param),
                    format!(
                        "基线 {name} 接受 {param}，但这一镜没写。\
                         不写就用基线自带的默认值，结果不受你控制"
                    ),
                ));
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(StudioError::SchemaViolation {
                stage: StageId::PromptPack,
                violations,
            })
        }
    }
}

/// 为「写了会被丢弃」的参数给出对症的下一步，而不是一句笼统的「不支持」。
fn dropped_hint(param: &str, workflow: &str) -> String {
    let head = format!("基线 {workflow} 没有绑定 {param}，写了会被静默丢弃");
    match param {
        "negative" => format!(
            "{head}。把约束改写成正向提示词里的完整句子——\
             「一镜到底，不切场景，画面中不出现任何文字、标志或水印」\
             比一串负向标签可靠"
        ),
        "length_frames" => format!("{head}。这条基线按秒收时长，改用 duration_seconds"),
        "duration_seconds" => format!("{head}。这条基线按帧数收时长，改用 length_frames 配 fps"),
        _ => format!("{head}。这个系列吃什么参数，看它的能力卡"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set() -> CapabilitySet {
        CapabilitySet::new(vec![
            WorkflowCapability {
                name: "minimax_h3/t2v".into(),
                params: [
                    "positive",
                    "width",
                    "height",
                    "length_frames",
                    "fps",
                    "seed",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                verified: true,
                unavailable_reason: None,
            },
            WorkflowCapability {
                name: "ltx2_5/t2v".into(),
                params: [
                    "positive",
                    "negative",
                    "width",
                    "height",
                    "duration_seconds",
                    "fps",
                    "seed",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                verified: true,
                unavailable_reason: None,
            },
            WorkflowCapability {
                name: "wan2_2/i2v".into(),
                params: vec!["seed".to_string()],
                verified: false,
                unavailable_reason: Some("正负提示词与尺寸走的连线尚未确认".into()),
            },
        ])
    }

    fn pack(shot: serde_json::Value) -> Outputs {
        let mut o = Outputs::new();
        o.insert("prompt_pack".into(), json!({ "shots": [shot] }));
        o
    }

    fn good_minimax() -> serde_json::Value {
        json!({
            "shot_id": "sh01", "workflow": "minimax_h3/t2v",
            "positive": "船头切开湖面", "width": 1080, "height": 1920,
            "length_frames": 42, "fps": 30, "seed": 101001,
            "audio": "湖水拍打船身"
        })
    }

    #[test]
    fn a_pack_matching_the_baseline_passes() {
        set().check_prompt_pack(&pack(good_minimax())).unwrap();
    }

    /// 这是最要紧的一条：negative 在这条基线上会被静默丢弃。
    #[test]
    fn writing_negative_on_a_baseline_without_it_is_rejected() {
        let mut shot = good_minimax();
        shot["negative"] = json!("文字, 水印");
        let e = set().check_prompt_pack(&pack(shot)).unwrap_err();
        assert_eq!(e.code(), "schema_violation");
        let msg = e.message();
        assert!(msg.contains("prompt_pack.shots[0].negative"), "{msg}");
        assert!(msg.contains("静默丢弃"), "{msg}");
        assert!(msg.contains("正向"), "要给出替代写法：{msg}");
    }

    /// LTX 按秒收时长，写 length_frames 会被丢弃、时长失控。
    #[test]
    fn the_wrong_duration_parameter_is_caught_both_ways() {
        let shot = json!({
            "shot_id": "sh01", "workflow": "ltx2_5/t2v",
            "positive": "p", "negative": "n", "width": 1088, "height": 1920,
            "length_frames": 42, "fps": 30, "seed": 1
        });
        let e = set().check_prompt_pack(&pack(shot)).unwrap_err();
        let msg = e.message();
        // 多写了 length_frames
        assert!(msg.contains("length_frames"), "{msg}");
        assert!(msg.contains("按秒收时长"), "{msg}");
        // 少写了 duration_seconds
        assert!(msg.contains("duration_seconds"), "{msg}");
    }

    #[test]
    fn a_missing_parameter_is_reported_with_the_default_warning() {
        let mut shot = good_minimax();
        shot.as_object_mut().unwrap().remove("seed");
        let e = set().check_prompt_pack(&pack(shot)).unwrap_err();
        assert!(e.message().contains("seed"), "{}", e.message());
        assert!(e.message().contains("默认值"), "{}", e.message());
    }

    /// 未核验的基线要在提交时就挡下，而不是等渲染时才报。
    #[test]
    fn an_unverified_baseline_is_rejected_at_submit_time() {
        let shot = json!({ "shot_id": "sh01", "workflow": "wan2_2/i2v", "seed": 1 });
        let e = set().check_prompt_pack(&pack(shot)).unwrap_err();
        let msg = e.message();
        assert!(msg.contains("尚未真机核验"), "{msg}");
        assert!(msg.contains("连线尚未确认"), "要带上具体原因：{msg}");
        assert!(msg.contains("minimax_h3/t2v"), "要给出可用的替代：{msg}");
    }

    #[test]
    fn an_unknown_baseline_lists_the_available_ones() {
        let shot = json!({ "shot_id": "sh01", "workflow": "made_up/t2v" });
        let e = set().check_prompt_pack(&pack(shot)).unwrap_err();
        let msg = e.message();
        assert!(msg.contains("没有名为 made_up/t2v"), "{msg}");
        assert!(msg.contains("ltx2_5/t2v"), "{msg}");
    }

    /// 非注入字段（shot_id / workflow / audio）不受这套校验管辖。
    #[test]
    fn non_injectable_fields_are_not_flagged() {
        let mut shot = good_minimax();
        shot["audio"] = json!("环境声：湖水");
        shot["shot_id"] = json!("sh01");
        set().check_prompt_pack(&pack(shot)).unwrap();
    }

    /// `references` 允许提前写：它声明的是资产关联，不是渲染参数。
    /// 基线还没有图片输入通道时写它不算错——等通道补上会自动生效。
    #[test]
    fn references_may_be_declared_before_the_baseline_supports_them() {
        let mut shot = good_minimax();
        shot["references"] = json!(["C01", "SC01"]);
        set().check_prompt_pack(&pack(shot)).unwrap();
    }

    /// 但基线一旦声明要 references，不写就是错——「少写」那个方向管住它。
    #[test]
    fn once_the_baseline_takes_references_omitting_them_is_an_error() {
        let caps = CapabilitySet::new(vec![WorkflowCapability {
            name: "minimax_h3/r2v".into(),
            params: ["positive", "references", "seed"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            verified: true,
            unavailable_reason: None,
        }]);
        let shot = json!({ "shot_id": "sh01", "workflow": "minimax_h3/r2v",
                           "positive": "p", "seed": 1 });
        let e = caps.check_prompt_pack(&pack(shot)).unwrap_err();
        assert!(e.message().contains("references"), "{}", e.message());
    }

    #[test]
    fn narrowing_the_schema_lists_only_the_verified_baselines() {
        let mut doc = crate::schema::stage_schema_document(StageId::PromptPack);
        set().narrow_schema(&mut doc);
        let field = doc
            .pointer("/properties/prompt_pack/properties/shots/items/properties/workflow")
            .unwrap();
        assert_eq!(
            field["enum"],
            json!(["ltx2_5/t2v", "minimax_h3/t2v"]),
            "未核验的 wan2_2/i2v 不该出现在可选项里"
        );
        assert!(field["description"].as_str().unwrap().contains("静默丢弃"));
    }

    /// 一条基线都没有时不动 schema——那是部署问题，不该用一个空 enum
    /// 把 Agent 卡在原地。
    #[test]
    fn an_empty_capability_set_leaves_the_schema_alone() {
        let before = crate::schema::stage_schema_document(StageId::PromptPack);
        let mut doc = before.clone();
        CapabilitySet::default().narrow_schema(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn verified_names_exclude_the_unverified_ones() {
        assert_eq!(
            set().verified_names(),
            vec!["ltx2_5/t2v".to_string(), "minimax_h3/t2v".to_string()]
        );
    }

    /// 形状不对的产物交给 schema 校验报，这里不重复报一遍。
    #[test]
    fn a_malformed_pack_is_left_to_the_schema_check() {
        let mut o = Outputs::new();
        o.insert("prompt_pack".into(), json!({ "shots": "不是数组" }));
        set().check_prompt_pack(&o).unwrap();
        set().check_prompt_pack(&Outputs::new()).unwrap();
    }
}
