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
}

/// 一个模型系列的全部片段。上层负责把它填出来（读文件或测试里手写）。
#[derive(Debug, Clone, Default)]
pub struct FragmentSet {
    pub backbone: Option<Fragment>,
    /// head id → 片段。
    pub heads: BTreeMap<String, Fragment>,
    /// guide kind → 片段。
    pub guides: BTreeMap<String, Fragment>,
    /// input 介质 → 片段。
    pub inputs: BTreeMap<String, Fragment>,
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
}

/// 组装出来的节点图，外加一份可读的组装记录。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledGraph {
    pub graph: Value,
    /// 这张图用了哪些片段，按顺序。留痕用，出问题时能看出是怎么拼的。
    pub used: Vec<String>,
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
            // `clip` 走的也是图片输入：ref_videos 的元素类型就是 IMAGE（帧序列）。
            GuideKind::Image | GuideKind::Clip => Medium::Image,
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

fn split_target(path: &str) -> Result<(String, String)> {
    let mut parts = path.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(node), Some("inputs"), Some(field), None) => {
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
mod tests {
    use super::*;
    use serde_json::json;

    fn nodes(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// 一份最小但结构完整的片段库，形状跟真实的
    /// `assets/workflows/minimax_h3/fragments/` 一致。
    fn fragments() -> FragmentSet {
        let mut backbone = Fragment::new(
            "minimax_h3",
            nodes(json!({
                "load_unet": { "class_type": "UNETLoader", "inputs": { "unet_name": "PLACEHOLDER" } },
                "sigmashift": { "class_type": "MiniMaxH3SigmaShift", "inputs": { "model": ["load_unet", 0] } },
                "load_clip": { "class_type": "CLIPLoader", "inputs": {} },
                "vae_video": { "class_type": "VAELoader", "inputs": {} },
                "vae_audio": { "class_type": "VAELoader", "inputs": {} },
                "noise": { "class_type": "RandomNoise", "inputs": {} },
                "scheduler": { "class_type": "BasicScheduler", "inputs": { "scheduler": "PLACEHOLDER" } },
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
        head_ref.autogrow.insert(
            "image".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_images".into(),
                prefix: "ref_image_".into(),
                max: 9,
            },
        );
        head_ref.autogrow.insert(
            "video".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_videos".into(),
                prefix: "ref_video_".into(),
                max: 3,
            },
        );
        head_ref.autogrow.insert(
            "video_audio".into(),
            AutogrowSlot {
                target: "h3_ref.inputs.ref_video_audios".into(),
                prefix: "ref_video_audio_".into(),
                max: 3,
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

        FragmentSet {
            backbone: Some(backbone),
            heads: BTreeMap::from([("reference".into(), head_ref), ("image".into(), head_img)]),
            guides: BTreeMap::from([("image".into(), guide_img)]),
            inputs: BTreeMap::from([("image".into(), input_img), ("video".into(), input_video)]),
        }
    }

    fn shot(head: &str) -> ShotDeclaration {
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

    fn img_ref(asset: &str) -> Reference {
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
