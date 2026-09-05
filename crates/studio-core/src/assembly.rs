//! 把逐镜头的**声明**翻译成 ComfyUI 的节点图。
//!
//! 见 [ADR-0005]：基线从「三张完整的图」降级为片段库，每一镜的图按 Agent
//! 提交的 `head` / `references` / `guides` 现场组装。
//!
//! **这里只有翻译，没有创作判断，也没有 I/O。** 片段从哪来（读文件还是测试
//! 里手写）由上层决定；这一层拿到 [`FragmentSet`] 就能把声明拼成图，所以
//! 「给定这份声明应该拼出这张图」可以在没有 GPU、没有 ComfyUI 的机器上
//! 完整单测。
//!
//! 两条必须守住的性质：
//!
//! 1. **确定性**：同一份声明组装两次，输出逐字节相同。否则
//!    `studio.retry_stage`（「内容没问题，原样重跑」）失去意义，落盘的
//!    debug 请求也对不上。
//! 2. **不推断接线**：所有连线来自片段元数据，而片段的接线是从已验证基线
//!    逐字抄的。按接口类型推断会静默错接——`decode_audio` 接 `sampler` 的
//!    第 0 个而非第 1 个输出就是活例子，两边类型都是 `LATENT`，类型系统
//!    挡不住。
//!
//! [ADR-0005]: ../../../docs/decisions/ADR-0005-workflow-fragments.md

use crate::error::{Result, StudioError, Violation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// 参考 / guide 的介质类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Medium {
    Image,
    Video,
    Audio,
}

impl Medium {
    pub fn as_str(self) -> &'static str {
        match self {
            Medium::Image => "image",
            Medium::Video => "video",
            Medium::Audio => "audio",
        }
    }
}

/// 一条参考声明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub kind: Medium,
    /// 指向 `visual_assets` 登记的产物。
    pub asset_id: String,
    /// 仅 `kind: video` 有意义：同时占用 `ref_videos` 与
    /// `ref_video_audios` 的**同号**槽位。
    #[serde(default)]
    pub with_audio: bool,
}

/// 一条 guide 声明：把某个素材锚在某一帧上。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guide {
    /// `image` | `clip` | `audio`。`clip` 用的也是图片输入（帧序列）。
    pub kind: GuideKind,
    /// 负数从末尾倒数。
    pub at_frame: i64,
    pub asset_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideKind {
    Image,
    Clip,
    Audio,
}

impl GuideKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GuideKind::Image => "image",
            GuideKind::Clip => "clip",
            GuideKind::Audio => "audio",
        }
    }
}

/// 一镜的完整声明——Agent 说「要什么」，不说「怎么接」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotDeclaration {
    pub shot_id: String,
    /// 片段库里的 head id，例如 `reference` / `image`。
    pub head: String,
    pub positive: String,
    pub width: i64,
    pub height: i64,
    pub length_frames: i64,
    pub fps: f64,
    /// **有意可缺省。** 种子是必写的（不写就不可复现），但那条由能力面对账
    /// 报出来。如果这里不给默认值，缺一个 seed 会让整份声明反序列化失败，
    /// 于是 V1–V9 一条都跑不了——Agent 补上种子重新提交，才看到剩下那堆
    /// 组合错误。一次报全比来回两趟好。
    #[serde(default)]
    pub seed: i64,
    #[serde(default)]
    pub references: Vec<Reference>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// `image` head 专用的首尾帧（不是 AUTOGROW，是具名槽位）。
    #[serde(default)]
    pub first_frame: Option<String>,
    #[serde(default)]
    pub last_frame: Option<String>,
}

/// 一个 AUTOGROW 槽位的规格，取自真机 `/object_info`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutogrowSlot {
    /// 形如 `h3_ref.inputs.ref_images`。
    pub target: String,
    /// 键名前缀，序号从 1 起，例如 `ref_image_`。
    pub prefix: String,
    pub max: usize,
    /// 这个槽位**真的起作用**验过没有。
    ///
    /// 跟输入片段的 `bindings_verified` 是两件事：那个说的是「素材进得去」，
    /// 这个说的是「进去之后模型理不理它」。`ref_audios` 就是活例子——
    /// LoadAudio 通道验过了（audio 锚点上 1kHz 纯音在输出里 4000 倍于邻频），
    /// 但同一段音频挂到 `ref_audios` 上，输出里一点痕迹都没有。接线合法、
    /// 图能跑、有音轨出来，就是参考没生效。两者不分开的话，只能二选一：
    /// 要么挡掉已经验通的锚点，要么把没生效的参考说成可用。
    pub verified: bool,
    /// 未核验时的原因，用在错误消息里。
    pub unverified_reason: Option<String>,
}

/// 一份片段。`nodes` 是节点图的一部分，其余是组装用的元数据。
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub id: String,
    pub nodes: Map<String, Value>,
    pub verified: bool,
    pub unavailable_reason: Option<String>,
    /// 参数名 → 若干个 `<节点>.inputs.<字段>`。
    pub bindings: BTreeMap<String, Vec<String>>,
    /// head 专用：要覆盖到骨架上的配套约束（权重名、调度器档位）。
    pub backbone_overrides: BTreeMap<String, Value>,
    /// head 专用：对外的输出端口。
    pub outputs: BTreeMap<String, Value>,
    /// head 专用：要从骨架接进来的线。
    pub wires_from_backbone: BTreeMap<String, Value>,
    /// head 专用：AUTOGROW 槽位，按介质。
    pub autogrow: BTreeMap<String, AutogrowSlot>,
    /// head 专用：具名的首尾帧槽位。
    pub frames: BTreeMap<String, String>,
    /// guide 专用：链式接线的三个位置。
    pub chain: BTreeMap<String, Value>,
    /// guide / input 专用：素材接到哪个输入上。
    pub media_input: Option<String>,
    /// input 专用：对外输出端口。
    pub input_outputs: BTreeMap<String, Value>,
    /// 骨架专用：必须由组装器填的位置，留空就报错。
    pub must_be_filled: Vec<String>,
}

impl Fragment {
    pub fn new(id: impl Into<String>, nodes: Map<String, Value>) -> Self {
        Fragment {
            id: id.into(),
            nodes,
            verified: true,
            unavailable_reason: None,
            bindings: BTreeMap::new(),
            backbone_overrides: BTreeMap::new(),
            outputs: BTreeMap::new(),
            wires_from_backbone: BTreeMap::new(),
            autogrow: BTreeMap::new(),
            frames: BTreeMap::new(),
            chain: BTreeMap::new(),
            media_input: None,
            input_outputs: BTreeMap::new(),
            must_be_filled: Vec::new(),
        }
    }

    /// 从落盘的片段文件解析。**纯数据变换，不碰文件系统**——读文件是上层的事，
    /// 这样「这个格式怎么解析」在没有 GPU、没有片段目录的机器上也能单测。
    ///
    /// 形状：顶层是节点图的一部分，外加一个 `_studio` 元数据块。
    /// `where_` 只用在报错消息里，指出是哪份文件。
    pub fn parse(text: &str, where_: &str) -> Result<(FragmentKind, Fragment)> {
        let bad = |detail: String| StudioError::ModelContractViolation { detail };
        let doc: Value = serde_json::from_str(text)
            .map_err(|e| bad(format!("片段 {where_} 不是合法 JSON：{e}")))?;
        let obj = doc
            .as_object()
            .ok_or_else(|| bad(format!("片段 {where_} 顶层必须是对象")))?;
        let meta = obj
            .get("_studio")
            .and_then(|m| m.as_object())
            .ok_or_else(|| bad(format!("片段 {where_} 缺 _studio 元数据块")))?;

        let kind = meta
            .get("kind")
            .and_then(|k| k.as_str())
            .and_then(FragmentKind::parse)
            .ok_or_else(|| {
                bad(format!(
                    "片段 {where_} 的 _studio.kind 缺失或不认识，\
                     只接受 backbone / head / guide / input"
                ))
            })?;
        let id = meta
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or_else(|| bad(format!("片段 {where_} 缺 _studio.id")))?;

        let nodes: Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| k.as_str() != "_studio")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if nodes.is_empty() {
            return Err(bad(format!("片段 {where_} 一个节点都没有")));
        }

        let mut frag = Fragment::new(id, nodes);
        frag.verified = meta
            .get("bindings_verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        frag.unavailable_reason = meta
            .get("unavailable_reason")
            .and_then(|v| v.as_str())
            .map(String::from);
        frag.bindings = string_lists(meta.get("bindings"));
        frag.backbone_overrides = value_map(meta.get("backbone_overrides"));
        frag.wires_from_backbone = value_map(meta.get("wires_from_backbone"));
        frag.chain = value_map(meta.get("chain"));
        frag.media_input = meta
            .get("media_input")
            .and_then(|v| v.as_str())
            .map(String::from);
        frag.frames = meta
            .get("frames")
            .and_then(|f| f.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        frag.must_be_filled = meta
            .get("must_be_filled")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        frag.autogrow = meta
            .get("autogrow")
            .and_then(|a| a.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| {
                        Some((
                            k.clone(),
                            AutogrowSlot {
                                target: v.get("target")?.as_str()?.to_string(),
                                prefix: v.get("prefix")?.as_str()?.to_string(),
                                max: v.get("max")?.as_u64()? as usize,
                                // 没写就是验过的——绝大多数槽位是从已验证基线切来的。
                                verified: v
                                    .get("verified")
                                    .and_then(|x| x.as_bool())
                                    .unwrap_or(true),
                                unverified_reason: v
                                    .get("unverified_reason")
                                    .and_then(|x| x.as_str())
                                    .map(String::from),
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // `outputs` 在 head 和 input 上含义不同：head 给的是喂给骨架的
        // conditioning / latent，input 给的是素材的 IMAGE / AUDIO。
        let outputs = value_map(meta.get("outputs"));
        match kind {
            FragmentKind::Input => frag.input_outputs = outputs,
            _ => frag.outputs = outputs,
        }

        // 声明的端口必须指向片段内真实存在的节点，否则要等到组装时才炸。
        let node_exists = |port: &Value| -> bool {
            port.as_array()
                .and_then(|a| a.first())
                .and_then(|f| f.as_str())
                .is_some_and(|n| frag.nodes.contains_key(n))
        };
        for (name, port) in frag.outputs.iter().chain(frag.input_outputs.iter()) {
            if !node_exists(port) {
                return Err(bad(format!(
                    "片段 {where_} 的 outputs.{name} 指向的节点不在这份片段里"
                )));
            }
        }
        if let Some(port) = frag.chain.get("positive_out") {
            if !node_exists(port) {
                return Err(bad(format!(
                    "片段 {where_} 的 chain.positive_out 指向的节点不在这份片段里"
                )));
            }
        }

        Ok((kind, frag))
    }
}

fn string_lists(v: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    v.and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let list = v
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    (k.clone(), list)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn value_map(v: Option<&Value>) -> BTreeMap<String, Value> {
    v.and_then(|v| v.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// 一个模型系列的全部片段。上层负责把它填出来（读文件或测试里手写）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FragmentSet {
    pub backbone: Option<Fragment>,
    /// head id → 片段。
    pub heads: BTreeMap<String, Fragment>,
    /// guide kind → 片段。
    pub guides: BTreeMap<String, Fragment>,
    /// input 介质 → 片段。
    pub inputs: BTreeMap<String, Fragment>,
    /// head id → 该 head 的 turbo 叠加层。preview 用，正式渲染不挂。
    pub overlays: BTreeMap<String, Fragment>,
}

impl FragmentSet {
    /// 可用的 head id，排序后返回。用来把 schema 里 `head` 的取值收窄到
    /// 这台机器真正能跑的那些。
    pub fn verified_heads(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .heads
            .values()
            .filter(|f| f.verified)
            .map(|f| f.id.clone())
            .collect();
        v.sort();
        v
    }

    /// 把一份解析好的片段放进对应的格子里。
    pub fn insert(&mut self, kind: FragmentKind, frag: Fragment) {
        match kind {
            FragmentKind::Backbone => self.backbone = Some(frag),
            FragmentKind::Head => {
                self.heads.insert(frag.id.clone(), frag);
            }
            FragmentKind::Guide => {
                self.guides.insert(frag.id.clone(), frag);
            }
            FragmentKind::Input => {
                self.inputs.insert(frag.id.clone(), frag);
            }
            FragmentKind::Overlay => {
                self.overlays.insert(frag.id.clone(), frag);
            }
        }
    }

    /// 这个片段库能接受的逐镜头参数名，取自骨架与各 head 的 `bindings` 键集合。
    ///
    /// 这是 [`crate::CapabilitySet`] 对账「写了会被静默丢弃」的数据源：
    /// 整图基线看基线自己的 `_studio.bindings`，片段化的系列看这里。
    /// `output_prefix` 不算——那是控制面决定产物落在哪，不是 Agent 写的。
    pub fn shot_params(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .backbone
            .iter()
            .chain(self.heads.values())
            .flat_map(|f| f.bindings.keys())
            .filter(|k| k.as_str() != "output_prefix")
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// 至少有骨架和一个已核验的 head，才算这个系列真的能跑。
    pub fn is_usable(&self) -> bool {
        self.backbone.is_some() && !self.verified_heads().is_empty()
    }
}

/// 片段在组装里扮演的角色，取自 `_studio.kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FragmentKind {
    Backbone,
    Head,
    Guide,
    Input,
    /// 叠加在骨架上的可选组合，目前只有 preview 的 turbo LoRA。
    Overlay,
}

impl FragmentKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "backbone" => Some(FragmentKind::Backbone),
            "head" => Some(FragmentKind::Head),
            "guide" => Some(FragmentKind::Guide),
            "input" => Some(FragmentKind::Input),
            "overlay" => Some(FragmentKind::Overlay),
            _ => None,
        }
    }
}

/// 出图用哪一套组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combination {
    /// 正式渲染：骨架原样，steps 按基线。
    Standard,
    /// 预览：挂 head 配套的 turbo LoRA、steps 降到 LoRA 的步数。
    ///
    /// 片段库里没有对应的叠加层、或者它还没真机核验时**自动退回
    /// [`Combination::Standard`]**，并在 [`AssembledGraph::notes`] 里说明。
    /// preview 少花点时间是锦上添花，为它把整个预览阶段卡死不值得。
    PreviewTurbo,
}

/// 组装出来的节点图，外加一份可读的组装记录。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledGraph {
    pub graph: Value,
    /// 这张图用了哪些片段，按顺序。留痕用，出问题时能看出是怎么拼的。
    pub used: Vec<String>,
    /// 组装过程中做过的降级说明，比如 turbo 叠加层没核验所以退回了普通组合。
    /// 空的表示完全按请求的组合拼出来了。
    pub notes: Vec<String>,
}

/// 把声明翻译成节点图。
///
/// `output_prefix` 是 `save_video.filename_prefix` 的值，由调用方按
/// 阶段与 `shot_id` 决定——组装器不关心产物落在哪。
pub fn assemble(
    set: &FragmentSet,
    shot: &ShotDeclaration,
    output_prefix: &str,
) -> Result<AssembledGraph> {
    assemble_as(set, shot, output_prefix, Combination::Standard)
}

/// [`assemble`] 加一个组合的选择。见 [`Combination`]。
pub fn assemble_as(
    set: &FragmentSet,
    shot: &ShotDeclaration,
    output_prefix: &str,
    combination: Combination,
) -> Result<AssembledGraph> {
    let backbone = set
        .backbone
        .as_ref()
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: "片段库里没有骨架（backbone），无法组装".into(),
        })?;
    let head = set.heads.get(&shot.head).ok_or_else(|| {
        let mut available = set.verified_heads();
        available.sort();
        StudioError::ModelContractViolation {
            detail: format!(
                "片段库里没有 head「{}」。可用的：{}",
                shot.head,
                if available.is_empty() {
                    "（一个都没有）".to_string()
                } else {
                    available.join("、")
                }
            ),
        }
    })?;
    require_verified(head)?;

    let mut used = vec![
        format!("backbone/{}", backbone.id),
        format!("head/{}", head.id),
    ];
    let mut graph = Map::new();
    for (k, v) in &backbone.nodes {
        graph.insert(k.clone(), v.clone());
    }

    // 1. head 的配套约束覆盖骨架——权重名、调度器档位这类「跟着 head 走」的
    //    东西。它们不是可自由选的参数，是真机跑出来的契约。
    for (path, val) in &head.backbone_overrides {
        write_at(&mut graph, path, val.clone())?;
    }

    // 2. head 节点进图，接骨架给它的线
    for (k, v) in &head.nodes {
        graph.insert(k.clone(), v.clone());
    }
    for (path, wire) in &head.wires_from_backbone {
        write_at(&mut graph, path, wire.clone())?;
    }

    // 2b. 可选的叠加层（preview 的 turbo LoRA）。它跟 head 是配套的
    //     ——ref2v 的 LoRA 挂不到 fl2va 的权重上——所以按 head id 取。
    let mut notes = Vec::new();
    if combination == Combination::PreviewTurbo {
        match set.overlays.get(&head.id) {
            Some(o) if o.verified => {
                for (k, v) in &o.nodes {
                    graph.insert(k.clone(), v.clone());
                }
                for (path, wire) in &o.wires_from_backbone {
                    write_at(&mut graph, path, wire.clone())?;
                }
                for (path, val) in &o.backbone_overrides {
                    write_at(&mut graph, path, val.clone())?;
                }
                used.push(format!("overlay/{}", o.id));
            }
            // 没核验就退回普通组合并说明。preview 省时间是锦上添花，
            // 为它把整个预览阶段卡死不值得——但也不能让人以为跑的是 turbo。
            Some(o) => notes.push(format!(
                "turbo 叠加层「{}」尚未真机核验（{}），这一镜退回普通组合，\
                 预览会慢一些但结果可信",
                o.id,
                o.unavailable_reason.as_deref().unwrap_or("原因未记录")
            )),
            None => notes.push(format!("head「{}」没有 turbo 叠加层，用普通组合", head.id)),
        }
    }

    // 3. 逐镜头参数。骨架与 head 的 bindings 合起来用。
    let params = shot_params(shot, output_prefix);
    for bindings in [&backbone.bindings, &head.bindings] {
        for (name, targets) in bindings {
            let Some(val) = params.get(name) else {
                continue;
            };
            for t in targets {
                write_at(&mut graph, t, val.clone())?;
            }
        }
    }

    // 4. references → AUTOGROW 槽位，序号从 1 递增
    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    for (i, r) in shot.references.iter().enumerate() {
        let slot = head.autogrow.get(r.kind.as_str()).ok_or_else(|| {
            StudioError::ModelContractViolation {
                detail: format!(
                    "head「{}」不接 {} 类参考（它没有对应的 AUTOGROW 槽位）",
                    head.id,
                    r.kind.as_str()
                ),
            }
        })?;
        if !slot.verified {
            return Err(StudioError::ModelContractViolation {
                detail: format!(
                    "head「{}」的 {} 类参考槽位尚未核验，不能用来渲染：{}",
                    head.id,
                    r.kind.as_str(),
                    slot.unverified_reason.as_deref().unwrap_or("原因未记录")
                ),
            });
        }
        let input =
            set.inputs
                .get(r.kind.as_str())
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!("片段库里没有 {} 类输入片段", r.kind.as_str()),
                })?;
        require_verified(input)?;

        let prefix = format!("ref{}", i + 1);
        let ids = splice(&mut graph, input, &prefix);
        set_media(&mut graph, input, &ids, &r.asset_id)?;
        used.push(format!("input/{}#{}", input.id, prefix));

        let n = bump(&mut counters, r.kind.as_str());
        let port = port_of(input, "image_or_audio", r.kind)?;
        let wire = resolve_port(&ids, &port);
        push_autogrow(&mut graph, slot, n, wire)?;

        // 视频参考带音轨时，音频占同号槽位——这是 ComfyUI 那侧的约定
        // （`ref_video_audio_N` 对应 `ref_video_N`）。
        if r.with_audio && r.kind == Medium::Video {
            let audio_slot = head.autogrow.get("video_audio").ok_or_else(|| {
                StudioError::ModelContractViolation {
                    detail: format!("head「{}」没有 video_audio 槽位", head.id),
                }
            })?;
            let audio_port = input.input_outputs.get("audio").cloned().ok_or_else(|| {
                StudioError::ModelContractViolation {
                    detail: "视频输入片段没有 audio 输出端口".into(),
                }
            })?;
            let wire = resolve_port(&ids, &audio_port);
            push_autogrow(&mut graph, audio_slot, n, wire)?;
        }
    }

    // 5. guides 串链：positive 接上一个，latent 一律接 head。
    //    AddGuide 只输出 CONDITIONING，所以 latent 不可能串。
    let head_cond = head.outputs.get("conditioning").cloned().ok_or_else(|| {
        StudioError::ModelContractViolation {
            detail: format!("head「{}」没有 conditioning 输出端口", head.id),
        }
    })?;
    let head_latent =
        head.outputs
            .get("latent")
            .cloned()
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: format!("head「{}」没有 latent 输出端口", head.id),
            })?;

    let mut chain_tail = head_cond;
    for (j, g) in shot.guides.iter().enumerate() {
        let frag =
            set.guides
                .get(g.kind.as_str())
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!("片段库里没有 {} 类 guide 片段", g.kind.as_str()),
                })?;
        require_verified(frag)?;

        let media_kind = match g.kind {
            GuideKind::Audio => Medium::Audio,
            GuideKind::Image => Medium::Image,
            // `clip` 要的是**帧序列**，得走 LoadVideo + GetVideoComponents
            // 才能得到多帧的 IMAGE。用图片输入（LoadImage）只会喂进去一张，
            // 类型都是 IMAGE，图能过校验，但锚定的东西完全不是声明的那个
            // ——正是本项目最怕的那种静默错接。
            GuideKind::Clip => Medium::Video,
        };
        let input = set.inputs.get(media_kind.as_str()).ok_or_else(|| {
            StudioError::ModelContractViolation {
                detail: format!("片段库里没有 {} 类输入片段", media_kind.as_str()),
            }
        })?;
        require_verified(input)?;

        let gprefix = format!("guide{}", j + 1);
        let gids = splice(&mut graph, frag, &gprefix);
        let iprefix = format!("guide{}_src", j + 1);
        let iids = splice(&mut graph, input, &iprefix);
        set_media(&mut graph, input, &iids, &g.asset_id)?;
        used.push(format!("guide/{}#{}", frag.id, gprefix));

        let pos_in = chain_path(frag, "positive_in", &gids)?;
        let lat_in = chain_path(frag, "latent_in", &gids)?;
        write_at(&mut graph, &pos_in, chain_tail.clone())?;
        write_at(&mut graph, &lat_in, head_latent.clone())?;

        for t in frag.bindings.get("at_frame").into_iter().flatten() {
            write_at(&mut graph, &rename_path(t, &gids), Value::from(g.at_frame))?;
        }
        if let Some(mi) = &frag.media_input {
            let port = port_of(input, "media", media_kind)?;
            write_at(
                &mut graph,
                &rename_path(mi, &gids),
                resolve_port(&iids, &port),
            )?;
        }

        chain_tail = frag
            .chain
            .get("positive_out")
            .map(|p| resolve_port(&gids, p))
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: format!("guide 片段「{}」没有 positive_out 端口", frag.id),
            })?;
    }

    // 6. image head 的首尾帧：具名槽位，不是 AUTOGROW
    for (slot_name, asset) in [("first", &shot.first_frame), ("last", &shot.last_frame)] {
        let Some(asset_id) = asset else { continue };
        let target =
            head.frames
                .get(slot_name)
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!("head「{}」没有 {slot_name} 帧槽位", head.id),
                })?;
        let input = set
            .inputs
            .get("image")
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: "片段库里没有 image 输入片段".into(),
            })?;
        let prefix = format!("{slot_name}_frame");
        let ids = splice(&mut graph, input, &prefix);
        set_media(&mut graph, input, &ids, asset_id)?;
        let port = port_of(input, "media", Medium::Image)?;
        write_at(&mut graph, target, resolve_port(&ids, &port))?;
        used.push(format!("input/{}#{}", input.id, prefix));
    }

    // 7. 骨架里留空的三处必须填上。留残值会让换 head 时指向不存在的节点。
    write_at(&mut graph, "guider.inputs.conditioning", chain_tail)?;
    write_at(&mut graph, "sampler.inputs.latent_image", head_latent)?;
    for path in &backbone.must_be_filled {
        if read_at(&graph, path).is_none() {
            return Err(StudioError::ModelContractViolation {
                detail: format!("组装完成后 {path} 仍然是空的——这是骨架要求必填的位置"),
            });
        }
    }

    Ok(AssembledGraph {
        graph: Value::Object(graph),
        used,
        notes,
    })
}

fn require_verified(f: &Fragment) -> Result<()> {
    if f.verified {
        return Ok(());
    }
    Err(StudioError::ModelContractViolation {
        detail: format!(
            "片段「{}」尚未核验，不能用来渲染：{}",
            f.id,
            f.unavailable_reason
                .clone()
                .unwrap_or_else(|| "没有写明原因".into())
        ),
    })
}

fn shot_params(shot: &ShotDeclaration, output_prefix: &str) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    m.insert("positive".into(), Value::from(shot.positive.clone()));
    m.insert("width".into(), Value::from(shot.width));
    m.insert("height".into(), Value::from(shot.height));
    m.insert("length_frames".into(), Value::from(shot.length_frames));
    m.insert("fps".into(), Value::from(shot.fps));
    m.insert("seed".into(), Value::from(shot.seed));
    m.insert(
        "output_prefix".into(),
        Value::from(output_prefix.to_string()),
    );
    m
}

/// 把片段的节点插进图里，节点 id 加前缀。返回 `原 id → 新 id` 的映射。
///
/// 前缀由片段角色和序号决定，**不含随机数、不含时间戳**——同一份声明两次
/// 组装必须得到逐字节相同的图。
fn splice(
    graph: &mut Map<String, Value>,
    frag: &Fragment,
    prefix: &str,
) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for (orig, node) in &frag.nodes {
        let new_id = format!("{prefix}_{orig}");
        ids.insert(orig.clone(), new_id.clone());
        graph.insert(new_id, node.clone());
    }
    // 片段内部的相互引用也要跟着改名。
    for new_id in ids.values() {
        let Some(node) = graph.get_mut(new_id) else {
            continue;
        };
        let Some(inputs) = node.get_mut("inputs").and_then(|i| i.as_object_mut()) else {
            continue;
        };
        for v in inputs.values_mut() {
            if let Some(arr) = v.as_array_mut() {
                if let Some(first) = arr.first_mut() {
                    if let Some(s) = first.as_str() {
                        if let Some(mapped) = ids.get(s) {
                            *first = Value::from(mapped.clone());
                        }
                    }
                }
            }
        }
    }
    ids
}

fn set_media(
    graph: &mut Map<String, Value>,
    input: &Fragment,
    ids: &BTreeMap<String, String>,
    asset: &str,
) -> Result<()> {
    for t in input.bindings.get("filename").into_iter().flatten() {
        write_at(graph, &rename_path(t, ids), Value::from(asset.to_string()))?;
    }
    Ok(())
}

/// 输入片段对外的端口。图片/视频给 IMAGE，音频给 AUDIO。
fn port_of(input: &Fragment, _role: &str, kind: Medium) -> Result<Value> {
    let key = match kind {
        Medium::Audio => "audio",
        Medium::Image | Medium::Video => "image",
    };
    input
        .input_outputs
        .get(key)
        .cloned()
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: format!("输入片段「{}」没有 {key} 输出端口", input.id),
        })
}

/// 把片段元数据里的端口（`["load", 0]`）按改名映射解析成图里的真实连线。
fn resolve_port(ids: &BTreeMap<String, String>, port: &Value) -> Value {
    let Some(arr) = port.as_array() else {
        return port.clone();
    };
    let mut out = arr.clone();
    if let Some(first) = out.first_mut() {
        if let Some(s) = first.as_str() {
            if let Some(mapped) = ids.get(s) {
                *first = Value::from(mapped.clone());
            }
        }
    }
    Value::Array(out)
}

fn chain_path(frag: &Fragment, key: &str, ids: &BTreeMap<String, String>) -> Result<String> {
    frag.chain
        .get(key)
        .and_then(|v| v.as_str())
        .map(|p| rename_path(p, ids))
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: format!("guide 片段「{}」没有 {key}", frag.id),
        })
}

/// `load.inputs.image` → `ref1_load.inputs.image`
fn rename_path(path: &str, ids: &BTreeMap<String, String>) -> String {
    let mut parts = path.splitn(2, '.');
    let (Some(node), Some(rest)) = (parts.next(), parts.next()) else {
        return path.to_string();
    };
    match ids.get(node) {
        Some(mapped) => format!("{mapped}.{rest}"),
        None => path.to_string(),
    }
}

fn bump(counters: &mut BTreeMap<String, usize>, key: &str) -> usize {
    let e = counters.entry(key.to_string()).or_insert(0);
    *e += 1;
    *e
}

fn push_autogrow(
    graph: &mut Map<String, Value>,
    slot: &AutogrowSlot,
    n: usize,
    wire: Value,
) -> Result<()> {
    if n > slot.max {
        return Err(StudioError::SchemaViolation {
            stage: crate::stage::StageId::PromptPack,
            violations: vec![Violation::new(
                slot.target.clone(),
                format!("这个槽位最多挂 {} 个，第 {n} 个放不下", slot.max),
            )],
        });
    }
    let (node, field) = split_target(&slot.target)?;
    let obj = graph
        .get_mut(&node)
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|i| i.as_object_mut())
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: format!("图里没有节点 {node} 或它没有 inputs"),
        })?;
    let slot_obj = obj
        .entry(field)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: format!("{} 不是对象，AUTOGROW 槽位必须是对象", slot.target),
        })?;
    slot_obj.insert(format!("{}{n}", slot.prefix), wire);
    Ok(())
}

/// `<节点id>.inputs.<输入名>` 拆成节点与输入名。
///
/// **输入名本身可以带点。** ComfyUI 的动态组合框（`COMFY_DYNAMICCOMBO_V3`）
/// 在 API 格式里是平铺的点号兄弟键——`ResizeImageMaskNode` 选了
/// `scale dimensions` 之后，宽高就叫 `resize_type.width` / `resize_type.height`。
/// 所以第三段之后的部分要原样接回去当输入名，不能判成「层级过深」。
///
/// 片段库（本模块）和整图基线（`studio-pipeline` 的 `Workflow`）都走这一份，
/// 不各写一份——同一条规则两处实现，这个项目已经栽过三次。
pub fn split_target(path: &str) -> Result<(String, String)> {
    let mut parts = path.splitn(3, '.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(node), Some("inputs"), Some(field)) if !node.is_empty() && !field.is_empty() => {
            Ok((node.to_string(), field.to_string()))
        }
        _ => Err(StudioError::ModelContractViolation {
            detail: format!("路径 {path} 应当形如 <节点id>.inputs.<输入名>"),
        }),
    }
}

fn write_at(graph: &mut Map<String, Value>, path: &str, value: Value) -> Result<()> {
    let (node, field) = split_target(path)?;
    let inputs = graph
        .get_mut(&node)
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|i| i.as_object_mut())
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: format!("图里没有节点 {node} 或它没有 inputs"),
        })?;
    inputs.insert(field, value);
    Ok(())
}

fn read_at(graph: &Map<String, Value>, path: &str) -> Option<Value> {
    let (node, field) = split_target(path).ok()?;
    graph.get(&node)?.get("inputs")?.get(&field).cloned()
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use serde_json::json;

    pub(crate) fn nodes(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// 一份最小但结构完整的片段库，形状跟真实的
    /// `assets/workflows/minimax_h3/fragments/` 一致。
    pub(crate) fn fragments() -> FragmentSet {
        let mut backbone = Fragment::new(
            "minimax_h3",
            nodes(json!({
                "load_unet": { "class_type": "UNETLoader", "inputs": { "unet_name": "PLACEHOLDER" } },
                "sigmashift": { "class_type": "MiniMaxH3SigmaShift", "inputs": { "model": ["load_unet", 0] } },
                "load_clip": { "class_type": "CLIPLoader", "inputs": {} },
                "vae_video": { "class_type": "VAELoader", "inputs": {} },
                "vae_audio": { "class_type": "VAELoader", "inputs": {} },
                "noise": { "class_type": "RandomNoise", "inputs": {} },
                "scheduler": { "class_type": "BasicScheduler", "inputs": { "scheduler": "PLACEHOLDER", "steps": 20 } },
                "guider": { "class_type": "BasicGuider", "inputs": { "model": ["sigmashift", 0] } },
                "sampler": { "class_type": "SamplerCustomAdvanced", "inputs": { "guider": ["guider", 0] } },
                "create_video": { "class_type": "CreateVideo", "inputs": {} },
                "save_video": { "class_type": "SaveVideo", "inputs": {} }
            })),
        );
        backbone
            .bindings
            .insert("seed".into(), vec!["noise.inputs.noise_seed".into()]);
        backbone
            .bindings
            .insert("fps".into(), vec!["create_video.inputs.fps".into()]);
        backbone.bindings.insert(
            "output_prefix".into(),
            vec!["save_video.inputs.filename_prefix".into()],
        );
        backbone.must_be_filled = vec![
            "guider.inputs.conditioning".into(),
            "sampler.inputs.latent_image".into(),
            "save_video.inputs.filename_prefix".into(),
        ];

        let mut head_ref = Fragment::new(
            "reference",
            nodes(json!({
                "h3_ref": { "class_type": "MiniMaxH3ReferenceToVideo", "inputs": {} }
            })),
        );
        head_ref.backbone_overrides.insert(
            "load_unet.inputs.unet_name".into(),
            json!("minimax_h3_ref2va_int8_convrot.safetensors"),
        );
        head_ref
            .backbone_overrides
            .insert("scheduler.inputs.scheduler".into(), json!("beta"));
        head_ref
            .outputs
            .insert("conditioning".into(), json!(["h3_ref", 0]));
        head_ref
            .outputs
            .insert("latent".into(), json!(["h3_ref", 1]));
        head_ref
            .wires_from_backbone
            .insert("h3_ref.inputs.clip".into(), json!(["load_clip", 0]));
        head_ref
            .bindings
            .insert("positive".into(), vec!["h3_ref.inputs.prompt".into()]);
        head_ref
            .bindings
            .insert("width".into(), vec!["h3_ref.inputs.width".into()]);
        head_ref
            .bindings
            .insert("height".into(), vec!["h3_ref.inputs.height".into()]);
        head_ref
            .bindings
            .insert("length_frames".into(), vec!["h3_ref.inputs.length".into()]);
        head_ref.autogrow.insert(
            "image".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_images".into(),
                prefix: "ref_image_".into(),
                max: 9,
                verified: true,
                unverified_reason: None,
            },
        );
        head_ref.autogrow.insert(
            "video".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_videos".into(),
                prefix: "ref_video_".into(),
                max: 3,
                verified: true,
                unverified_reason: None,
            },
        );
        head_ref.autogrow.insert(
            "video_audio".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_video_audios".into(),
                prefix: "ref_video_audio_".into(),
                max: 3,
                verified: true,
                unverified_reason: None,
            },
        );

        let mut head_img = Fragment::new(
            "image",
            nodes(json!({ "h3_i2v": { "class_type": "MiniMaxH3ImageToVideo", "inputs": {} } })),
        );
        head_img.backbone_overrides.insert(
            "load_unet.inputs.unet_name".into(),
            json!("minimax_h3_fl2va_int8_convrot.safetensors"),
        );
        head_img
            .backbone_overrides
            .insert("scheduler.inputs.scheduler".into(), json!("simple"));
        head_img
            .outputs
            .insert("conditioning".into(), json!(["h3_i2v", 0]));
        head_img
            .outputs
            .insert("latent".into(), json!(["h3_i2v", 1]));
        head_img
            .bindings
            .insert("positive".into(), vec!["h3_i2v.inputs.prompt".into()]);
        head_img
            .bindings
            .insert("width".into(), vec!["h3_i2v.inputs.width".into()]);
        head_img
            .bindings
            .insert("height".into(), vec!["h3_i2v.inputs.height".into()]);
        head_img
            .bindings
            .insert("length_frames".into(), vec!["h3_i2v.inputs.length".into()]);
        head_img
            .frames
            .insert("first".into(), "h3_i2v.inputs.first_frame".into());
        head_img
            .frames
            .insert("last".into(), "h3_i2v.inputs.last_frame".into());

        let mut guide_img = Fragment::new(
            "image",
            nodes(json!({ "add_guide": { "class_type": "MiniMaxH3AddGuide", "inputs": {} } })),
        );
        guide_img.media_input = Some("add_guide.inputs.image".into());
        guide_img
            .bindings
            .insert("at_frame".into(), vec!["add_guide.inputs.frame_idx".into()]);
        guide_img
            .chain
            .insert("positive_in".into(), json!("add_guide.inputs.positive"));
        guide_img
            .chain
            .insert("latent_in".into(), json!("add_guide.inputs.latent"));
        guide_img
            .chain
            .insert("positive_out".into(), json!(["add_guide", 0]));

        let mut input_img = Fragment::new(
            "image",
            nodes(json!({ "load": { "class_type": "LoadImage", "inputs": {} } })),
        );
        input_img
            .bindings
            .insert("filename".into(), vec!["load.inputs.image".into()]);
        input_img
            .input_outputs
            .insert("image".into(), json!(["load", 0]));

        // 视频输入有两个节点，用来验证片段内部相互引用会跟着改名。
        let mut input_video = Fragment::new(
            "video",
            nodes(json!({
                "load": { "class_type": "LoadVideo", "inputs": {} },
                "split": { "class_type": "GetVideoComponents", "inputs": { "video": ["load", 0] } }
            })),
        );
        input_video
            .bindings
            .insert("filename".into(), vec!["load.inputs.file".into()]);
        input_video
            .input_outputs
            .insert("image".into(), json!(["split", 0]));
        input_video
            .input_outputs
            .insert("audio".into(), json!(["split", 1]));

        // clip 类 guide 跟 image 类共用 AddGuide 节点，区别只在素材是帧序列
        // 而不是单张图——`ref_videos` 的元素类型本来就是 IMAGE。
        let mut guide_clip = guide_img.clone();
        guide_clip.id = "clip".into();

        // preview 的 turbo 叠加层：LoRA 插在 load_unet 与 sigmashift 之间，
        // steps 与调度器都跟着 LoRA 走。
        let mut overlay_ref = Fragment::new(
            "reference",
            nodes(json!({
                "lora": { "class_type": "LoraLoaderModelOnly",
                          "inputs": { "lora_name": "ref2v_turbo_4step.safetensors",
                                      "strength_model": 1.0 } }
            })),
        );
        overlay_ref
            .wires_from_backbone
            .insert("lora.inputs.model".into(), json!(["load_unet", 0]));
        overlay_ref
            .backbone_overrides
            .insert("sigmashift.inputs.model".into(), json!(["lora", 0]));
        overlay_ref
            .backbone_overrides
            .insert("scheduler.inputs.steps".into(), json!(4));
        overlay_ref
            .backbone_overrides
            .insert("scheduler.inputs.scheduler".into(), json!("simple"));

        FragmentSet {
            backbone: Some(backbone),
            heads: BTreeMap::from([("reference".into(), head_ref), ("image".into(), head_img)]),
            guides: BTreeMap::from([("image".into(), guide_img), ("clip".into(), guide_clip)]),
            inputs: BTreeMap::from([("image".into(), input_img), ("video".into(), input_video)]),
            overlays: BTreeMap::from([("reference".into(), overlay_ref)]),
        }
    }

    pub(crate) fn shot(head: &str) -> ShotDeclaration {
        ShotDeclaration {
            shot_id: "S01".into(),
            head: head.into(),
            positive: "一位女性走过红土球场".into(),
            width: 1344,
            height: 768,
            length_frames: 73,
            fps: 24.0,
            seed: 101,
            references: vec![],
            guides: vec![],
            first_frame: None,
            last_frame: None,
        }
    }

    pub(crate) fn img_ref(asset: &str) -> Reference {
        Reference {
            kind: Medium::Image,
            asset_id: asset.into(),
            with_audio: false,
        }
    }

    /// 1 秒空镜：image head，没有参考也没有 guide。
    #[test]
    fn a_bare_image_shot_wires_head_straight_into_the_guider() {
        let mut s = shot("image");
        s.first_frame = Some("plate.png".into());
        let out = assemble(&fragments(), &s, "media/S01").unwrap();
        let g = &out.graph;

        assert_eq!(g["guider"]["inputs"]["conditioning"], json!(["h3_i2v", 0]));
        assert_eq!(g["sampler"]["inputs"]["latent_image"], json!(["h3_i2v", 1]));
        // 跟着 head 走的配套约束
        assert_eq!(g["scheduler"]["inputs"]["scheduler"], json!("simple"));
        assert_eq!(
            g["load_unet"]["inputs"]["unet_name"],
            json!("minimax_h3_fl2va_int8_convrot.safetensors")
        );
        // 首帧是具名槽位，不是 AUTOGROW
        assert_eq!(
            g["h3_i2v"]["inputs"]["first_frame"],
            json!(["first_frame_load", 0])
        );
        assert_eq!(g["first_frame_load"]["inputs"]["image"], json!("plate.png"));
    }

    /// 接续镜：reference head + 2 参考 + 2 个链式 guide。
    /// 这是整个组装器最要紧的一条——AddGuide 只吐 CONDITIONING，
    /// 所以 latent 一律接 head，只有 positive 串成链。
    #[test]
    fn chained_guides_thread_positive_but_always_take_latent_from_the_head() {
        let mut s = shot("reference");
        s.references = vec![img_ref("C01.front"), img_ref("SC02.key")];
        s.guides = vec![
            Guide {
                kind: GuideKind::Image,
                at_frame: 0,
                asset_id: "S02.tail".into(),
            },
            Guide {
                kind: GuideKind::Image,
                at_frame: -1,
                asset_id: "C01.profile".into(),
            },
        ];
        let out = assemble(&fragments(), &s, "media/S03").unwrap();
        let g = &out.graph;

        // positive 串链：head → guide1 → guide2 → guider
        assert_eq!(
            g["guide1_add_guide"]["inputs"]["positive"],
            json!(["h3_ref", 0])
        );
        assert_eq!(
            g["guide2_add_guide"]["inputs"]["positive"],
            json!(["guide1_add_guide", 0])
        );
        assert_eq!(
            g["guider"]["inputs"]["conditioning"],
            json!(["guide2_add_guide", 0])
        );
        // latent 全部接 head，不串链
        assert_eq!(
            g["guide1_add_guide"]["inputs"]["latent"],
            json!(["h3_ref", 1])
        );
        assert_eq!(
            g["guide2_add_guide"]["inputs"]["latent"],
            json!(["h3_ref", 1])
        );
        assert_eq!(g["sampler"]["inputs"]["latent_image"], json!(["h3_ref", 1]));
        // 负数帧号原样传下去，由模型按「从末尾倒数」解释
        assert_eq!(g["guide2_add_guide"]["inputs"]["frame_idx"], json!(-1));
        // AUTOGROW 序号从 1 起
        assert_eq!(
            g["h3_ref"]["inputs"]["ref_images"],
            json!({ "ref_image_1": ["ref1_load", 0], "ref_image_2": ["ref2_load", 0] })
        );
        assert_eq!(g["scheduler"]["inputs"]["scheduler"], json!("beta"));
    }

    /// preview 的 turbo 组合：LoRA 进图、steps 与调度器跟着 LoRA 走。
    #[test]
    fn the_preview_turbo_combination_swaps_in_the_lora_and_its_steps() {
        let set = fragments();
        let s = shot("reference");
        let out = assemble_as(&set, &s, "media/S01", Combination::PreviewTurbo).unwrap();
        let g = &out.graph;
        assert_eq!(g["lora"]["class_type"], json!("LoraLoaderModelOnly"));
        // LoRA 插在 load_unet 与 sigmashift 之间。
        assert_eq!(g["lora"]["inputs"]["model"], json!(["load_unet", 0]));
        assert_eq!(g["sigmashift"]["inputs"]["model"], json!(["lora", 0]));
        // steps 与调度器都跟着 LoRA 走——低步数下 beta 出来的画面是坏的。
        assert_eq!(g["scheduler"]["inputs"]["steps"], json!(4));
        assert_eq!(g["scheduler"]["inputs"]["scheduler"], json!("simple"));
        assert!(out.used.contains(&"overlay/reference".to_string()));
        assert!(out.notes.is_empty(), "挂上了就不该有降级说明");
    }

    /// 关掉 turbo（正式渲染就是这条路）回到普通组合，图里没有 LoRA。
    #[test]
    fn the_standard_combination_has_no_lora() {
        let out = assemble(&fragments(), &shot("reference"), "media/S01").unwrap();
        assert!(out.graph.get("lora").is_none());
        assert_eq!(out.graph["scheduler"]["inputs"]["steps"], json!(20));
        assert_eq!(out.graph["scheduler"]["inputs"]["scheduler"], json!("beta"));
    }

    /// 叠加层没真机核验时退回普通组合并说明——preview 省时间是锦上添花，
    /// 为它把整个预览阶段卡死不值得，但也不能让人以为跑的是 turbo。
    #[test]
    fn an_unverified_overlay_falls_back_and_says_so() {
        let mut set = fragments();
        let o = set.overlays.get_mut("reference").unwrap();
        o.verified = false;
        o.unavailable_reason = Some("没在真机上出过片".into());
        let out =
            assemble_as(&set, &shot("reference"), "m/S01", Combination::PreviewTurbo).unwrap();
        assert!(out.graph.get("lora").is_none());
        assert_eq!(out.graph["scheduler"]["inputs"]["steps"], json!(20));
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].contains("没在真机上出过片"), "{:?}", out.notes);
    }

    /// 这个 head 压根没有叠加层时同样退回，不是错误。
    #[test]
    fn a_head_without_an_overlay_falls_back_quietly() {
        let mut set = fragments();
        set.overlays.clear();
        let out = assemble_as(&set, &shot("image"), "m/S01", Combination::PreviewTurbo).unwrap();
        assert!(out.graph.get("lora").is_none());
        assert_eq!(out.notes.len(), 1);
        assert!(
            out.notes[0].contains("没有 turbo 叠加层"),
            "{:?}",
            out.notes
        );
    }

    /// `clip` 锚的是**帧序列**，必须走 LoadVideo + GetVideoComponents，
    /// 不能走 LoadImage。两条路的输出都是 IMAGE，图都能过 ComfyUI 的校验，
    /// 但后者只喂进去一张静帧——声明的是接续一段，实际接的是一张图。
    #[test]
    fn a_clip_guide_loads_a_frame_sequence_not_a_still() {
        let mut s = shot("reference");
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S02.tail22".into(),
        }];
        let g = assemble(&fragments(), &s, "media/S03").unwrap().graph;
        assert_eq!(g["guide1_src_load"]["class_type"], json!("LoadVideo"));
        assert_eq!(
            g["guide1_add_guide"]["inputs"]["image"],
            json!(["guide1_src_split", 0]),
            "帧序列要取 GetVideoComponents 的 IMAGE 输出"
        );
        // 对照：image 类 guide 仍然走 LoadImage。
        let mut s2 = shot("reference");
        s2.guides = vec![Guide {
            kind: GuideKind::Image,
            at_frame: 0,
            asset_id: "C01.front".into(),
        }];
        let g2 = assemble(&fragments(), &s2, "media/S03").unwrap().graph;
        assert_eq!(g2["guide1_src_load"]["class_type"], json!("LoadImage"));
    }

    /// 群戏：5 张参考，序号必须连续且不重号。
    #[test]
    fn a_crowd_shot_numbers_every_reference_slot_in_order() {
        let mut s = shot("reference");
        s.references = (0..5).map(|i| img_ref(&format!("A{i}"))).collect();
        let out = assemble(&fragments(), &s, "media/S07").unwrap();
        let slots = out.graph["h3_ref"]["inputs"]["ref_images"]
            .as_object()
            .unwrap();
        assert_eq!(slots.len(), 5);
        for n in 1..=5 {
            assert!(
                slots.contains_key(&format!("ref_image_{n}")),
                "缺 ref_image_{n}"
            );
        }
    }

    /// 同一份声明组装两次必须逐字节相同。否则 retry_stage 的
    /// 「内容没问题，原样重跑」就不成立，落盘的 debug 请求也对不上。
    #[test]
    fn assembling_the_same_declaration_twice_is_byte_identical() {
        let mut s = shot("reference");
        s.references = vec![img_ref("C01"), img_ref("C02"), img_ref("C03")];
        s.guides = vec![Guide {
            kind: GuideKind::Image,
            at_frame: 5,
            asset_id: "G".into(),
        }];
        let a = assemble(&fragments(), &s, "media/S01").unwrap();
        let b = assemble(&fragments(), &s, "media/S01").unwrap();
        assert_eq!(
            serde_json::to_string(&a.graph).unwrap(),
            serde_json::to_string(&b.graph).unwrap()
        );
        assert_eq!(a.used, b.used);
    }

    /// 视频参考带音轨时占同号槽位，且片段内部的相互引用要跟着改名。
    #[test]
    fn a_video_reference_with_audio_takes_the_same_numbered_slot() {
        let mut s = shot("reference");
        s.references = vec![Reference {
            kind: Medium::Video,
            asset_id: "C01.anchor".into(),
            with_audio: true,
        }];
        let out = assemble(&fragments(), &s, "media/S01").unwrap();
        let g = &out.graph;
        // 片段内 split 引用 load，改名后仍要指对
        assert_eq!(g["ref1_split"]["inputs"]["video"], json!(["ref1_load", 0]));
        assert_eq!(
            g["h3_ref"]["inputs"]["ref_videos"],
            json!({ "ref_video_1": ["ref1_split", 0] })
        );
        assert_eq!(
            g["h3_ref"]["inputs"]["ref_video_audios"],
            json!({ "ref_video_audio_1": ["ref1_split", 1] })
        );
    }

    #[test]
    fn exceeding_a_slot_cap_is_a_schema_violation_not_a_silent_drop() {
        let mut s = shot("reference");
        s.references = (0..10).map(|i| img_ref(&format!("A{i}"))).collect();
        let e = assemble(&fragments(), &s, "media/S01").unwrap_err();
        assert_eq!(e.code(), "schema_violation");
        assert!(e.message().contains("最多挂 9"), "{}", e.message());
    }

    #[test]
    fn an_unknown_head_names_what_is_available() {
        let s = shot("teleport");
        let e = assemble(&fragments(), &s, "media/S01").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("image"), "{}", e.message());
        assert!(e.message().contains("reference"), "{}", e.message());
    }

    /// image head 没有 AUTOGROW 槽位，挂参考要当场报错而不是默默丢掉。
    #[test]
    fn an_image_head_refuses_references_instead_of_dropping_them() {
        let mut s = shot("image");
        s.references = vec![img_ref("C01")];
        let e = assemble(&fragments(), &s, "media/S01").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("不接"), "{}", e.message());
    }

    /// 未核验的片段不许用来渲染——跟未核验的整图基线同一套规矩。
    #[test]
    fn an_unverified_fragment_blocks_rendering() {
        let mut set = fragments();
        let v = set.inputs.get_mut("image").unwrap();
        v.verified = false;
        v.unavailable_reason = Some("没在真机上跑通过".into());

        let mut s = shot("reference");
        s.references = vec![img_ref("C01")];
        let e = assemble(&set, &s, "media/S01").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("尚未核验"), "{}", e.message());
        assert!(e.message().contains("没在真机上跑通过"), "{}", e.message());
    }

    /// 骨架留空的三处必须被填上，缺一个都要报错而不是交出半张图。
    #[test]
    fn a_backbone_hole_left_unfilled_is_reported() {
        let mut set = fragments();
        set.backbone
            .as_mut()
            .unwrap()
            .bindings
            .remove("output_prefix");
        let e = assemble(&set, &shot("image"), "media/S01").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("filename_prefix"), "{}", e.message());
    }
}

/// MiniMax H3 的帧数网格：`17k + 5`，k ≥ 0，即 5 / 22 / 39 / 56 / 73…
///
/// 模型会自己 snap 到最近的合法值，所以写个 50 帧下去不报错——但产出的
/// 时长跟提示词包里声明的对不上，而 `post` 拼接是按声明的时长算的。
/// 显式挡下比让它悄悄改掉好。
pub const FRAME_GRID_STEP: i64 = 17;
pub const FRAME_GRID_BASE: i64 = 5;

pub fn is_on_frame_grid(frames: i64) -> bool {
    frames >= FRAME_GRID_BASE && (frames - FRAME_GRID_BASE) % FRAME_GRID_STEP == 0
}

/// 离给定帧数最近的两个合法值，用在报错消息里。
fn nearest_grid(frames: i64) -> (i64, i64) {
    if frames <= FRAME_GRID_BASE {
        return (FRAME_GRID_BASE, FRAME_GRID_BASE + FRAME_GRID_STEP);
    }
    let k = (frames - FRAME_GRID_BASE) / FRAME_GRID_STEP;
    let lo = FRAME_GRID_BASE + k * FRAME_GRID_STEP;
    (lo, lo + FRAME_GRID_STEP)
}

/// 提交 `prompt_pack` 时的组合合法性校验（SPEC-0014 §6 的 V1–V8）。
///
/// 跟组装器分开是有意的：组装器只在渲染时跑，而这些错误应该在**提交那一刻**
/// 就报出来——让 Agent 当场按 remedy 改，而不是等花完 GPU 时间才发现。
///
/// `known_assets` 是 `visual_assets` 登记过的产物 id。传空切片表示跳过 V7
/// （上游阶段还没产出时不该拿这条卡住）。
///
/// `prior_shots` 是本包里排在这一镜**前面**的 shot_id。镜头之间接续靠的是
/// 引用上一镜的尾段（`sh01.tail`），那东西不可能出现在 `visual_assets` 里
/// ——它要等 sh01 渲染完才存在。所以引用有两个来源，V7 查前者，V9 查后者。
pub fn validate_shot(
    set: &FragmentSet,
    shot: &ShotDeclaration,
    known_assets: &[String],
    prior_shots: &[String],
    index: usize,
) -> Vec<Violation> {
    let at = |field: &str| format!("prompt_pack.shots[{index}].{field}");
    let mut v = Vec::new();
    let check_asset = |v: &mut Vec<Violation>, path: String, asset_id: &str| {
        // V9：镜间引用（`sh01.tail`）只能指向本包里更靠前的镜头。
        if let Some(shot_id) = parse_shot_segment(asset_id).map(|s| s.shot_id) {
            if prior_shots.iter().any(|s| s == shot_id) {
                return;
            }
            v.push(Violation::new(
                path,
                format!(
                    "「{asset_id}」指的是 {shot_id} 的片段，而 {shot_id} 不排在这一镜前面\
                     ——它还没渲出来，接不上。镜间引用只能指向本包里更靠前的镜头。"
                ),
            ));
            return;
        }
        // V7：其余的必须是 visual_assets 登记过的产物。
        // 上游还没产出时传空切片，这条跳过，不拿它卡人。
        if known_assets.is_empty() {
            return;
        }
        if let AssetRef::Unknown = classify_asset(asset_id, known_assets) {
            v.push(Violation::new(
                path,
                format!(
                    "visual_assets 里没有「{asset_id}」。可用的：{}。\
                     要接上一镜就写 <上一镜的 shot_id>.tail",
                    preview_list(known_assets)
                ),
            ));
        }
    };

    let Some(head) = set.heads.get(&shot.head) else {
        v.push(Violation::new(
            at("head"),
            format!("没有这个 head。可用的：{}", set.verified_heads().join("、")),
        ));
        return v;
    };

    // V4：帧数吃 17k+5 网格
    if !is_on_frame_grid(shot.length_frames) {
        let (lo, hi) = nearest_grid(shot.length_frames);
        v.push(Violation::new(
            at("length_frames"),
            format!(
                "{} 不在 MiniMax H3 的帧数网格上（17k+5：5 / 22 / 39 / 56 / 73…）。\
                 最近的合法值是 {lo} 或 {hi}。写别的值模型会自己 snap，\
                 于是成片时长跟这里声明的对不上，而 post 拼接按声明算。",
                shot.length_frames
            ),
        ));
    }

    // V8：画幅取 32 的倍数，短边不超过原生画布
    for (name, val) in [("width", shot.width), ("height", shot.height)] {
        if val % 32 != 0 {
            v.push(Violation::new(
                at(name),
                format!("{val} 不是 32 的倍数。ComfyUI 会四舍五入，实际出图尺寸跟这里对不上。"),
            ));
        }
    }
    let short_edge = shot.width.min(shot.height);
    if short_edge > 768 {
        v.push(Violation::new(
            at("width"),
            format!(
                "短边 {short_edge} 超过 MiniMax H3 的原生画布 768。\
                 超出部分不会带来细节，只是更慢；要更高分辨率走 post 的超分。"
            ),
        ));
    }

    // V1 / V2：head 决定能挂什么
    if head.autogrow.is_empty() && !shot.references.is_empty() {
        v.push(Violation::new(
            at("references"),
            format!(
                "head「{}」不接参考（它没有 AUTOGROW 槽位）。要挂参考就换成有槽位的 head。",
                head.id
            ),
        ));
    }
    let mut used: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, r) in shot.references.iter().enumerate() {
        let n = used.entry(r.kind.as_str()).or_insert(0);
        *n += 1;
        match head.autogrow.get(r.kind.as_str()) {
            Some(slot) if !slot.verified => v.push(Violation::new(
                at(&format!("references[{i}].kind")),
                format!(
                    "head「{}」的 {} 类参考槽位尚未核验（{}），现在选不了。",
                    head.id,
                    r.kind.as_str(),
                    slot.unverified_reason.as_deref().unwrap_or("原因未记录")
                ),
            )),
            Some(slot) if *n > slot.max => v.push(Violation::new(
                at(&format!("references[{i}]")),
                format!(
                    "{} 类参考最多 {} 个，这是第 {} 个。",
                    r.kind.as_str(),
                    slot.max,
                    n
                ),
            )),
            Some(_) => {}
            None if !head.autogrow.is_empty() => v.push(Violation::new(
                at(&format!("references[{i}].kind")),
                format!("head「{}」没有 {} 类参考的槽位。", head.id, r.kind.as_str()),
            )),
            None => {}
        }
        // V6：音轨只能跟着视频走
        if r.with_audio && r.kind != Medium::Video {
            v.push(Violation::new(
                at(&format!("references[{i}].with_audio")),
                "只有 video 类参考能带音轨——它占用同号的 ref_video_audio 槽位。",
            ));
        }
        // V7 / V9：引用的资产必须真的存在，或者是更靠前那一镜的片段
        check_asset(
            &mut v,
            at(&format!("references[{i}].asset_id")),
            &r.asset_id,
        );
    }

    // V2：image head 的 guide 只能是首尾两帧
    let frames_only = head.autogrow.is_empty() && !head.frames.is_empty();
    if frames_only && shot.guides.len() > 2 {
        v.push(Violation::new(
            at("guides"),
            format!(
                "head「{}」只有首尾两个帧槽位，挂不了 {} 个 guide。",
                head.id,
                shot.guides.len()
            ),
        ));
    }

    for (j, g) in shot.guides.iter().enumerate() {
        // V3：帧号必须落在这一镜的范围内
        let in_range = g.at_frame >= -shot.length_frames && g.at_frame < shot.length_frames;
        if !in_range {
            v.push(Violation::new(
                at(&format!("guides[{j}].at_frame")),
                format!(
                    "{} 超出这一镜的帧范围。合法区间是 [-{}, {})，负数从末尾倒数。",
                    g.at_frame, shot.length_frames, shot.length_frames
                ),
            ));
        }
        if frames_only && !(g.at_frame == 0 || g.at_frame == -1) {
            v.push(Violation::new(
                at(&format!("guides[{j}].at_frame")),
                format!(
                    "head「{}」只能锚首帧（0）或尾帧（-1），锚不了第 {} 帧。\
                     要在任意帧锚定就换成有 AddGuide 支持的 head。",
                    head.id, g.at_frame
                ),
            ));
        }
        check_asset(&mut v, at(&format!("guides[{j}].asset_id")), &g.asset_id);

        // V5：clip 锚点的长度吃同一套 17k+5 网格，而且**必须短于这一镜**。
        //
        // 后半句是真机跑出来的：22 帧的锚点挂在 22 帧的镜头上，整镜就是锚点
        // 本身，提示词一个字都不生效。等长（或更长）的锚点等于把整镜钉死，
        // 那不是接续，是复制。
        if let Some(seg) = parse_shot_segment(&g.asset_id) {
            if let Some(n) = seg.frames {
                let n = n as i64;
                if !is_on_frame_grid(n) {
                    let (lo, hi) = nearest_grid(n);
                    v.push(Violation::new(
                        at(&format!("guides[{j}].asset_id")),
                        format!(
                            "锚点长度 {n} 帧不在网格上（17k+5：5 / 22 / 39 / 56…）。\
                             最近的合法值是 {lo} 或 {hi}，写成 {}.tail{lo} 这样。",
                            seg.shot_id
                        ),
                    ));
                }
                if n >= shot.length_frames {
                    v.push(Violation::new(
                        at(&format!("guides[{j}].asset_id")),
                        format!(
                            "锚点 {n} 帧不比这一镜的 {} 帧短——等长的锚点会把整镜钉死，\
                             模型只会把它复现出来，提示词一个字都不生效。\
                             接续要的是**开头几帧**跟住上一镜，取短一点（比如 {}.tail5）。",
                            shot.length_frames, seg.shot_id
                        ),
                    ));
                }
            }
        }
    }

    // V2 的另一半：首尾帧是 `head: image` 专用的具名槽位。写在没有这两个
    // 槽位的 head 上会一路走到渲染才炸，那时 GPU 时间已经花出去了。
    for (field, asset) in [
        ("first_frame", &shot.first_frame),
        ("last_frame", &shot.last_frame),
    ] {
        let Some(asset_id) = asset else { continue };
        let slot = field.trim_end_matches("_frame");
        if !head.frames.contains_key(slot) {
            v.push(Violation::new(
                at(field),
                format!(
                    "head「{}」没有 {slot} 帧槽位，{field} 写了不会有任何效果。\
                     要给首尾帧就换成有帧槽位的 head（{}）。",
                    head.id,
                    frames_heads(set)
                ),
            ));
            continue;
        }
        check_asset(&mut v, at(field), asset_id);
    }

    v
}

/// 有具名首尾帧槽位的 head，用在报错消息里给出可换的选项。
fn frames_heads(set: &FragmentSet) -> String {
    let ids: Vec<&str> = set
        .heads
        .values()
        .filter(|h| h.verified && !h.frames.is_empty())
        .map(|h| h.id.as_str())
        .collect();
    if ids.is_empty() {
        "这台机器上没有".to_string()
    } else {
        ids.join("、")
    }
}

/// 一条资产引用在 `visual_assets` 里认不认得出来。
enum AssetRef {
    Registered,
    Unknown,
}

/// `sh01.tail` / `sh01.tail22` / `sh01.head` 拆出来的三段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotSegment<'a> {
    pub shot_id: &'a str,
    /// true 取尾段，false 取首段。
    pub from_tail: bool,
    /// 带了帧数就是一段，没带就是一帧静图。
    pub frames: Option<u32>,
}

/// 镜间引用写成 `<shot_id>.tail` / `<shot_id>.head`（可带帧数，如 `.tail22`）。
/// 它们不在 `visual_assets` 里，也不可能在——上一镜的尾段要等那一镜渲完
/// 才存在。所以先按这个形状认，认不出来再回落到登记过的资产。
///
/// 登记过的资产 id 里也可能有点（`C01.front` 这样的视角），但后缀不是
/// tail/head，认不成镜间引用，会正常落到第二条路上。
///
/// **这条规则只能有一份实现。** 上层解析素材、调度分波用的都是这个函数——
/// 各写一份的话，某一天有人给其中一份加了新后缀，校验就会放行一个解析不出来
/// 的引用，或者反过来。
fn classify_asset(asset_id: &str, known_assets: &[String]) -> AssetRef {
    if known_assets.iter().any(|a| a == asset_id) {
        return AssetRef::Registered;
    }
    AssetRef::Unknown
}

/// 这条引用是不是「上一镜的某一段」，是的话指的是哪一镜。
pub fn parse_shot_segment(asset_id: &str) -> Option<ShotSegment<'_>> {
    let (shot_id, segment) = asset_id.split_once('.')?;
    let digits = segment.trim_start_matches(|c: char| !c.is_ascii_digit());
    let word = &segment[..segment.len() - digits.len()];
    let from_tail = match word.to_ascii_lowercase().as_str() {
        "tail" => true,
        "head" => false,
        _ => return None,
    };
    Some(ShotSegment {
        shot_id,
        from_tail,
        frames: digits.parse().ok(),
    })
}

fn preview_list(items: &[String]) -> String {
    if items.len() <= 8 {
        return items.join("、");
    }
    format!("{}… 等 {} 个", items[..8].join("、"), items.len())
}

#[cfg(test)]
mod validation_tests {
    use super::tests_support::{fragments, img_ref, shot};
    use super::*;

    fn assets() -> Vec<String> {
        ["C01.front", "C01.anchor", "SC02.key", "S02.tail"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn find<'a>(vs: &'a [Violation], needle: &str) -> Option<&'a Violation> {
        vs.iter().find(|v| v.message.contains(needle))
    }

    /// V4：帧数必须落在 17k+5 网格上。模型会自己 snap，于是成片时长跟
    /// 声明的对不上，而 post 拼接按声明算——这是会一路错到交付的那种。
    #[test]
    fn v4_frames_off_the_grid_are_rejected_with_the_nearest_legal_values() {
        assert!(is_on_frame_grid(5) && is_on_frame_grid(22) && is_on_frame_grid(73));
        assert!(!is_on_frame_grid(50) && !is_on_frame_grid(0));

        let mut s = shot("reference");
        s.length_frames = 50;
        let v = validate_shot(&fragments(), &s, &[], &[], 0);
        let hit = find(&v, "帧数网格").expect("应当报帧数网格");
        assert!(
            hit.message.contains("39") && hit.message.contains("56"),
            "{}",
            hit.message
        );
        assert_eq!(hit.path, "prompt_pack.shots[0].length_frames");
    }

    #[test]
    fn v8_dimensions_must_be_multiples_of_32_and_within_the_native_canvas() {
        let mut s = shot("reference");
        s.width = 1000;
        s.height = 900;
        let v = validate_shot(&fragments(), &s, &[], &[], 0);
        assert!(find(&v, "32 的倍数").is_some(), "{v:?}");
        assert!(
            find(&v, "原生画布").is_some(),
            "短边 900 超 768 该报，{v:?}"
        );
    }

    /// V1：超出槽位上限在提交时就挡下，不等到渲染。
    #[test]
    fn v1_too_many_references_are_caught_at_submit_time() {
        let mut s = shot("reference");
        s.references = (0..10).map(|i| img_ref(&format!("A{i}"))).collect();
        let v = validate_shot(&fragments(), &s, &[], &[], 0);
        assert!(find(&v, "最多 9 个").is_some(), "{v:?}");
    }

    /// V2：image head 只有首尾两个帧槽位。
    #[test]
    fn v2_an_image_head_takes_neither_references_nor_arbitrary_frame_guides() {
        let mut s = shot("image");
        s.references = vec![img_ref("C01.front")];
        s.guides = vec![Guide {
            kind: GuideKind::Image,
            at_frame: 30,
            asset_id: "S02.tail".into(),
        }];
        let v = validate_shot(&fragments(), &s, &[], &[], 0);
        assert!(find(&v, "不接参考").is_some(), "{v:?}");
        assert!(find(&v, "只能锚首帧").is_some(), "{v:?}");
    }

    /// V3：帧号要落在这一镜的范围内，负数从末尾倒数所以下界是 -length。
    #[test]
    fn v3_guide_frames_must_fall_inside_the_shot() {
        let mut s = shot("reference");
        s.length_frames = 22;
        s.guides = vec![
            Guide {
                kind: GuideKind::Image,
                at_frame: 22,
                asset_id: "S02.tail".into(),
            },
            Guide {
                kind: GuideKind::Image,
                at_frame: -1,
                asset_id: "S02.tail".into(),
            },
        ];
        let v = validate_shot(&fragments(), &s, &[], &["S02".into()], 0);
        assert_eq!(v.len(), 1, "只有第一个越界：{v:?}");
        assert!(v[0].message.contains("[-22, 22)"), "{}", v[0].message);
    }

    /// V6：音轨只能跟着视频参考走。
    #[test]
    fn v6_only_a_video_reference_may_carry_audio() {
        let mut s = shot("reference");
        s.references = vec![Reference {
            kind: Medium::Image,
            asset_id: "C01.front".into(),
            with_audio: true,
        }];
        let v = validate_shot(&fragments(), &s, &[], &[], 0);
        assert!(find(&v, "只有 video").is_some(), "{v:?}");
    }

    /// V7：引用不存在的资产要当场报，并把可用的列出来。
    #[test]
    fn v7_an_unknown_asset_id_lists_what_is_available() {
        let mut s = shot("reference");
        s.references = vec![img_ref("C99.nope")];
        let v = validate_shot(&fragments(), &s, &assets(), &[], 0);
        let hit = find(&v, "C99.nope").expect("{v:?}");
        assert!(
            hit.message.contains("C01.front"),
            "要列出可用的：{}",
            hit.message
        );
    }

    /// 未核验的 AUTOGROW 槽位要挡下，而且理由要说清是「进去了但模型不理」，
    /// 不是「进不去」——两种错的下一步完全不同。
    #[test]
    fn an_unverified_autogrow_slot_is_refused_with_its_reason() {
        let mut set = fragments();
        let slot = set
            .heads
            .get_mut("reference")
            .unwrap()
            .autogrow
            .get_mut("image")
            .unwrap();
        slot.verified = false;
        slot.unverified_reason = Some("挂上去输出里量不到影响".into());

        let mut s = shot("reference");
        s.references = vec![img_ref("C01")];

        // 提交时就要报，别等到烧 GPU。
        let v = validate_shot(&set, &s, &assets(), &[], 0);
        let hit = find(&v, "尚未核验").expect("{v:?}");
        assert!(hit.message.contains("量不到影响"), "{}", hit.message);

        // 真走到组装也拒绝，不许拼出来。
        let e = assemble(&set, &s, "m/S01").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("量不到影响"), "{}", e.message());
    }

    /// 槽位没写 `verified` 就是验过的——绝大多数是从已验证基线切来的。
    #[test]
    fn a_slot_without_an_explicit_flag_counts_as_verified() {
        let (_, frag) = Fragment::parse(
            r#"{"h": {"class_type":"X","inputs":{}},
                "_studio": {"kind":"head","id":"h","bindings_verified":true,
                  "outputs":{"conditioning":["h",0],"latent":["h",1]},
                  "autogrow":{"image":{"target":"h.inputs.refs","prefix":"r_","max":9}}}}"#,
            "test",
        )
        .unwrap();
        assert!(frag.autogrow["image"].verified);
    }

    /// V5：等长的锚点会把整镜钉死。
    ///
    /// 真机跑出来的：22 帧锚点挂 22 帧镜头，出来的整段就是锚点本身，
    /// 提示词一个字都不生效。那不是接续，是复制。
    #[test]
    fn v5_an_anchor_as_long_as_the_shot_is_rejected() {
        let mut s = shot("reference");
        s.shot_id = "S03".into();
        s.length_frames = 22;
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S02.tail22".into(),
        }];
        let v = validate_shot(&fragments(), &s, &assets(), &["S02".into()], 1);
        let hit = find(&v, "钉死").expect("{v:?}");
        assert!(hit.message.contains("复现"), "{}", hit.message);
        assert!(
            hit.message.contains("tail5"),
            "要给出照抄就能改的写法：{}",
            hit.message
        );
    }

    /// 短锚点是正常的接续，不该报。
    #[test]
    fn v5_a_short_anchor_is_fine() {
        let mut s = shot("reference");
        s.shot_id = "S03".into();
        s.length_frames = 39;
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S02.tail5".into(),
        }];
        assert_eq!(
            validate_shot(&fragments(), &s, &assets(), &["S02".into()], 1),
            vec![]
        );
    }

    /// 锚点长度同样吃 17k+5 网格。
    #[test]
    fn v5_an_anchor_off_the_frame_grid_is_rejected() {
        let mut s = shot("reference");
        s.shot_id = "S03".into();
        s.length_frames = 56;
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S02.tail10".into(),
        }];
        let v = validate_shot(&fragments(), &s, &assets(), &["S02".into()], 1);
        let hit = find(&v, "锚点长度").expect("{v:?}");
        assert!(
            hit.message.contains("5") && hit.message.contains("22"),
            "{}",
            hit.message
        );
    }

    /// V9：接上一镜的尾段是镜头之间接得住的主要手段，而那东西不可能在
    /// `visual_assets` 里——它要等上一镜渲完才存在。所以 `sh01.tail`
    /// 这类引用走另一条路：只要 sh01 排在前面就认。
    #[test]
    fn v9_a_prior_shots_tail_is_a_legal_reference() {
        let mut s = shot("reference");
        s.shot_id = "S03".into();
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S02.tail22".into(),
        }];
        let prior = vec!["S01".to_string(), "S02".to_string()];
        assert_eq!(
            validate_shot(&fragments(), &s, &assets(), &prior, 2),
            vec![]
        );
    }

    /// 反过来：引用还没渲的镜头就是接不上，得当场挡下。
    #[test]
    fn v9_referring_to_a_later_shot_is_rejected() {
        let mut s = shot("reference");
        s.shot_id = "S01".into();
        s.guides = vec![Guide {
            kind: GuideKind::Clip,
            at_frame: 0,
            asset_id: "S05.tail".into(),
        }];
        let v = validate_shot(&fragments(), &s, &assets(), &[], 0);
        let hit = find(&v, "S05").expect("{v:?}");
        assert!(hit.message.contains("还没渲出来"), "{}", hit.message);
        assert!(hit.message.contains("更靠前"), "{}", hit.message);
    }

    /// 镜间引用的解析规则只有这一份实现——校验、调度分波、素材裁切
    /// 用的都是它。各写一份的话，某天有人给其中一份加了新后缀，
    /// 校验就会放行一个解析不出来的引用，或者反过来。
    #[test]
    fn the_shot_segment_rule_parses_every_documented_form() {
        let bare = parse_shot_segment("sh01.tail").unwrap();
        assert_eq!(bare.shot_id, "sh01");
        assert!(bare.from_tail);
        assert_eq!(bare.frames, None, "不带帧数 = 一帧静图");

        let clip = parse_shot_segment("S02.tail22").unwrap();
        assert_eq!(clip.shot_id, "S02");
        assert!(clip.from_tail);
        assert_eq!(clip.frames, Some(22), "带帧数 = 一段");

        let head = parse_shot_segment("S02.head5").unwrap();
        assert!(!head.from_tail);
        assert_eq!(head.frames, Some(5));

        // 视角 id 认不成镜间引用，会落到「登记过的资产」那条路上。
        for not_a_segment in ["C01.front", "C01", "SC02.key_angle", "sh01.", "sh01.tails"] {
            assert!(
                parse_shot_segment(not_a_segment).is_none(),
                "{not_a_segment} 不该被认成镜间引用"
            );
        }
    }

    /// 资产 id 里带点的（`C01.front` 这种视角）不该被误认成镜间引用。
    #[test]
    fn v9_does_not_swallow_asset_ids_that_merely_contain_a_dot() {
        let mut s = shot("reference");
        s.references = vec![img_ref("C01.front")];
        assert_eq!(validate_shot(&fragments(), &s, &assets(), &[], 0), vec![]);
    }

    /// 首尾帧写在没有帧槽位的 head 上要当场挡下——不挡就一路走到渲染才炸，
    /// 那时 GPU 时间已经花出去了。
    #[test]
    fn v2_a_frame_slot_on_a_head_without_one_is_rejected() {
        let mut s = shot("reference");
        s.first_frame = Some("C01.front".into());
        let v = validate_shot(&fragments(), &s, &assets(), &[], 0);
        let hit = find(&v, "first_frame").expect("{v:?}");
        assert!(hit.message.contains("没有 first 帧槽位"), "{}", hit.message);
        assert!(
            hit.message.contains("image"),
            "要给出可换的 head：{}",
            hit.message
        );
    }

    /// 有槽位时首尾帧照样要查资产存不存在——以前这两个字段完全没走校验。
    #[test]
    fn v7_covers_the_frame_slots_too() {
        let mut s = shot("image");
        s.first_frame = Some("C99.nope".into());
        s.last_frame = Some("C01.front".into());
        let v = validate_shot(&fragments(), &s, &assets(), &[], 0);
        assert_eq!(v.len(), 1, "只有 first_frame 那条不存在：{v:?}");
        assert!(find(&v, "C99.nope").is_some(), "{v:?}");
    }

    /// 首尾帧也能接上一镜——那是 image head 做接续的方式。
    #[test]
    fn v9_a_frame_slot_may_point_at_a_prior_shots_tail() {
        let mut s = shot("image");
        s.shot_id = "S02".into();
        s.first_frame = Some("S01.tail".into());
        assert_eq!(
            validate_shot(&fragments(), &s, &assets(), &["S01".into()], 1),
            vec![]
        );
    }

    /// 上游还没产出时传空清单，V7 不该拿这条卡住。
    #[test]
    fn v7_is_skipped_when_the_asset_list_is_not_available_yet() {
        let mut s = shot("reference");
        s.references = vec![img_ref("C99.nope")];
        assert!(validate_shot(&fragments(), &s, &[], &[], 0).is_empty());
    }

    /// 一份完全合规的声明不该报任何东西。
    #[test]
    fn a_valid_shot_produces_no_violations() {
        let mut s = shot("reference");
        s.width = 1344;
        s.height = 768;
        s.length_frames = 73;
        s.references = vec![img_ref("C01.front"), img_ref("SC02.key")];
        s.guides = vec![Guide {
            kind: GuideKind::Image,
            at_frame: 0,
            asset_id: "S02.tail".into(),
        }];
        assert_eq!(
            validate_shot(&fragments(), &s, &assets(), &["S02".into()], 0),
            vec![]
        );
    }

    /// 每条违规都要有可定位的路径，否则 Agent 不知道改哪。
    #[test]
    fn every_violation_carries_an_addressable_path() {
        let mut s = shot("reference");
        s.length_frames = 50;
        s.width = 1000;
        s.references = vec![Reference {
            kind: Medium::Image,
            asset_id: "nope".into(),
            with_audio: true,
        }];
        let v = validate_shot(&fragments(), &s, &assets(), &[], 3);
        assert!(v.len() >= 3, "{v:?}");
        for one in &v {
            assert!(
                one.path.starts_with("prompt_pack.shots[3]."),
                "路径要能定位到具体镜头与字段：{}",
                one.path
            );
        }
    }
}

#[cfg(test)]
mod golden_sample_tests {
    use super::*;
    use crate::{fixtures, StageId};

    /// 黄金样例是给 Agent 看的范本——它自己必须是一份合规的声明。
    /// 样例违规而校验不报，等于教 Agent 写错的东西。
    #[test]
    fn the_golden_prompt_pack_satisfies_every_validator() {
        let pack = fixtures::outputs(StageId::PromptPack);
        let shots = pack["prompt_pack"]["shots"].as_array().unwrap();
        assert_eq!(shots.len(), 5);

        let set = tests_support::fragments();
        for (i, raw) in shots.iter().enumerate() {
            let shot: ShotDeclaration = serde_json::from_value(raw.clone())
                .unwrap_or_else(|e| panic!("样例第 {i} 镜不是合法声明：{e}\n{raw}"));

            // 帧数必须落在 17k+5 网格上，否则模型 snap 之后时长跟声明对不上，
            // 而 post 拼接按声明算——这种错会一路错到交付。
            assert!(
                is_on_frame_grid(shot.length_frames),
                "样例 {} 的 {} 帧不在网格上",
                shot.shot_id,
                shot.length_frames
            );

            let assets: Vec<String> = shot
                .references
                .iter()
                .map(|r| r.asset_id.clone())
                .chain(shot.guides.iter().map(|g| g.asset_id.clone()))
                .collect();
            let prior: Vec<String> = shots[..i]
                .iter()
                .map(|s| s["shot_id"].as_str().unwrap().to_string())
                .collect();
            let v = validate_shot(&set, &shot, &assets, &prior, i);
            assert_eq!(v, vec![], "样例 {} 不合规", shot.shot_id);
        }
    }

    /// 样例得真的能组装出图，否则它示范的是一份拼不起来的声明。
    #[test]
    fn every_golden_shot_assembles_into_a_graph() {
        let pack = fixtures::outputs(StageId::PromptPack);
        let set = tests_support::fragments();
        for raw in pack["prompt_pack"]["shots"].as_array().unwrap() {
            let shot: ShotDeclaration = serde_json::from_value(raw.clone()).unwrap();
            let out = assemble(&set, &shot, &format!("media/{}", shot.shot_id))
                .unwrap_or_else(|e| panic!("样例 {} 组装失败：{}", shot.shot_id, e.message()));
            // 三处必填的位置都得填上
            for path in ["guider", "sampler", "save_video"] {
                assert!(out.graph.get(path).is_some(), "组装结果缺 {path}");
            }
        }
    }
}
