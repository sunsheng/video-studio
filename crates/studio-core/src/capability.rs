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
//!
//! # 两种形状
//!
//! 从 [ADR-0005] 起，能力面有两个数据源，按 `core_model_family` 分派：
//!
//! - **整图基线**（`ltx2_5` / `wan2_2`…）：一个系列一组完整的图，镜头写
//!   `workflow: "ltx2_5/t2v"` 选其中一张，能力面来自基线的 `_studio.bindings`。
//! - **片段库**（`minimax_h3`）：图在渲染时按声明现场组装，镜头写
//!   `head` + `references` + `guides`，能力面来自骨架与各 head 的 `bindings`。
//!
//! **对账逻辑两边完全一样**（多写 → 静默丢弃，少写 → 走基线默认值），
//! 换的只是数据源。形状用错则当场报错并说清这个系列该写哪一种。
//!
//! [ADR-0005]: ../../../docs/decisions/ADR-0005-workflow-fragments.md

use crate::assembly::{self, FragmentSet, ShotDeclaration};
use crate::error::{Result, StudioError, Violation};
use crate::stage::StageId;
use crate::Outputs;
use std::collections::BTreeMap;

/// 「写了但基线不吃」要报错的参数名。
///
/// 逐镜头产物里还有 `shot_id`、`workflow`、`audio` 这些不进节点图的字段，
/// 它们不在这里，也就不受这套校验约束。
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

/// 只有片段化的系列才吃的声明式字段。写在整图基线的镜头上要当场挡下。
///
/// 这是 ADR-0005 收紧的那条豁免：以前 `references` 允许「提前写，等基线
/// 支持了自动生效」，因为那时候确实没有别的地方能声明参考资产。现在有了
/// ——`minimax_h3` 的片段库就是为参考、锚点准备的。整图基线上再写这些，
/// 只会被静默丢弃，没有任何以后会生效的路径，所以规则从「允许」改成「禁止，
/// 并指出该换哪个系列」。
const DECLARATIVE_FIELDS: [&str; 5] = ["head", "references", "guides", "first_frame", "last_frame"];

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

/// 当前这台机器上全部可用基线，整图与片段两种形状都在里面。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapabilitySet {
    workflows: Vec<WorkflowCapability>,
    /// 系列名 → 片段库。在这里出现的系列走声明式形状，不写 `workflow`。
    fragments: BTreeMap<String, FragmentSet>,
}

impl CapabilitySet {
    pub fn new(workflows: Vec<WorkflowCapability>) -> Self {
        Self {
            workflows,
            fragments: BTreeMap::new(),
        }
    }

    /// 挂上片段化系列的片段库。只收「真的能跑」的（有骨架 + 至少一个已核验
    /// head）——半份片段库不该让整个系列切到声明式形状，那会把 Agent 引到
    /// 一条走不通的路上。
    pub fn with_fragments(mut self, sets: BTreeMap<String, FragmentSet>) -> Self {
        self.fragments = sets.into_iter().filter(|(_, s)| s.is_usable()).collect();
        self
    }

    pub fn get(&self, name: &str) -> Option<&WorkflowCapability> {
        self.workflows.iter().find(|w| w.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.workflows.is_empty() && self.fragments.is_empty()
    }

    /// 这个系列的片段库，没有就说明它走整图基线。
    pub fn fragments_for(&self, family: &str) -> Option<&FragmentSet> {
        self.fragments.get(family)
    }

    /// 走声明式形状的系列名，排序后返回。
    pub fn fragment_families(&self) -> Vec<String> {
        self.fragments.keys().cloned().collect()
    }

    /// 属于这个系列的整图基线名（已核验的），排序后返回。
    fn family_names(&self, family: &str) -> Vec<String> {
        let prefix = format!("{family}/");
        let mut v: Vec<String> = self
            .workflows
            .iter()
            .filter(|w| w.verified && w.name.starts_with(&prefix))
            .map(|w| w.name.clone())
            .collect();
        v.sort();
        v
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

    /// 把 `prompt_pack` 的 schema 文档收窄到这台机器真正能跑的东西。
    ///
    /// Agent 提交前会先调 `studio.schema` —— 与其让它写完一整包提示词、
    /// 提交时才被告知「这条基线没核验」，不如在它看 schema 的那一刻就只给
    /// 能用的那几条。
    ///
    /// `family` 是上游 `visual_assets` 定下的 `core_model_family`。给了就
    /// **按系列只留一种形状**：片段化的系列删掉 `workflow`，整图的系列删掉
    /// 声明式那几个字段——两种形状同时摆在 Agent 面前，它一定会混着写。
    /// 传 `None`（上游还没定）时两种都留，各自收窄取值。
    ///
    /// 没有可用能力面时不动 schema：那是部署问题，该在渲染时以
    /// `model_contract_violation` 报出来，不是靠一个空 enum 把 Agent 卡在原地。
    pub fn narrow_schema(&self, doc: &mut serde_json::Value, family: Option<&str>) {
        let Some(props) = doc
            .pointer_mut("/properties/prompt_pack/properties/shots/items/properties")
            .and_then(|p| p.as_object_mut())
        else {
            return;
        };

        let fragment_set = family.and_then(|f| self.fragments.get(f));
        let whole_graph_names = match family {
            Some(f) if fragment_set.is_none() => self.family_names(f),
            Some(_) => Vec::new(),
            None => self.verified_names(),
        };

        // 片段化的系列：删掉 workflow，把 head 收窄到这台机器有的那几个。
        if let Some(set) = fragment_set {
            props.remove("workflow");
            props.remove("duration_seconds");
            let heads = set.verified_heads();
            if let Some(obj) = props.get_mut("head").and_then(|h| h.as_object_mut()) {
                obj.insert("enum".into(), serde_json::json!(heads));
                obj.insert(
                    "description".into(),
                    serde_json::json!(format!(
                        "这一镜用哪种生成方式。这台机器上可用的就是这几个：{}。\
                         reference 挂参考锁身份与风格，起幅由模型定；\
                         image 给首尾帧，锁构图与运动轨迹。\
                         图在渲染时按这份声明现场组装，没有 workflow 字段可选",
                        heads.join("、")
                    )),
                );
            }
            return;
        }

        // 整图基线：删掉声明式字段，把 workflow 收窄到可用的那几条。
        if family.is_some() || !self.fragments.is_empty() {
            for f in DECLARATIVE_FIELDS {
                props.remove(f);
            }
        }
        if whole_graph_names.is_empty() {
            return;
        }
        let Some(obj) = props.get_mut("workflow").and_then(|w| w.as_object_mut()) else {
            return;
        };
        obj.insert("enum".into(), serde_json::json!(whole_graph_names));
        obj.insert(
            "description".into(),
            serde_json::json!(
                "使用的已验证 workflow 名。这台机器上可用的就是这几条，\
                 未核验的基线不在其中。每条吃的参数不同——写了它不吃的参数\
                 会被静默丢弃，提交时会被挡下；写之前先看这个系列的能力卡"
            ),
        );
    }

    /// 对提示词包做双向对账，按 `core_model_family` 分派到两种形状之一。
    ///
    /// 两个方向都要查，因为两种错法的后果一样严重：
    /// **多写**的参数会被静默丢弃，**少写**的参数会让基线用自己的默认值。
    ///
    /// `known_assets` 是 `visual_assets` 登记过的产物 id，用来查参考与锚点
    /// 引用的资产是否真的存在。传空切片表示跳过这一条（上游还没产出时不该
    /// 拿它卡人）。
    pub fn check_prompt_pack(&self, outputs: &Outputs, known_assets: &[String]) -> Result<()> {
        let pack = outputs.get("prompt_pack");
        let Some(shots) = pack.and_then(|p| p.get("shots")).and_then(|s| s.as_array()) else {
            // 形状本身不对，交给 schema 校验去报——这里不重复报一遍。
            return Ok(());
        };
        let family = pack
            .and_then(|p| p.get("core_model_family"))
            .and_then(|f| f.as_str())
            .unwrap_or_default();

        let violations = match self.fragments.get(family) {
            Some(set) => self.check_declarative(set, family, shots, known_assets),
            None => self.check_whole_graph(shots),
        };

        if violations.is_empty() {
            Ok(())
        } else {
            Err(StudioError::SchemaViolation {
                stage: StageId::PromptPack,
                violations,
            })
        }
    }

    /// 片段化的系列：形状由 `head` + `references` + `guides` 描述，
    /// 组合合法性交给 [`assembly::validate_shot`]，参数对账的数据源换成
    /// 「骨架 + head 的 bindings」——逻辑与整图那条一模一样。
    fn check_declarative(
        &self,
        set: &FragmentSet,
        family: &str,
        shots: &[serde_json::Value],
        known_assets: &[String],
    ) -> Vec<Violation> {
        let mut violations = Vec::new();
        let params = set.shot_params();
        // 接续镜引用上一镜的尾段，所以要知道谁排在谁前面。
        let ids: Vec<String> = shots
            .iter()
            .map(|s| {
                s.get("shot_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        for (i, shot) in shots.iter().enumerate() {
            let at = |field: &str| format!("prompt_pack.shots[{i}].{field}");
            let Some(obj) = shot.as_object() else {
                continue;
            };
            let prior_shots = &ids[..i];

            // 形状用错：这个系列的图是现场组装的，没有整图基线可选。
            if obj.contains_key("workflow") {
                violations.push(Violation::new(
                    at("workflow"),
                    format!(
                        "{family} 的图在渲染时按声明现场组装，没有整图基线可选——\
                         写 workflow 不会有任何效果。删掉它，改用 head（{}）\
                         配 references / guides 描述这一镜要什么。",
                        set.verified_heads().join("、")
                    ),
                ));
                continue;
            }

            // 方向一：写了这个系列不吃的参数。
            for param in INJECTABLE_PARAMS {
                if !obj.contains_key(param) || params.iter().any(|p| p == param) {
                    continue;
                }
                violations.push(Violation::new(at(param), dropped_hint(param, family)));
            }
            // 方向二：这个系列要的参数没写。
            for param in &params {
                if obj.contains_key(param.as_str()) {
                    continue;
                }
                violations.push(Violation::new(
                    at(param),
                    format!(
                        "{family} 的骨架接受 {param}，但这一镜没写。\
                         不写就用片段自带的默认值，结果不受你控制"
                    ),
                ));
            }

            // 组合合法性：head 认不认这些参考与锚点、帧数在不在网格上。
            // 反序列化失败说明形状不对，那是 schema 的事，这里不重复报。
            if let Ok(decl) = serde_json::from_value::<ShotDeclaration>(shot.clone()) {
                violations.extend(assembly::validate_shot(
                    set,
                    &decl,
                    known_assets,
                    prior_shots,
                    i,
                ));
            }
        }
        violations
    }

    /// 整图基线：镜头写 `workflow` 选一张已验证的图。
    fn check_whole_graph(&self, shots: &[serde_json::Value]) -> Vec<Violation> {
        let mut violations = Vec::new();
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

            // 形状用错：声明式那几个字段在整图基线上没有任何落点。
            for field in DECLARATIVE_FIELDS {
                if !obj.contains_key(field) {
                    continue;
                }
                violations.push(Violation::new(
                    at(field),
                    declarative_on_whole_graph(field, name, &self.fragment_families()),
                ));
            }

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
        violations
    }
}

/// 声明式字段写在整图基线上——不是「以后会生效」，是这条路根本不通。
fn declarative_on_whole_graph(field: &str, workflow: &str, families: &[String]) -> String {
    let head = format!(
        "{field} 只有片段化的系列吃，基线 {workflow} 上写了不会有任何效果，\
         也不会以后自动生效——整图基线的节点是固定的，没有挂参考的槽位"
    );
    if families.is_empty() {
        format!("{head}。这台机器上没有片段化的系列，把这个需求改写进 positive。")
    } else {
        format!(
            "{head}。要挂参考或锚点，把 core_model_family 换成 {}，\
             那边用 head + references + guides 描述。",
            families.join("、")
        )
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
    use crate::assembly::tests_support::fragments;
    use serde_json::json;

    /// 只有整图基线的那台机器：两条已核验、一条没核验。
    fn set() -> CapabilitySet {
        CapabilitySet::new(vec![
            WorkflowCapability {
                name: "wan2_2/t2v".into(),
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

    /// 同一台机器，外加 `minimax_h3` 的片段库——真实部署就是这个样子。
    fn mixed() -> CapabilitySet {
        set().with_fragments(BTreeMap::from([("minimax_h3".to_string(), fragments())]))
    }

    fn pack(family: &str, shot: serde_json::Value) -> Outputs {
        let mut o = Outputs::new();
        o.insert(
            "prompt_pack".into(),
            json!({ "core_model_family": family, "shots": [shot] }),
        );
        o
    }

    /// 按帧数收时长、不吃 negative 的整图基线镜头。
    fn whole_graph_shot() -> serde_json::Value {
        json!({
            "shot_id": "sh01", "workflow": "wan2_2/t2v",
            "positive": "船头切开湖面", "width": 1080, "height": 1920,
            "length_frames": 42, "fps": 30, "seed": 101001,
            "audio": "湖水拍打船身"
        })
    }

    /// 片段化系列的镜头：没有 workflow，用 head 声明。
    fn declarative_shot() -> serde_json::Value {
        json!({
            "shot_id": "sh01", "head": "reference",
            "positive": "船头切开湖面", "width": 768, "height": 1344,
            "length_frames": 56, "fps": 24, "seed": 101001,
            "audio": "湖水拍打船身"
        })
    }

    // ---------- 整图基线那条路 ----------

    #[test]
    fn a_pack_matching_the_baseline_passes() {
        set()
            .check_prompt_pack(&pack("wan2_2", whole_graph_shot()), &[])
            .unwrap();
    }

    /// 这是最要紧的一条：negative 在这条基线上会被静默丢弃。
    #[test]
    fn writing_negative_on_a_baseline_without_it_is_rejected() {
        let mut shot = whole_graph_shot();
        shot["negative"] = json!("文字, 水印");
        let e = set()
            .check_prompt_pack(&pack("wan2_2", shot), &[])
            .unwrap_err();
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
        let e = set()
            .check_prompt_pack(&pack("ltx2_5", shot), &[])
            .unwrap_err();
        let msg = e.message();
        // 多写了 length_frames
        assert!(msg.contains("length_frames"), "{msg}");
        assert!(msg.contains("按秒收时长"), "{msg}");
        // 少写了 duration_seconds
        assert!(msg.contains("duration_seconds"), "{msg}");
    }

    #[test]
    fn a_missing_parameter_is_reported_with_the_default_warning() {
        let mut shot = whole_graph_shot();
        shot.as_object_mut().unwrap().remove("seed");
        let e = set()
            .check_prompt_pack(&pack("wan2_2", shot), &[])
            .unwrap_err();
        assert!(e.message().contains("seed"), "{}", e.message());
        assert!(e.message().contains("默认值"), "{}", e.message());
    }

    /// 未核验的基线要在提交时就挡下，而不是等渲染时才报。
    #[test]
    fn an_unverified_baseline_is_rejected_at_submit_time() {
        let shot = json!({ "shot_id": "sh01", "workflow": "wan2_2/i2v", "seed": 1 });
        let e = set()
            .check_prompt_pack(&pack("wan2_2", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("尚未真机核验"), "{msg}");
        assert!(msg.contains("连线尚未确认"), "要带上具体原因：{msg}");
        assert!(msg.contains("wan2_2/t2v"), "要给出可用的替代：{msg}");
    }

    #[test]
    fn an_unknown_baseline_lists_the_available_ones() {
        let shot = json!({ "shot_id": "sh01", "workflow": "made_up/t2v" });
        let e = set()
            .check_prompt_pack(&pack("made_up", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("没有名为 made_up/t2v"), "{msg}");
        assert!(msg.contains("ltx2_5/t2v"), "{msg}");
    }

    /// 非注入字段（shot_id / workflow / audio）不受这套校验管辖。
    #[test]
    fn non_injectable_fields_are_not_flagged() {
        let mut shot = whole_graph_shot();
        shot["audio"] = json!("环境声：湖水");
        shot["shot_id"] = json!("sh01");
        set().check_prompt_pack(&pack("wan2_2", shot), &[]).unwrap();
    }

    /// ADR-0005 收紧的那条豁免：`references` 以前允许「提前写，等基线支持了
    /// 自动生效」。现在有片段化的系列专门干这个，整图基线上再写就是死路，
    /// 必须当场挡下并指出该换哪个系列。
    #[test]
    fn declarative_fields_on_a_whole_graph_baseline_are_rejected() {
        let mut shot = whole_graph_shot();
        shot["references"] = json!([{ "kind": "image", "asset_id": "C01" }]);
        let e = mixed()
            .check_prompt_pack(&pack("wan2_2", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("prompt_pack.shots[0].references"), "{msg}");
        assert!(msg.contains("不会以后自动生效"), "{msg}");
        assert!(msg.contains("minimax_h3"), "要指出该换哪个系列：{msg}");
    }

    /// 一台机器上一个片段化系列都没有时，报错不能凭空推荐一个不存在的系列。
    #[test]
    fn without_any_fragment_family_the_remedy_falls_back_to_the_prompt() {
        let mut shot = whole_graph_shot();
        shot["head"] = json!("reference");
        let e = set()
            .check_prompt_pack(&pack("wan2_2", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("没有片段化的系列"), "{msg}");
        assert!(msg.contains("positive"), "{msg}");
    }

    // ---------- 片段库那条路 ----------

    #[test]
    fn a_declarative_pack_matching_the_fragment_library_passes() {
        mixed()
            .check_prompt_pack(&pack("minimax_h3", declarative_shot()), &[])
            .unwrap();
    }

    /// 形状用错的另一半：片段化的系列没有整图基线可选。
    #[test]
    fn writing_workflow_on_a_fragment_family_is_rejected() {
        let mut shot = declarative_shot();
        shot["workflow"] = json!("minimax_h3/t2v");
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("prompt_pack.shots[0].workflow"), "{msg}");
        assert!(msg.contains("现场组装"), "{msg}");
        assert!(msg.contains("reference"), "要列出可用的 head：{msg}");
    }

    /// 对账逻辑在片段这条路上一模一样：negative 一样会被静默丢弃。
    #[test]
    fn negative_is_dropped_on_the_fragment_family_too() {
        let mut shot = declarative_shot();
        shot["negative"] = json!("文字, 水印");
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("prompt_pack.shots[0].negative"), "{msg}");
        assert!(msg.contains("静默丢弃"), "{msg}");
    }

    #[test]
    fn a_missing_parameter_on_the_fragment_family_is_reported() {
        let mut shot = declarative_shot();
        shot.as_object_mut().unwrap().remove("length_frames");
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &[])
            .unwrap_err();
        assert!(e.message().contains("length_frames"), "{}", e.message());
        assert!(e.message().contains("默认值"), "{}", e.message());
    }

    /// 缺 seed 不该把这一镜的组合校验整个挡掉——不然 Agent 补上种子
    /// 重新提交，才看到剩下那堆错。一次报全比来回两趟好。
    #[test]
    fn a_missing_seed_does_not_mask_the_combination_rules() {
        let mut shot = declarative_shot();
        shot.as_object_mut().unwrap().remove("seed");
        shot["length_frames"] = json!(50); // 同时不在帧网格上
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &[])
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("seed"), "缺种子要报：{msg}");
        assert!(msg.contains("帧数网格"), "组合校验也要照跑：{msg}");
    }

    /// 组合合法性由 assembly::validate_shot 出，这里只确认它真的接上了。
    #[test]
    fn combination_rules_run_on_the_fragment_family() {
        let mut shot = declarative_shot();
        shot["length_frames"] = json!(50); // 不在 17k+5 网格上
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &[])
            .unwrap_err();
        assert!(e.message().contains("帧数网格"), "{}", e.message());
    }

    /// V7：参考的资产必须在 visual_assets 里登记过。
    #[test]
    fn a_reference_to_an_unregistered_asset_is_rejected() {
        let mut shot = declarative_shot();
        shot["references"] = json!([{ "kind": "image", "asset_id": "C99" }]);
        let known = vec!["C01".to_string(), "SC01".to_string()];
        let e = mixed()
            .check_prompt_pack(&pack("minimax_h3", shot), &known)
            .unwrap_err();
        let msg = e.message();
        assert!(msg.contains("C99"), "{msg}");
        assert!(msg.contains("C01"), "要列出可用的：{msg}");
    }

    /// 半份片段库（没有骨架）不该让整个系列切到声明式形状——那会把 Agent
    /// 引到一条走不通的路上，不如让它继续走整图基线。
    #[test]
    fn an_incomplete_fragment_library_is_not_accepted() {
        let mut half = fragments();
        half.backbone = None;
        let caps = set().with_fragments(BTreeMap::from([("minimax_h3".to_string(), half)]));
        assert!(caps.fragment_families().is_empty());
        assert!(caps.fragments_for("minimax_h3").is_none());
    }

    // ---------- schema 收窄 ----------

    fn shot_props(doc: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
        doc.pointer("/properties/prompt_pack/properties/shots/items/properties")
            .unwrap()
            .as_object()
            .unwrap()
    }

    #[test]
    fn narrowing_the_schema_lists_only_the_verified_baselines() {
        let mut doc = crate::schema::stage_schema_document(StageId::PromptPack);
        set().narrow_schema(&mut doc, None);
        let field = &shot_props(&doc)["workflow"];
        assert_eq!(
            field["enum"],
            json!(["ltx2_5/t2v", "wan2_2/t2v"]),
            "未核验的 wan2_2/i2v 不该出现在可选项里"
        );
        assert!(field["description"].as_str().unwrap().contains("静默丢弃"));
    }

    /// 片段化的系列：只留声明式那一种形状，workflow 整个删掉。
    /// 两种形状同时摆着，Agent 一定会混着写。
    #[test]
    fn a_fragment_family_gets_only_the_declarative_shape() {
        let mut doc = crate::schema::stage_schema_document(StageId::PromptPack);
        mixed().narrow_schema(&mut doc, Some("minimax_h3"));
        let props = shot_props(&doc);
        assert!(
            !props.contains_key("workflow"),
            "片段化的系列没有整图基线可选"
        );
        assert!(
            !props.contains_key("duration_seconds"),
            "这个系列按帧数收时长"
        );
        assert_eq!(props["head"]["enum"], json!(["image", "reference"]));
        assert!(props.contains_key("references"));
        assert!(props.contains_key("guides"));
    }

    /// 整图基线的系列：声明式字段全删掉，workflow 只留本系列的。
    #[test]
    fn a_whole_graph_family_gets_only_the_workflow_shape() {
        let mut doc = crate::schema::stage_schema_document(StageId::PromptPack);
        mixed().narrow_schema(&mut doc, Some("ltx2_5"));
        let props = shot_props(&doc);
        for f in DECLARATIVE_FIELDS {
            assert!(!props.contains_key(f), "{f} 不该留给整图基线");
        }
        assert_eq!(
            props["workflow"]["enum"],
            json!(["ltx2_5/t2v"]),
            "只留本系列的基线，别的系列的图跟这个系列的参数对不上"
        );
    }

    /// 一条基线都没有时不动 schema——那是部署问题，不该用一个空 enum
    /// 把 Agent 卡在原地。
    #[test]
    fn an_empty_capability_set_leaves_the_schema_alone() {
        let before = crate::schema::stage_schema_document(StageId::PromptPack);
        let mut doc = before.clone();
        CapabilitySet::default().narrow_schema(&mut doc, None);
        assert_eq!(doc, before);
    }

    #[test]
    fn verified_names_exclude_the_unverified_ones() {
        assert_eq!(
            set().verified_names(),
            vec!["ltx2_5/t2v".to_string(), "wan2_2/t2v".to_string()]
        );
    }

    /// 形状不对的产物交给 schema 校验报，这里不重复报一遍。
    #[test]
    fn a_malformed_pack_is_left_to_the_schema_check() {
        let mut o = Outputs::new();
        o.insert("prompt_pack".into(), json!({ "shots": "不是数组" }));
        set().check_prompt_pack(&o, &[]).unwrap();
        set().check_prompt_pack(&Outputs::new(), &[]).unwrap();
    }
}
