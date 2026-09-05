//! 三个确定性阶段的实现：渲染、后期、验收。
//!
//! 这一层把控制面的决策变成实际动作：向 ComfyUI 提交、用 ffmpeg 拼接、
//! 用 ffprobe 核对。**运行本程序的机器不需要 GPU**——推理全在 ComfyUI 那侧。

pub mod assets;
pub mod subtitles;
pub mod workflow;

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use studio_comfy::Comfy;
use studio_core::contract::{AnswerOption, Confirmation, SelectionType};
use studio_core::{
    CapabilitySet, Fragment, FragmentSet, Outputs, Result, StageId, StudioError, WorkflowCapability,
};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_media::Media;
use workflow::Workflow;

/// 单镜提交-等待-下载失败后允许重试的次数。重试打到同一个入口——
/// 换后端节点是代理那一侧的事，控制面只负责再试一次。
const MAX_SHOT_ATTEMPTS: u32 = 3;

/// 预览阶段的短边目标像素——只降分辨率，帧数/时长照抄提示词包，不变。
const PREVIEW_SHORT_EDGE: i64 = 480;

/// `render` 和 `preview` 共享同一套提交-等待-下载逻辑，区别只在于目标
/// 分辨率和产物落盘的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerateMode {
    /// 正式尺寸，落在 `media/`。
    Render,
    /// 480p 短边预览，落在 `media/preview/`，不占用后续 `render` 的产物路径。
    Preview,
}

impl GenerateMode {
    fn dest_dir(self) -> &'static str {
        match self {
            GenerateMode::Render => "media",
            GenerateMode::Preview => "media/preview",
        }
    }

    fn debug_dir(self) -> &'static str {
        match self {
            GenerateMode::Render => "debug",
            GenerateMode::Preview => "debug/preview",
        }
    }
}

/// 按短边等比缩放到 `target_short_edge`，长边取整到偶数——多数视频编码器
/// 和扩散模型都要求宽高为偶数。不关心 16 像素对齐：那是模型自己的事
/// （参见 workflow 请求 1080 实际吐出 1072 的记录），预览阶段更不必较真。
fn scale_to_short_edge(width: i64, height: i64, target_short_edge: i64) -> (i64, i64) {
    if width <= 0 || height <= 0 {
        return (target_short_edge, target_short_edge);
    }
    let portrait_or_square = width <= height;
    let (short, long) = if portrait_or_square {
        (width, height)
    } else {
        (height, width)
    };
    let scale = target_short_edge as f64 / short as f64;
    let mut new_long = (long as f64 * scale).round() as i64;
    if new_long % 2 != 0 {
        new_long += 1;
    }
    if portrait_or_square {
        (target_short_edge, new_long)
    } else {
        (new_long, target_short_edge)
    }
}

/// 交付短边。短视频平台的通行规格是 1080×1920，短边就是这个数。
///
/// 模型的原生画布比它小——MiniMax H3 是短边 768——中间那一段由 `post` 的
/// 超分补上。
const DELIVERY_SHORT_EDGE: i64 = 1080;

/// 成片超分用的基线。见 `assets/workflows/seedvr2/SOURCE-README.md`。
const UPSCALE_WORKFLOW: &str = "seedvr2/upscale";

/// 一镜要怎么超。目标尺寸按 ffprobe 实测的源宽高算，不按提示词包里写的。
struct UpscaleJob {
    idx: usize,
    shot_id: String,
    src: PathBuf,
    from: (i64, i64),
    to: (i64, i64),
    seed: i64,
}

/// 这一镜要超到多大。
///
/// `aspect` 是 `brief.aspect_ratio` 里那个字符串。它是自由文本（schema 只
/// 要求是字符串），所以解析不出 `W:H` 时**退回素材自己的宽高比**——那样至少
/// 分辨率是对的，比拿一个猜出来的画幅去裁画面安全。
///
/// 两条规矩：
///
/// - **只放大不缩小**。短边取 `max(DELIVERY_SHORT_EDGE, 源短边)`，
///   已经够大的素材不会被这一步降下去。
/// - 两边都取偶数。H.264 要求如此。
///
/// 顺带修正画幅：MiniMax 的「9:16」画布实际是 768×1344，化简是 4:7；
/// 按 9:16 算出来的目标交给 `ResizeImageMaskNode` 的 `crop=center`
/// 居中裁掉多出来的 1.6% 宽度。
fn delivery_dims(aspect: &str, src_w: i64, src_h: i64) -> (i64, i64) {
    if src_w <= 0 || src_h <= 0 {
        return (DELIVERY_SHORT_EDGE, DELIVERY_SHORT_EDGE);
    }
    let src_short = src_w.min(src_h);
    let short = src_short.max(DELIVERY_SHORT_EDGE);

    // `W:H` 解析得出来才用它，否则按素材自己的比例走。
    let ratio = aspect
        .split_once(':')
        .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
        .filter(|(a, b)| *a > 0.0 && *b > 0.0);
    let (rw, rh) = match ratio {
        Some(v) => v,
        None => return scale_to_short_edge(src_w, src_h, short),
    };

    let (mut w, mut h) = if rw <= rh {
        (short, (short as f64 * rh / rw).round() as i64)
    } else {
        ((short as f64 * rw / rh).round() as i64, short)
    };
    if w % 2 != 0 {
        w += 1;
    }
    if h % 2 != 0 {
        h += 1;
    }
    (w, h)
}

pub struct Pipeline {
    /// 已验证 workflow 基线的所在目录，通常是程序目录下的 `assets/workflows`。
    baselines: PathBuf,
}

impl Pipeline {
    pub fn new(baselines: PathBuf) -> Pipeline {
        Pipeline { baselines }
    }

    /// 从程序目录推出基线目录。
    pub fn from_program_dir(program_dir: Option<&std::path::Path>) -> Pipeline {
        let base = program_dir
            .map(|p| p.join("assets/workflows"))
            .unwrap_or_else(|| PathBuf::from("assets/workflows"));
        Pipeline::new(base)
    }
}

impl StageExecutor for Pipeline {
    fn execute(&self, stage: StageId, ctx: &ExecContext<'_>) -> Result<Outputs> {
        match stage {
            StageId::Preview => self.preview(ctx),
            StageId::Render => self.render(ctx),
            StageId::Post => self.post(ctx),
            StageId::Review => self.review(ctx),
            other => Err(StudioError::internal(format!(
                "{other} 不是确定性阶段，不该走到这里"
            ))),
        }
    }

    /// preview 执行完不直接判过，用这份文案挂起等确认；确认后才轮到
    /// 花钱的正式渲染。revise 一律退回 prompt_pack——preview 自己不产出
    /// 独立内容，问题只可能出在 prompt_pack 决定的内容上。
    fn gate_confirmation(&self, stage: StageId) -> Option<Confirmation> {
        match stage {
            StageId::Preview => Some(Confirmation {
                prompt: "480p 预览已生成，构图与内容是否符合预期？确认后开始正式 1080p 渲染。"
                    .to_string(),
                selection_type: SelectionType::Single,
                options: vec![
                    AnswerOption::new("approve", "预览符合预期，开始正式渲染"),
                    AnswerOption::revise("revise", "预览有问题，退回提示词重新调整"),
                ],
            }),
            _ => None,
        }
    }

    /// 扫一遍基线目录，把每条基线的 `_studio.bindings` 投影成能力面，
    /// 顺带把有 `fragments/` 子目录的系列读成片段库。
    ///
    /// 引擎拿它在提交 `prompt_pack` 时对账。读不出来的基线直接跳过——
    /// 目录本身缺失或损坏是部署问题，会在渲染时以
    /// `model_contract_violation` 报出来，不该在提交阶段变成一堆噪声。
    fn capabilities(&self) -> Option<CapabilitySet> {
        let mut out = Vec::new();
        let mut fragments: BTreeMap<String, FragmentSet> = BTreeMap::new();
        let families = std::fs::read_dir(&self.baselines).ok()?;
        for family in families.flatten() {
            if !family.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let family_name = family.file_name().to_string_lossy().to_string();
            if let Some(set) = load_fragment_set(&family.path().join("fragments")) {
                fragments.insert(family_name.clone(), set);
            }
            let Ok(modes) = std::fs::read_dir(family.path()) else {
                continue;
            };
            for mode in modes.flatten() {
                let path = mode.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                // `SOURCE-*.json` 是随基线一起留档的上游说明，不是基线本身。
                if stem.starts_with("SOURCE-") {
                    continue;
                }
                let name = format!("{family_name}/{stem}");
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(wf) = Workflow::parse(&text, &name) else {
                    continue;
                };
                // 不是给镜头选的基线不进能力面——`seedvr2/upscale` 是 `post`
                // 内部用的，让 Agent 看见它只会让它写进某一镜然后跑出个空文件名。
                if !wf.is_shot_baseline() {
                    continue;
                }
                out.push(WorkflowCapability {
                    params: wf.parameters(),
                    verified: wf.is_verified(),
                    unavailable_reason: wf.unavailable_reason().map(String::from),
                    name,
                });
            }
        }
        if out.is_empty() && fragments.is_empty() {
            return None;
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Some(CapabilitySet::new(out).with_fragments(fragments))
    }
}

/// 读一个系列的 `fragments/` 目录。目录不存在说明这个系列走整图基线，
/// 不是错误。单份片段读不出来就跳过——半份片段库会被
/// [`CapabilitySet::with_fragments`] 挡在门外，不会悄悄生效。
fn load_fragment_set(dir: &std::path::Path) -> Option<FragmentSet> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut set = FragmentSet::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let where_ = path.file_name().map(|n| n.to_string_lossy().to_string());
        let Ok((kind, frag)) = Fragment::parse(&text, where_.as_deref().unwrap_or("?")) else {
            continue;
        };
        set.insert(kind, frag);
    }
    Some(set)
}

/// 把镜头按接续依赖分波：同一波之间没有依赖，可以随便并发；
/// 下一波要等上一波全部出片，因为它得从那些产物里裁尾段。
///
/// 依赖只可能指向更靠前的镜头（`studio-core` 的 V9 在提交时就挡住了环），
/// 所以一遍扫过去按「引用到的最大波次 + 1」定波，不需要拓扑排序。
/// 没有接续引用时全部落在第 0 波，跟以前的行为一模一样。
fn dependency_waves(shots: &[Value]) -> Vec<Vec<usize>> {
    let mut wave_of: BTreeMap<String, usize> = BTreeMap::new();
    let mut assigned: Vec<usize> = Vec::with_capacity(shots.len());

    for shot in shots {
        let mut wave = 0usize;
        let refs = shot["references"].as_array().into_iter().flatten();
        let guides = shot["guides"].as_array().into_iter().flatten();
        // 首尾帧同样可能接上一镜——`head: image` 做接续就是把上一镜的尾帧
        // 填进 first_frame。漏算它，这一镜会跟被接的那一镜排进同一波，
        // 等到解析素材时那一镜还没出片。
        let frames = ["first_frame", "last_frame"]
            .into_iter()
            .filter_map(|k| shot.get(k));
        for asset_id in refs
            .chain(guides)
            .filter_map(|item| item["asset_id"].as_str())
            .chain(frames.filter_map(|v| v.as_str()))
        {
            let Some(seg) = studio_core::assembly::parse_shot_segment(asset_id) else {
                continue;
            };
            // 指向不存在或更靠后的镜头在提交时就被 V9 挡了；真走到这里
            // 只当它没有依赖，让后面的解析报一条说得清的错。
            if let Some(w) = wave_of.get(seg.shot_id) {
                wave = wave.max(w + 1);
            }
        }
        if let Some(id) = shot["shot_id"].as_str() {
            wave_of.insert(id.to_string(), wave);
        }
        assigned.push(wave);
    }

    let count = assigned.iter().max().map(|m| m + 1).unwrap_or(0);
    let mut waves = vec![Vec::new(); count];
    for (idx, w) in assigned.iter().enumerate() {
        waves[*w].push(idx);
    }
    waves
}

/// 取最接近的 `step` 倍数，至少一个 `step`。
fn round_to(v: i64, step: i64) -> i64 {
    ((v + step / 2) / step).max(1) * step
}

/// preview 要覆盖的目标尺寸；render 用提示词包原样的宽高，返回 `None`
/// 表示「不覆盖，结果里也不需要单独报宽高」。
fn preview_dims(shot: &Value, mode: GenerateMode) -> Option<(i64, i64)> {
    if mode != GenerateMode::Preview {
        return None;
    }
    let dim = |k: &str| {
        shot.get(k)
            .and_then(|v| v.as_i64())
            .unwrap_or(PREVIEW_SHORT_EDGE)
    };
    Some(scale_to_short_edge(
        dim("width"),
        dim("height"),
        PREVIEW_SHORT_EDGE,
    ))
}

fn wrap(stage: StageId, v: Value) -> Outputs {
    let mut m = Outputs::new();
    m.insert(stage.output_key().to_string(), v);
    m
}

fn need<'a>(inputs: &'a Value, key: &str, stage: StageId) -> Result<&'a Value> {
    inputs.get(key).ok_or_else(|| StudioError::StageNotReady {
        stage,
        blocked_on: StageId::parse(key).unwrap_or(StageId::PromptPack),
    })
}

impl Pipeline {
    /// 按配置的并发度分片渲染，详见 [`Pipeline::generate`]。
    fn render(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
        let results = self.generate(ctx, StageId::Render, GenerateMode::Render)?;
        Ok(wrap(StageId::Render, json!({ "shots": results })))
    }

    /// 480p 短边预览：跟 `render` 共享同一套提交-等待-下载与并发逻辑，
    /// 只把目标分辨率换成短边 480、产物落到 `media/preview/`。
    /// 帧数、帧率、提示词照抄提示词包，不变——便宜的只是分辨率。
    fn preview(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
        let results = self.generate(ctx, StageId::Preview, GenerateMode::Preview)?;
        Ok(wrap(StageId::Preview, json!({ "shots": results })))
    }

    /// 按配置的并发度分片生成：起 `comfy.concurrency` 个 worker 线程，
    /// 各镜头放共享队列，谁先跑完手上那镜就去认领下一镜。
    ///
    /// 入口只有一个 URL，**具体落到哪个后端节点由代理决定**——控制面既看不见
    /// 也不该管。并发度按代理后面实际的节点数配（`COMFY_CONCURRENCY`）。
    ///
    /// 实测单镜十来分钟，串行渲染 8 镜可能超过一个半小时；8 路并发理论上
    /// 十几分钟就能全部跑完。产出仍按镜头在提示词包里的原始顺序落回
    /// `shots` 数组——`post` 阶段拼接靠的是这个顺序，不是谁先完工。
    ///
    /// **接续镜要排队**：引用了 `sh01.tail` 的镜头得等 sh01 渲完才有东西可裁，
    /// 所以先按依赖分波（[`dependency_waves`]），波内并发、波与波串行。
    /// 没有接续引用时只有一波，跟以前完全一样。
    fn generate(
        &self,
        ctx: &ExecContext<'_>,
        stage: StageId,
        mode: GenerateMode,
    ) -> Result<Vec<Value>> {
        let pack = need(&ctx.inputs, "prompt_pack", stage)?;
        let shots = pack["shots"]
            .as_array()
            .ok_or_else(|| StudioError::internal("提示词包里没有 shots"))?;

        let comfy = Comfy::from_settings(ctx.settings);
        comfy.ensure_reachable()?;

        let total = shots.len();
        let waves = dependency_waves(shots);
        let plan = ctx
            .inputs
            .get(StageId::VisualAssets.output_key())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let results: Mutex<Vec<Option<Value>>> = Mutex::new(vec![None; total]);
        let mut rendered: BTreeMap<String, assets::RenderedShot> = BTreeMap::new();

        if waves.len() > 1 {
            ctx.say(format!(
                "{total} 个镜头分 {} 波跑：接续镜要等它引用的那一镜先出片",
                waves.len()
            ));
        }

        for wave in waves {
            if ctx.is_cancelled() {
                return Err(StudioError::internal("渲染被中断"));
            }
            // 每一波都拿上一波的产物重建解析器——接续镜要从那里裁尾段。
            let resolver = assets::AssetResolver::new(
                ctx.bundle,
                ctx.settings,
                plan.clone(),
                rendered.clone(),
            );
            let queue: Mutex<VecDeque<usize>> = Mutex::new(wave.iter().copied().collect());
            let failure: Mutex<Option<StudioError>> = Mutex::new(None);
            let worker_count = ctx.settings.comfy_concurrency().min(wave.len().max(1));

            std::thread::scope(|scope| {
                for _ in 0..worker_count {
                    let queue = &queue;
                    let results = &results;
                    let failure = &failure;
                    let comfy = &comfy;
                    let resolver = &resolver;
                    scope.spawn(move || loop {
                        if ctx.is_cancelled() || failure.lock().unwrap().is_some() {
                            return;
                        }
                        let idx = queue.lock().unwrap().pop_front();
                        let Some(idx) = idx else { return };
                        let shot = &shots[idx];
                        match self.generate_shot(ctx, comfy, resolver, idx, total, shot, mode) {
                            Ok(v) => results.lock().unwrap()[idx] = Some(v),
                            Err(e) => {
                                let mut f = failure.lock().unwrap();
                                if f.is_none() {
                                    *f = Some(e);
                                }
                                return;
                            }
                        }
                    });
                }
            });

            if let Some(e) = failure.into_inner().unwrap() {
                return Err(e);
            }
            // 这一波的产物进登记表，供下一波的接续镜裁尾段。
            let done = results.lock().unwrap();
            for idx in wave {
                let Some(v) = done[idx].as_ref() else {
                    continue;
                };
                let (Some(id), Some(path)) = (v["shot_id"].as_str(), v["path"].as_str()) else {
                    continue;
                };
                rendered.insert(
                    id.to_string(),
                    assets::RenderedShot {
                        path: path.to_string(),
                        duration_seconds: v["duration_seconds"].as_f64().unwrap_or(0.0),
                        fps: shots[idx]["fps"].as_f64().unwrap_or(24.0),
                    },
                );
            }
        }

        if ctx.is_cancelled() {
            return Err(StudioError::internal("渲染被中断"));
        }
        Ok(results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|v| v.expect("每一波排空后该波的下标都有结果，否则已提前返回错误"))
            .collect())
    }

    /// 一个镜头的提交-等待-下载，失败时最多重试 [`MAX_SHOT_ATTEMPTS`] 次。
    /// 重试打到同一个入口——换后端节点是代理那一侧的事，控制面只负责再试一次。
    #[allow(clippy::too_many_arguments)]
    fn generate_shot(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        resolver: &assets::AssetResolver<'_>,
        idx: usize,
        total: usize,
        shot: &Value,
        mode: GenerateMode,
    ) -> Result<Value> {
        let shot_id = shot["shot_id"].as_str().unwrap_or("shot").to_string();

        let dims = preview_dims(shot, mode);

        // 两种形状：片段化的系列写 head，现场组装；其余系列写 workflow，
        // 加载整图基线再填参数。哪一种由提示词包自己说了算。
        let graph = if shot.get("head").is_some() {
            self.assemble_shot(ctx, comfy, resolver, &shot_id, shot, mode)?
        } else {
            let wf_name =
                shot["workflow"]
                    .as_str()
                    .ok_or_else(|| StudioError::ModelContractViolation {
                        detail: format!("{shot_id} 既没写 head 也没写 workflow，不知道该怎么出图"),
                    })?;
            let wf = ctx
                .step("load_baseline")
                .shot(&shot_id)
                .with("workflow", json!(wf_name))
                .done(Workflow::load(&self.baselines, wf_name).and_then(|w| {
                    w.require_verified()?;
                    Ok(w)
                }))?;
            let mut params = Map::new();
            if let Some(o) = shot.as_object() {
                for (k, v) in o {
                    params.insert(k.clone(), v.clone());
                }
            }
            if let Some((pw, ph)) = dims {
                params.insert("width".to_string(), json!(pw));
                params.insert("height".to_string(), json!(ph));
            }
            wf.apply(&params)?
        };

        // 落一份可以直接 curl 复现的请求体：节点故障时不用整套跑起来就能单独调试
        // 这一镜——`curl -X POST <node>/prompt -H "Content-Type: application/json"
        // --data @<bundle>/<debug_dir>/<shot_id>.request.json`。写失败不影响主流程。
        let debug_rel = format!("{}/{shot_id}.request.json", mode.debug_dir());
        if let Ok(body) = serde_json::to_string_pretty(&json!({
            "prompt": graph,
            "client_id": "video-studio"
        })) {
            let _ = ctx.bundle.write(&debug_rel, &format!("{body}\n"));
        }

        let mut last_err = None;
        for attempt in 1..=MAX_SHOT_ATTEMPTS {
            if ctx.is_cancelled() {
                return Err(StudioError::internal("渲染被中断"));
            }
            match self.generate_shot_once(
                ctx, comfy, idx, total, &shot_id, &graph, &debug_rel, shot, mode, dims,
            ) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if attempt < MAX_SHOT_ATTEMPTS {
                        ctx.say(format!(
                            "{}/{total} {shot_id} 第 {attempt} 次尝试失败（{}），重试中",
                            idx + 1,
                            e.message()
                        ));
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| StudioError::internal(format!("{shot_id} 渲染重试耗尽"))))
    }

    /// 片段化的系列：把声明翻译成节点图。
    ///
    /// 素材要先落到 ComfyUI 那侧才能在图里引用，所以顺序是
    /// **解析 → 上传 → 把声明里的 asset_id 换成上传后的文件名 → 组装**。
    /// 组装器只认名字，不关心素材从哪来。
    fn assemble_shot(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        resolver: &assets::AssetResolver<'_>,
        shot_id: &str,
        shot: &Value,
        mode: GenerateMode,
    ) -> Result<Value> {
        let mut decl: studio_core::ShotDeclaration =
            serde_json::from_value(shot.clone()).map_err(|e| {
                StudioError::ModelContractViolation {
                    detail: format!("{shot_id} 的声明读不出来：{e}"),
                }
            })?;
        if let Some((pw, ph)) = preview_dims(shot, mode) {
            // 片段化的系列要求画幅是 32 的倍数（V8）。短边缩放算出来的
            // 长边多半不是——`768x1344` 缩到 480 得到 840，不是 32 的倍数。
            // 不修就等于我们自己写进 remedy 的那句话：ComfyUI 会四舍五入，
            // 实际出图尺寸跟登记的对不上。
            decl.width = round_to(pw, 32);
            decl.height = round_to(ph, 32);
        }

        let family = ctx.inputs[StageId::VisualAssets.output_key()]["core_model_family"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let set =
            self.fragment_set(&family)
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!(
                        "{shot_id} 用的是声明式形状，但这台机器上没有 {family} 的片段库。\
                     片段库在 assets/workflows/{family}/fragments/ 下，\
                     缺骨架或没有已核验的 head 都会让它整个不可用"
                    ),
                })?;

        // 参考、锚点、首尾帧全都要上传。同一份素材多镜共用时只传一次。
        let mut ids: Vec<String> = decl.references.iter().map(|r| r.asset_id.clone()).collect();
        ids.extend(decl.guides.iter().map(|g| g.asset_id.clone()));
        ids.extend(decl.first_frame.clone());
        ids.extend(decl.last_frame.clone());
        let mut remote: BTreeMap<String, String> = BTreeMap::new();
        for id in ids {
            if remote.contains_key(&id) {
                continue;
            }
            let name = ctx
                .step("upload_asset")
                .shot(shot_id)
                .with("asset_id", json!(id))
                .done(resolver.upload(comfy, &id))?;
            remote.insert(id, name);
        }
        let rename = |id: &String| remote.get(id).cloned().unwrap_or_else(|| id.clone());
        for r in &mut decl.references {
            r.asset_id = rename(&r.asset_id);
        }
        for g in &mut decl.guides {
            g.asset_id = rename(&g.asset_id);
        }
        decl.first_frame = decl.first_frame.as_ref().map(rename);
        decl.last_frame = decl.last_frame.as_ref().map(rename);

        // preview 换一套更便宜的组合：挂 head 配套的 turbo LoRA、steps 降到
        // LoRA 的步数。预览门要看的只是构图与内容。叠加层没核验时组装器会
        // 自己退回普通组合，并在 notes 里说明——不会悄悄跑出不可信的东西。
        let combination = match (mode, ctx.settings.comfy_preview_turbo()) {
            (GenerateMode::Preview, true) => studio_core::assembly::Combination::PreviewTurbo,
            _ => studio_core::assembly::Combination::Standard,
        };

        let out = ctx
            .step("assemble")
            .shot(shot_id)
            .with("head", json!(decl.head))
            .with("combination", json!(format!("{combination:?}")))
            .done(studio_core::assembly::assemble_as(
                &set,
                &decl,
                &format!("studio/{shot_id}"),
                combination,
            ))?;
        for note in &out.notes {
            ctx.say(format!("{shot_id}：{note}"));
        }
        ctx.step("assembled")
            .shot(shot_id)
            .with("fragments", json!(out.used))
            .with("notes", json!(out.notes))
            .done(Ok::<(), StudioError>(()))?;
        Ok(out.graph)
    }

    /// 这个系列的片段库，没有就说明它走整图基线。
    fn fragment_set(&self, family: &str) -> Option<FragmentSet> {
        let set = load_fragment_set(&self.baselines.join(family).join("fragments"))?;
        set.is_usable().then_some(set)
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_shot_once(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        idx: usize,
        total: usize,
        shot_id: &str,
        graph: &Value,
        debug_rel: &str,
        shot: &Value,
        mode: GenerateMode,
        dims: Option<(i64, i64)>,
    ) -> Result<Value> {
        let node = comfy.node();
        let sub = ctx
            .progress_and_step(format!("{}/{total} {shot_id} 提交", idx + 1), "submit")
            .shot(shot_id)
            .node(node)
            .with("debug_request", json!(debug_rel))
            .done(comfy.submit(graph, "video-studio"))?;

        ctx.say(format!(
            "{}/{total} {shot_id} 渲染中（{}）",
            idx + 1,
            sub.prompt_id
        ));
        let files = comfy.wait(&sub)?;
        let first = files.first().ok_or_else(|| StudioError::ComfyFailed {
            node: node.to_string(),
            detail: format!("{shot_id} 执行完成但没有产出文件"),
        })?;

        let ext = std::path::Path::new(&first.filename)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "mp4".into());
        let rel = format!("{}/{shot_id}.{ext}", mode.dest_dir());
        let dest = ctx.bundle.resolve(&rel)?;
        let bytes = ctx
            .progress_and_step(format!("{}/{total} {shot_id} 下载", idx + 1), "download")
            .shot(shot_id)
            .node(node)
            .prompt(&sub.prompt_id)
            .with("path", json!(rel))
            .done(comfy.download(first, &dest))?;
        let _ = bytes;

        let duration_seconds = shot
            .get("length_frames")
            .and_then(|f| f.as_f64())
            .unwrap_or(0.0)
            / shot.get("fps").and_then(|f| f.as_f64()).unwrap_or(30.0);

        Ok(match mode {
            GenerateMode::Render => json!({
                "shot_id": shot_id,
                "node": node,
                "prompt_id": sub.prompt_id,
                "path": rel,
                "duration_seconds": duration_seconds
            }),
            GenerateMode::Preview => {
                let (width, height) = dims.expect("preview 模式下 dims 必然算过");
                json!({
                    "shot_id": shot_id,
                    "node": node,
                    "prompt_id": sub.prompt_id,
                    "path": rel,
                    "width": width,
                    "height": height,
                    "duration_seconds": duration_seconds
                })
            }
        })
    }

    fn post(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
        let render = need(&ctx.inputs, "render", StageId::Post)?;
        let script = need(&ctx.inputs, "script", StageId::Post)?;
        let media = Media::new(ctx.settings);

        let shots = render["shots"]
            .as_array()
            .ok_or_else(|| StudioError::internal("渲染结果里没有 shots"))?;
        let mut parts = Vec::new();
        for s in shots {
            let rel = s["path"]
                .as_str()
                .ok_or_else(|| StudioError::internal("渲染结果缺少 path"))?;
            parts.push(ctx.bundle.resolve(rel)?);
        }

        // 交付规格是短边 1080，而模型的原生画布只有 768。**先逐镜超分再拼接**：
        // 显存跟成片长度解耦（整片 300 帧进一个 latent 多半要开分块），
        // 复用逐镜重试，时序模型也不必去缝合两镜之间的硬切。
        // 超分后各镜参数仍然一致，下面的 can_stream_copy 照样成立。
        let upscale = ctx.settings.comfy_upscale();
        if upscale {
            let brief = need(&ctx.inputs, "brief", StageId::Post)?;
            let aspect = brief["aspect_ratio"].as_str().unwrap_or("9:16");
            parts = self.upscale_shots(ctx, &media, shots, &parts, aspect)?;
        } else {
            ctx.say("按配置跳过超分，成片是模型的原生画布");
        }

        // 先用 ffprobe 判断能不能直接 copy —— 五个镜头本来就是同一套参数出的，
        // 一致的话没必要让 ffmpeg 重编码一遍。
        let stream_copy = ctx
            .progress_and_step(
                format!("检查 {} 个镜头能否直接拼接", parts.len()),
                "probe_parts",
            )
            .with("parts", json!(parts.len()))
            .done(media.can_stream_copy(&parts))?;
        ctx.say(format!(
            "拼接 {} 个镜头（{}）",
            parts.len(),
            if stream_copy {
                "直接复制流"
            } else {
                "重编码"
            }
        ));
        let final_rel = "media/final.mp4";
        let final_path = ctx.bundle.resolve(final_rel)?;
        ctx.step("concat")
            .with("parts", json!(parts.len()))
            .with("stream_copied", json!(stream_copy))
            .done(media.concat(&parts, &final_path, !stream_copy))?;

        let cover_rel = "media/cover.jpg";
        let cover_path = ctx.bundle.resolve(cover_rel)?;
        ctx.progress_and_step("抽取封面", "cover")
            .done(media.extract_frame(&final_path, 0.5, &cover_path))?;

        let mut out = json!({
            "video": final_rel,
            "cover": cover_rel,
            "upscaled": upscale
        });

        if let Some(srt) = subtitles::from_script(script) {
            let srt_rel = "media/subtitles.srt";
            ctx.progress_and_step("写入字幕", "subtitles")
                .with("bytes", json!(srt.len()))
                .done(ctx.bundle.write(srt_rel, &srt))?;
            out["subtitles"] = json!(srt_rel);
        }

        let info = ctx
            .progress_and_step("核对成片元数据", "probe_final")
            .done(media.probe(&final_path))?;
        out["duration_seconds"] = json!(info.duration_seconds);
        out["aspect_ratio"] = json!(info.aspect_ratio());
        out["delivery"] = json!(format!("{}x{}", info.width, info.height));
        out["stream_copied"] = json!(stream_copy);

        Ok(wrap(StageId::Post, out))
    }

    /// 逐镜把渲染产物超分到交付规格，返回给拼接用的新路径（按原顺序）。
    ///
    /// **不降级。** ComfyUI 不可达、基线缺失或未核验，都在这里结构化阻塞，
    /// 不会安静地把原生画布的片子当成交付件交出去。真要接受原生画布，
    /// 用 `COMFY_UPSCALE=0` 明确说——那是一个选择，不是一次失败。
    fn upscale_shots(
        &self,
        ctx: &ExecContext<'_>,
        media: &Media,
        shots: &[Value],
        parts: &[PathBuf],
        aspect: &str,
    ) -> Result<Vec<PathBuf>> {
        let wf = ctx.step("load_upscale_baseline").done(
            Workflow::load(&self.baselines, UPSCALE_WORKFLOW).and_then(|w| {
                w.require_verified()?;
                Ok(w)
            }),
        )?;

        let comfy = Comfy::from_settings(ctx.settings);
        comfy.ensure_reachable()?;

        // 目标尺寸按各镜实测的宽高算，不按提示词包里写的——写的和出的对不上
        // 正是这个项目反复踩的那种坑，而这里 ffprobe 就在手边。
        let mut jobs: Vec<UpscaleJob> = Vec::new();
        for (idx, (shot, src)) in shots.iter().zip(parts).enumerate() {
            let shot_id = shot["shot_id"].as_str().unwrap_or("shot").to_string();
            let info = media.probe(src)?;
            let (w, h) = (info.width as i64, info.height as i64);
            let (tw, th) = delivery_dims(aspect, w, h);
            jobs.push(UpscaleJob {
                idx,
                shot_id,
                src: src.clone(),
                from: (w, h),
                to: (tw, th),
                seed: shot["seed"].as_i64().unwrap_or(idx as i64 + 1),
            });
        }

        let total = jobs.len();
        let todo: Vec<usize> = jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| j.to != j.from)
            .map(|(i, _)| i)
            .collect();
        for j in jobs.iter().filter(|j| j.to == j.from) {
            ctx.step("upscale")
                .shot(&j.shot_id)
                .with("skipped", json!("已经是交付规格"))
                .with("size", json!(format!("{}x{}", j.from.0, j.from.1)))
                .done(Ok::<(), StudioError>(()))?;
        }
        if todo.is_empty() {
            ctx.say("各镜已经是交付规格，不需要超分");
            return Ok(parts.to_vec());
        }
        ctx.say(format!(
            "超分 {} 个镜头到 {}x{}",
            todo.len(),
            jobs[todo[0]].to.0,
            jobs[todo[0]].to.1
        ));

        let results: Mutex<Vec<Option<PathBuf>>> = Mutex::new(vec![None; total]);
        let queue: Mutex<VecDeque<usize>> = Mutex::new(todo.iter().copied().collect());
        let failure: Mutex<Option<StudioError>> = Mutex::new(None);
        let worker_count = ctx.settings.comfy_concurrency().min(todo.len());

        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let (queue, results, failure, comfy, wf, jobs) =
                    (&queue, &results, &failure, &comfy, &wf, &jobs);
                scope.spawn(move || loop {
                    if ctx.is_cancelled() || failure.lock().unwrap().is_some() {
                        return;
                    }
                    let Some(i) = queue.lock().unwrap().pop_front() else {
                        return;
                    };
                    match self.upscale_one(ctx, comfy, wf, &jobs[i], total) {
                        Ok(p) => results.lock().unwrap()[i] = Some(p),
                        Err(e) => {
                            let mut f = failure.lock().unwrap();
                            if f.is_none() {
                                *f = Some(e);
                            }
                            return;
                        }
                    }
                });
            }
        });

        if let Some(e) = failure.into_inner().unwrap() {
            return Err(e);
        }
        if ctx.is_cancelled() {
            return Err(StudioError::internal("超分被中断"));
        }
        // 跳过的那几镜沿用原产物，顺序仍然是分镜顺序——拼接靠的是这个。
        Ok(results
            .into_inner()
            .unwrap()
            .into_iter()
            .zip(parts)
            .map(|(up, src)| up.unwrap_or_else(|| src.clone()))
            .collect())
    }

    /// 一镜的上传-提交-等待-下载，失败时最多重试 [`MAX_SHOT_ATTEMPTS`] 次。
    fn upscale_one(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        wf: &Workflow,
        job: &UpscaleJob,
        total: usize,
    ) -> Result<PathBuf> {
        let mut last_err = None;
        for attempt in 1..=MAX_SHOT_ATTEMPTS {
            if ctx.is_cancelled() {
                return Err(StudioError::internal("超分被中断"));
            }
            match self.upscale_once(ctx, comfy, wf, job, total) {
                Ok(p) => return Ok(p),
                Err(e) => {
                    if attempt < MAX_SHOT_ATTEMPTS {
                        ctx.say(format!(
                            "{} 超分第 {attempt} 次尝试失败（{}），重试中",
                            job.shot_id,
                            e.message()
                        ));
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err
            .unwrap_or_else(|| StudioError::internal(format!("{} 超分重试耗尽", job.shot_id))))
    }

    fn upscale_once(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        wf: &Workflow,
        job: &UpscaleJob,
        total: usize,
    ) -> Result<PathBuf> {
        let node = comfy.node();
        let bytes = std::fs::read(&job.src).map_err(|_| StudioError::ArtifactMissing {
            path: job.src.display().to_string(),
        })?;
        // `/upload/image` 收 mp4——video 通道那次真机验收里验过。
        let remote = comfy.upload_image(&format!("{}.mp4", job.shot_id), &bytes)?;

        let mut params = Map::new();
        params.insert("filename".into(), json!(remote));
        params.insert("width".into(), json!(job.to.0));
        params.insert("height".into(), json!(job.to.1));
        params.insert("seed".into(), json!(job.seed));
        params.insert(
            "output_prefix".into(),
            json!(format!("studio/upscaled/{}", job.shot_id)),
        );
        let graph = wf.apply(&params)?;

        let sub = ctx
            .progress_and_step(
                format!(
                    "{}/{total} {} 超分 {}x{} → {}x{}",
                    job.idx + 1,
                    job.shot_id,
                    job.from.0,
                    job.from.1,
                    job.to.0,
                    job.to.1
                ),
                "upscale_submit",
            )
            .shot(&job.shot_id)
            .node(node)
            .with("from", json!(format!("{}x{}", job.from.0, job.from.1)))
            .with("to", json!(format!("{}x{}", job.to.0, job.to.1)))
            .done(comfy.submit(&graph, "video-studio"))?;

        let files = comfy.wait(&sub)?;
        let first = files.first().ok_or_else(|| StudioError::ComfyFailed {
            node: node.to_string(),
            detail: format!("{} 超分执行完成但没有产出文件", job.shot_id),
        })?;

        let rel = format!("media/upscaled/{}.mp4", job.shot_id);
        let dest = ctx.bundle.resolve(&rel)?;
        ctx.step("upscale")
            .shot(&job.shot_id)
            .node(node)
            .prompt(&sub.prompt_id)
            .with("path", json!(rel))
            .with("to", json!(format!("{}x{}", job.to.0, job.to.1)))
            .done(comfy.download(first, &dest))?;
        Ok(dest)
    }

    fn review(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
        let post = need(&ctx.inputs, "post", StageId::Review)?;
        let script = need(&ctx.inputs, "script", StageId::Review)?;
        let brief = need(&ctx.inputs, "brief", StageId::Review)?;
        let storyboard = need(&ctx.inputs, "storyboard", StageId::Review)?;
        let media = Media::new(ctx.settings);

        let video_rel = post["video"]
            .as_str()
            .ok_or_else(|| StudioError::internal("后期结果里没有 video"))?;
        let info = ctx
            .progress_and_step("读取成片实测元数据", "probe_final")
            .done(media.probe(&ctx.bundle.resolve(video_rel)?))?;

        let want_duration = script["total_duration_seconds"].as_f64().unwrap_or(0.0);
        let want_ratio = brief["aspect_ratio"].as_str().unwrap_or("9:16");
        let want_shots = storyboard["shots"].as_array().map(|a| a.len()).unwrap_or(0);
        let got_shots = ctx.inputs["render"]["shots"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        // 每一条都必须基于 ffprobe 的实测值，不能靠推断。
        // 这四条全是技术项：passed 只由它们决定。
        let checks = vec![
            check(
                "总时长",
                (info.duration_seconds - want_duration).abs() <= 0.5,
                format!(
                    "实测 {:.2} 秒，剧本要求 {want_duration} 秒",
                    info.duration_seconds
                ),
            ),
            check(
                "画幅",
                info.aspect_ratio() == want_ratio,
                format!(
                    "实测 {}（{}x{}），要求 {want_ratio}",
                    info.aspect_ratio(),
                    info.width,
                    info.height
                ),
            ),
            check(
                "镜头数",
                want_shots > 0 && got_shots == want_shots,
                format!("渲染 {got_shots} 个，分镜 {want_shots} 个"),
            ),
            check(
                "音轨",
                info.has_audio,
                match &info.audio_codec {
                    Some(c) => format!("实测存在音轨（{c}）"),
                    None => "成片里没有音轨".to_string(),
                },
            ),
        ];

        let passed = checks
            .iter()
            .all(|c| c["passed"].as_bool().unwrap_or(false));
        Ok(wrap(
            StageId::Review,
            json!({ "passed": passed, "checks": checks }),
        ))
    }
}

/// 技术验收的一项。控制面只出这一类——内容那半由 Agent 事后补，
/// 见 `studio-core::rubric`。
fn check(name: &str, passed: bool, detail: String) -> Value {
    json!({ "name": name, "kind": "technical", "passed": passed, "detail": detail })
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use studio_engine::executor::{ExecRecorder, ProgressNote};
    use studio_engine::{Bundle, Settings};

    /// 一个应答一切请求都成功的假 ComfyUI 节点：健康、接受提交、立刻跑完、能下载。
    struct NodeStub {
        url: String,
        _handle: std::thread::JoinHandle<()>,
    }

    fn healthy_node() -> NodeStub {
        node_stub(0)
    }

    /// 前 `drop_first` 个连接直接掐掉不回应答，之后一切正常。
    /// 单入口之后「重试」不再是换节点，而是对同一个 URL 再来一次——
    /// 这个桩就是用来逼出那条路径的。
    fn node_stub(drop_first: usize) -> NodeStub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = 0usize;
            for stream in listener.incoming().take(128) {
                let Ok(mut stream) = stream else { continue };
                seen += 1;
                if seen <= drop_first {
                    drop(stream); // 连上就断，制造一次连接层失败
                    continue;
                }
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = if path.starts_with("/queue") {
                    json!({ "queue_running": [], "queue_pending": [] }).to_string()
                } else if path.starts_with("/prompt") {
                    json!({ "prompt_id": "p1", "number": 1 }).to_string()
                } else if path.starts_with("/history/") {
                    json!({ "p1": {
                        "status": { "status_str": "success", "completed": true },
                        "outputs": { "9": { "videos": [ { "filename": "out.mp4" } ] } }
                    }})
                    .to_string()
                } else if path.starts_with("/view") {
                    "fake-bytes".to_string()
                } else {
                    "{}".to_string()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        NodeStub {
            url: format!("http://127.0.0.1:{port}"),
            _handle: handle,
        }
    }

    /// 已验证的最小 workflow 基线：`minimax_h3/t2v`。
    fn write_baseline(dir: &std::path::Path) {
        let family = dir.join("minimax_h3");
        std::fs::create_dir_all(&family).unwrap();
        let baseline = json!({
            "_studio": { "bindings": { "positive": ["6.inputs.text"] }, "bindings_verified": true },
            "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "" } }
        });
        std::fs::write(family.join("t2v.json"), baseline.to_string()).unwrap();
    }

    fn shot(id: &str) -> Value {
        json!({
            "shot_id": id, "workflow": "minimax_h3/t2v", "positive": "p",
            "width": 100, "height": 100, "length_frames": 30, "fps": 30
        })
    }

    /// 搭好一个可以直接喂给 `Pipeline::render` 的 bundle + settings，
    /// `.env` 里的 `COMFY_NODE` 指向传入的那个假节点。
    fn scaffold(dir: &std::path::Path, node: &str) -> (Bundle, Settings) {
        scaffold_with_concurrency(dir, node, 4)
    }

    fn scaffold_with_concurrency(
        dir: &std::path::Path,
        node: &str,
        concurrency: usize,
    ) -> (Bundle, Settings) {
        let bundle = Bundle::scaffold(dir).unwrap();
        std::fs::write(
            dir.join(".env"),
            format!(
                "COMFY_NODE={node}\nCOMFY_TIMEOUT_SECS=20\nCOMFY_POLL_INTERVAL_SECS=1\n\
                 COMFY_CONCURRENCY={concurrency}\n"
            ),
        )
        .unwrap();
        let settings = Settings::load(None, Some(dir));
        (bundle, settings)
    }

    /// 后期阶段的输入：一镜渲染产物 + 剧本 + 需求。
    fn post_inputs(bundle: &Bundle) -> Value {
        bundle
            .write("media/sh01.mp4", "not-really-a-video")
            .unwrap();
        json!({
            "render": { "shots": [{ "shot_id": "sh01", "path": "media/sh01.mp4" }] },
            "script": { "segments": [] },
            "brief": { "aspect_ratio": "9:16" }
        })
    }

    /// **超分缺基线时结构化阻塞，不静默交原生画布。**
    ///
    /// 这条守的是「不降级」：交付规格达不到是要报出来的，不是悄悄把 768
    /// 短边的片子当成交付件交出去。
    #[test]
    fn a_missing_upscale_baseline_blocks_post_instead_of_downgrading() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path()); // 只有渲染基线，没有 seedvr2
        let good = healthy_node();
        let (bundle, settings) = scaffold(bundle_dir.path(), &good.url);

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let recorder = ExecRecorder::at(bundle.root());
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: post_inputs(&bundle),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let e = pipeline.post(&ctx).unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(
            e.message().contains("seedvr2/upscale"),
            "错误里要指名缺的是哪一条基线：{}",
            e.message()
        );
    }

    /// `COMFY_UPSCALE=0` 之后 post 一步都不碰 ComfyUI——基线缺着也照走，
    /// 后面卡在哪是 ffmpeg 那条老路的事，不再是超分的事。
    #[test]
    fn turning_upscale_off_skips_the_baseline_entirely() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        // 节点故意配成连不上：关掉超分之后 post 不该去碰它。
        let (bundle, _) = scaffold(bundle_dir.path(), "http://127.0.0.1:1");
        std::fs::write(
            bundle_dir.path().join(".env"),
            "COMFY_NODE=http://127.0.0.1:1\nCOMFY_UPSCALE=0\n",
        )
        .unwrap();
        let settings = Settings::load(None, Some(bundle_dir.path()));
        assert!(!settings.comfy_upscale());

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let recorder = ExecRecorder::at(bundle.root());
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: post_inputs(&bundle),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        // 那段假 mp4 迟早会让 ffprobe 或 ffmpeg 报错——重点是**不是**
        // model_contract_violation / comfy_unavailable：超分整条路没走。
        if let Err(e) = pipeline.post(&ctx) {
            assert!(
                !matches!(e.code(), "model_contract_violation" | "comfy_unavailable"),
                "关掉超分之后不该再碰基线或 ComfyUI，却报了 {}：{}",
                e.code(),
                e.message()
            );
        }
    }

    #[test]
    fn render_returns_comfy_unavailable_when_no_node_is_healthy() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let (bundle, settings) = scaffold(bundle_dir.path(), "http://127.0.0.1:1");

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let recorder = ExecRecorder::at(bundle.root());
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: json!({ "prompt_pack": { "shots": [shot("sh01")] } }),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let e = pipeline.render(&ctx).unwrap_err();
        assert_eq!(e.code(), "comfy_unavailable");
    }

    /// 第一次提交撞上连接层失败，重试打回同一个入口并跑成功。
    /// 单入口之后重试不再是「换个节点」，但重试路径本身必须还在。
    #[test]
    fn render_shot_retries_against_the_same_entrypoint_after_a_transient_failure() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        // 第一个连接直接被掐断 —— 提交必然失败，逼出重试路径。
        let good = node_stub(1);
        let (bundle, settings) = scaffold(bundle_dir.path(), &good.url);

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let comfy = Comfy::from_settings(&settings);
        let recorder = ExecRecorder::at(bundle.root());
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: Value::Null,
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let shot = shot("sh01");
        let resolver = assets::AssetResolver::new(&bundle, &settings, json!({}), BTreeMap::new());
        let result = pipeline
            .generate_shot(&ctx, &comfy, &resolver, 0, 1, &shot, GenerateMode::Render)
            .unwrap();
        assert_eq!(result["node"], json!(good.url), "重试应当打回同一个入口");
        assert_eq!(result["shot_id"], json!("sh01"));
    }

    /// 多个 worker 应当并发分担多个镜头，且不管谁先跑完，
    /// 落回 outputs 时仍按提示词包里镜头的原始顺序——`post` 阶段拼接靠这个顺序。
    #[test]
    fn render_runs_shots_concurrently_and_preserves_original_order() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let node = healthy_node();
        let (bundle, settings) = scaffold_with_concurrency(bundle_dir.path(), &node.url, 3);

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let recorder = ExecRecorder::at(bundle.root());
        let shot_ids = ["sh01", "sh02", "sh03", "sh04", "sh05"];
        let shots: Vec<Value> = shot_ids.iter().map(|id| shot(id)).collect();
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: json!({ "prompt_pack": { "shots": shots } }),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let outputs = pipeline.render(&ctx).unwrap();
        let got = outputs["render"]["shots"].as_array().unwrap();
        assert_eq!(got.len(), 5);
        for (i, id) in shot_ids.iter().enumerate() {
            assert_eq!(got[i]["shot_id"], json!(*id), "结果顺序必须与提示词包一致");
            assert_eq!(got[i]["path"], json!(format!("media/{id}.mp4")));
        }
    }

    /// preview 复用 render 同一套并发/重试逻辑，但落到 media/preview/、
    /// 尺寸按短边 480 缩放——帧数/时长照抄提示词包，不变。
    #[test]
    fn preview_scales_resolution_and_writes_under_media_preview() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let good = healthy_node();
        let (bundle, settings) = scaffold(bundle_dir.path(), &good.url);

        let pipeline = Pipeline::new(baselines_dir.path().to_path_buf());
        let recorder = ExecRecorder::at(bundle.root());
        // 竖屏 1080x1920，跟真实提示词包一致；短边 1080 缩到 480。
        let mut sh01 = shot("sh01");
        sh01["width"] = json!(1080);
        sh01["height"] = json!(1920);
        sh01["length_frames"] = json!(42);
        sh01["fps"] = json!(30);
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: json!({ "prompt_pack": { "shots": [sh01] } }),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let outputs = pipeline.preview(&ctx).unwrap();
        let got = &outputs["preview"]["shots"][0];
        assert_eq!(got["path"], json!("media/preview/sh01.mp4"));
        assert_eq!(got["width"], json!(480));
        assert_eq!(got["height"], json!(854));
        assert_eq!(got["duration_seconds"], json!(42.0 / 30.0));
    }

    /// 声明式的镜头走组装那条路：**用仓库里真实的片段库**拼出图，
    /// 提交给假节点，把产物下回来。这一条守的是 S5 那段接线本身——
    /// 组装器单测归单测，它有没有真的接到渲染链路上是另一回事。
    #[test]
    fn a_declarative_shot_is_assembled_and_rendered_end_to_end() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let node = healthy_node();
        let (bundle, settings) = scaffold(bundle_dir.path(), &node.url);

        // 基线目录直接指向仓库的 assets/workflows，这样片段库是真的那一份。
        let pipeline = Pipeline::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows"),
        );
        let recorder = ExecRecorder::at(bundle.root());
        // 无参考、无锚点的一镜：不需要任何素材上传，链路本身能单独验。
        let shot = json!({
            "shot_id": "sh01", "head": "reference", "positive": "船头切开湖面",
            "width": 768, "height": 1344, "length_frames": 56, "fps": 24, "seed": 1
        });
        let ctx = ExecContext {
            bundle: &bundle,
            settings: &settings,
            inputs: json!({
                "asset_plan": { "core_model_family": "minimax_h3", "assets": [] },
                "prompt_pack": { "shots": [shot] }
            }),
            progress: &ProgressNote::default(),
            recorder: &recorder,
            cancelled: &AtomicBool::new(false),
        };

        let outputs = pipeline.render(&ctx).unwrap();
        let got = &outputs["render"]["shots"][0];
        assert_eq!(got["shot_id"], json!("sh01"));
        assert_eq!(got["path"], json!("media/sh01.mp4"));

        // 落盘的 debug 请求体就是提交出去的那张图——组装的结果可复现。
        let body = std::fs::read_to_string(bundle.resolve("debug/sh01.request.json").unwrap())
            .expect("应当落一份可以直接 curl 复现的请求体");
        let sent: Value = serde_json::from_str(&body).unwrap();
        let graph = &sent["prompt"];
        assert_eq!(graph["h3_ref"]["inputs"]["prompt"], json!("船头切开湖面"));
        assert_eq!(graph["h3_ref"]["inputs"]["length"], json!(56));
        assert_eq!(
            graph["save_video"]["inputs"]["filename_prefix"],
            json!("studio/sh01")
        );
        // head 的配套约束覆盖到了骨架上：reference head 配 ref2va 权重 + beta。
        assert_eq!(graph["scheduler"]["inputs"]["scheduler"], json!("beta"));
        assert!(graph["load_unet"]["inputs"]["unet_name"]
            .as_str()
            .unwrap()
            .contains("ref2va"));
    }

    /// 没有接续引用时只有一波——分波逻辑不该给普通的包平白加上串行。
    #[test]
    fn shots_without_continuation_all_run_in_one_wave() {
        let shots = vec![shot("sh01"), shot("sh02"), shot("sh03")];
        assert_eq!(dependency_waves(&shots), vec![vec![0, 1, 2]]);
    }

    /// 接续镜要等它引用的那一镜出片，所以落到下一波。
    #[test]
    fn a_continuation_shot_lands_in_the_next_wave() {
        let mut sh02 = shot("sh02");
        sh02["guides"] = json!([{ "kind": "image", "at_frame": 0, "asset_id": "sh01.tail" }]);
        let mut sh03 = shot("sh03");
        sh03["guides"] = json!([{ "kind": "image", "at_frame": 0, "asset_id": "sh02.tail" }]);
        // sh04 谁也不接，跟 sh01 同一波。
        let shots = vec![shot("sh01"), sh02, sh03, shot("sh04")];
        assert_eq!(
            dependency_waves(&shots),
            vec![vec![0, 3], vec![1], vec![2]],
            "一条接续链排成三波，无关的镜头留在第一波一起跑"
        );
    }

    /// image head 靠 first_frame 接上一镜，分波必须算上它。
    /// 漏算的话这一镜会跟被接的那一镜同波跑，等解析素材时那一镜还没出片。
    #[test]
    fn a_frame_slot_continuation_also_creates_a_wave() {
        let mut sh02 = shot("sh02");
        sh02["head"] = json!("image");
        sh02["first_frame"] = json!("sh01.tail");
        assert_eq!(
            dependency_waves(&[shot("sh01"), sh02]),
            vec![vec![0], vec![1]]
        );
    }

    /// 预览尺寸要落在 32 的网格上：768x1344 短边缩到 480 得到 840，
    /// 不是 32 的倍数——不修就等于我们自己写进 remedy 的那句话，
    /// ComfyUI 四舍五入之后实际尺寸跟登记的对不上。
    #[test]
    fn preview_dimensions_are_rounded_onto_the_grid() {
        assert_eq!(scale_to_short_edge(768, 1344, 480), (480, 840));
        assert_eq!(round_to(840, 32), 832);
        assert_eq!(round_to(480, 32), 480);
        assert_eq!(round_to(10, 32), 32, "不能round 成 0");
    }

    /// 引用的是登记过的资产（`C01`、`C01.front`），不是镜间片段——
    /// 认错了会平白把整包串行化。
    #[test]
    fn registered_asset_references_do_not_create_waves() {
        let mut s = shot("sh02");
        s["references"] = json!([{ "kind": "image", "asset_id": "C01" },
                                 { "kind": "image", "asset_id": "SC02.key_angle" }]);
        assert_eq!(dependency_waves(&[shot("sh01"), s]), vec![vec![0, 1]]);
    }

    #[test]
    fn scale_to_short_edge_keeps_aspect_and_rounds_to_even() {
        assert_eq!(scale_to_short_edge(1080, 1920, 480), (480, 854));
        assert_eq!(scale_to_short_edge(1920, 1080, 480), (854, 480));
        assert_eq!(scale_to_short_edge(1024, 1024, 480), (480, 480));
        // 已经比目标短边还小也照样缩放，不做「已经够小就跳过」的特殊分支。
        assert_eq!(scale_to_short_edge(0, 0, 480), (480, 480));
    }

    #[test]
    fn delivery_dims_lands_on_the_declared_aspect() {
        // MiniMax 的「9:16」画布实际是 4:7；按声明的 9:16 算目标，
        // 多出来的宽度由 ResizeImageMaskNode 的 crop=center 裁掉。
        assert_eq!(delivery_dims("9:16", 768, 1344), (1080, 1920));
        assert_eq!(delivery_dims("16:9", 1344, 768), (1920, 1080));
        assert_eq!(delivery_dims("1:1", 768, 768), (1080, 1080));
        assert_eq!(delivery_dims("4:5", 768, 960), (1080, 1350));
    }

    /// `brief.aspect_ratio` 是自由文本。解析不出来时退回素材自己的比例——
    /// 拿一个猜出来的画幅去裁画面比不裁危险得多。
    #[test]
    fn an_unparseable_aspect_falls_back_to_the_source_ratio() {
        assert_eq!(delivery_dims("竖屏", 768, 1344), (1080, 1890));
        assert_eq!(delivery_dims("", 768, 1344), (1080, 1890));
        assert_eq!(delivery_dims("9:0", 768, 1344), (1080, 1890));
        assert_eq!(delivery_dims("9:16", 0, 0), (1080, 1080));
    }

    /// 只放大不缩小：已经够大的素材不该被这一步降下去。
    #[test]
    fn delivery_dims_never_shrinks_the_source() {
        assert_eq!(delivery_dims("9:16", 1440, 2560), (1440, 2560));
        assert_eq!(delivery_dims("16:9", 2560, 1440), (2560, 1440));
    }

    /// 两边都要偶数，H.264 要求如此。
    #[test]
    fn delivery_dims_are_even() {
        let (w, h) = delivery_dims("7:15", 768, 1344);
        assert_eq!((w % 2, h % 2), (0, 0), "{w}x{h} 里有奇数边");
    }
}

/// 拿**仓库里真实的基线文件**验能力面，而不是测试里手写的假数据。
///
/// 这一组测试是「写了会被静默丢弃」这条规则的最后一道锁：基线一改，
/// 这里立刻红，而不是等到某次渲染出来的画面莫名其妙不对。
#[cfg(test)]
mod real_baselines {
    use super::*;

    fn caps() -> CapabilitySet {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows");
        Pipeline::new(dir)
            .capabilities()
            .expect("仓库里应当有基线可读")
    }

    /// 默认核心系列走片段组装，不走整图基线。
    #[test]
    fn minimax_is_a_fragment_family() {
        let caps = caps();
        assert_eq!(caps.fragment_families(), vec!["minimax_h3".to_string()]);
        let set = caps
            .fragments_for("minimax_h3")
            .expect("片段库应当读得出来");
        assert!(set.backbone.is_some(), "缺骨架的片段库拼不出图");
        assert_eq!(set.verified_heads(), vec!["image", "reference"]);
        // 参考、锚点、素材三类片段各自齐全。
        assert!(set.guides.contains_key("image") && set.guides.contains_key("clip"));
        for medium in ["image", "video", "audio"] {
            assert!(set.inputs.contains_key(medium), "缺 {medium} 类输入片段");
        }
    }

    /// preview 的 turbo 叠加层：**用真实的片段文件**验它挂上去之后
    /// LoRA 与步数、调度器都对。调度器尤其要紧——真机对比里
    /// reference head 的 beta 档在 4 步下出来的画面是坏的（见
    /// `SOURCE-fragments.md`），overlay 必须把它盖成 simple。
    #[test]
    fn the_real_turbo_overlays_swap_in_the_lora_and_the_right_schedule() {
        use studio_core::assembly::{assemble_as, Combination};
        let caps = caps();
        let set = caps.fragments_for("minimax_h3").unwrap();
        for (head, steps, lora) in [
            ("reference", 4, "ref2v_turbo_4step"),
            ("image", 8, "fl2v_turbo_8step"),
        ] {
            let mut shot = studio_core::ShotDeclaration {
                shot_id: "S01".into(),
                head: head.into(),
                positive: "p".into(),
                width: 640,
                height: 384,
                length_frames: 22,
                fps: 24.0,
                seed: 1,
                references: Vec::new(),
                guides: Vec::new(),
                first_frame: None,
                last_frame: None,
            };
            if head == "image" {
                shot.first_frame = Some("f.png".into());
            }
            let out = assemble_as(set, &shot, "t/S01", Combination::PreviewTurbo)
                .unwrap_or_else(|e| panic!("{head} 的 turbo 组合拼不起来：{}", e.message()));
            assert!(out.notes.is_empty(), "{head}: {:?}", out.notes);
            let g = &out.graph;
            assert!(g["lora"]["inputs"]["lora_name"]
                .as_str()
                .unwrap()
                .contains(lora));
            assert_eq!(g["lora"]["inputs"]["model"], json!(["load_unet", 0]));
            assert_eq!(g["sigmashift"]["inputs"]["model"], json!(["lora", 0]));
            assert_eq!(g["scheduler"]["inputs"]["steps"], json!(steps));
            assert_eq!(
                g["scheduler"]["inputs"]["scheduler"],
                json!("simple"),
                "{head}：低步数下调度器档位是成败关键，overlay 必须显式写死"
            );
        }
    }

    /// 成片超分的基线：真读仓库里那一份，五条绑定都要指到存在的节点上。
    ///
    /// `width` / `height` 绑的是**带点的输入名**（`resize_type.width`）——
    /// 动态组合框的子输入就长这样。`check()` 过不去就说明路径规则又收窄了。
    #[test]
    fn the_upscale_baseline_is_sound_and_verified() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows");
        let w = Workflow::load(&dir, "seedvr2/upscale").expect("超分基线应当读得出来");
        w.check().expect("每条绑定都要指到存在的节点上");
        w.require_verified()
            .expect("这份基线真机跑过并人眼看过，应当是已核验");

        let mut p = Map::new();
        p.insert("filename".into(), json!("sh01.mp4"));
        p.insert("width".into(), json!(1080));
        p.insert("height".into(), json!(1920));
        p.insert("seed".into(), json!(7));
        p.insert("output_prefix".into(), json!("studio/up/sh01"));
        let g = w.apply(&p).unwrap();

        assert_eq!(g["load"]["inputs"]["file"], json!("sh01.mp4"));
        assert_eq!(g["resize"]["inputs"]["resize_type.width"], json!(1080));
        assert_eq!(g["resize"]["inputs"]["resize_type.height"], json!(1920));
        assert_eq!(
            g["resize"]["inputs"]["resize_type"],
            json!("scale dimensions"),
            "组合键本身不能被宽高覆盖掉"
        );
        // 一步采样的那几个数是模板原值，谁都不许改。
        assert_eq!(g["sampler"]["inputs"]["steps"], json!(1));
        assert_eq!(g["sampler"]["inputs"]["cfg"], json!(1));
        assert_eq!(g["sampler"]["inputs"]["scheduler"], json!("simple"));
        // 音轨从 GetVideoComponents 直接接到 CreateVideo，超分不能把声音丢了。
        assert_eq!(g["create"]["inputs"]["audio"], json!(["comp", 1]));
        assert_eq!(g["create"]["inputs"]["fps"], json!(["comp", 2]));
    }

    /// 卡片的两条基线：真读仓库里那两份，绑定都要指到存在的节点上。
    ///
    /// `multiref_edit` 还多一段 `reference_chain`——参考数由内容决定，
    /// 固定路径数组喂不下，所以它是第二种可变槽位形态（链式），
    /// 跟 `minimax_h3` 那边 AUTOGROW 的平铺编号并列。
    #[test]
    fn the_card_baselines_are_sound_and_verified() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows");
        for name in ["flux2_dev/t2i", "flux2_dev/multiref_edit"] {
            let w =
                Workflow::load(&dir, name).unwrap_or_else(|e| panic!("{name}: {}", e.message()));
            w.check()
                .unwrap_or_else(|e| panic!("{name} 的绑定指向不存在的节点：{}", e.message()));
            w.require_verified()
                .unwrap_or_else(|e| panic!("{name} 应当已核验：{}", e.message()));
            let mut p = Map::new();
            p.insert("positive".into(), json!("一个人"));
            p.insert("width".into(), json!(768));
            p.insert("height".into(), json!(1344));
            p.insert("seed".into(), json!(7));
            p.insert("output_prefix".into(), json!("studio/cards/C01"));
            let g = w.apply(&p).unwrap();
            assert_eq!(g["pos"]["inputs"]["text"], json!("一个人"));
            // 宽高要同时落到 scheduler 和 latent 上，只写一处会出错尺寸
            assert_eq!(g["sigmas"]["inputs"]["width"], json!(768));
            assert_eq!(g["latent"]["inputs"]["height"], json!(1344));
            assert_eq!(g["noise"]["inputs"]["noise_seed"], json!(7));
        }
    }

    /// 拿**仓库里真实的** `multiref_edit` 展开一条两张参考的链。
    #[test]
    fn the_real_card_baseline_expands_a_reference_chain() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows");
        let w = Workflow::load(&dir, "flux2_dev/multiref_edit").unwrap();
        assert_eq!(w.max_references(), 10, "FLUX.2 的参考上限是 10");

        let mut p = Map::new();
        p.insert("positive".into(), json!("侧面"));
        let g = w
            .apply_with_refs(&p, &["C01_front_full.png".into(), "C01_profile.png".into()])
            .unwrap();
        assert_eq!(
            g["ref1_load"]["inputs"]["image"],
            json!("C01_front_full.png")
        );
        assert_eq!(g["ref2_load"]["inputs"]["image"], json!("C01_profile.png"));
        // 链外的 vae 不能被改名，改了就断了
        assert_eq!(g["ref2_encode"]["inputs"]["vae"], json!(["vae", 0]));
        // 一环扣一环，最后接回采样器
        assert_eq!(
            g["ref1_link"]["inputs"]["conditioning"],
            json!(["guidance", 0])
        );
        assert_eq!(
            g["ref2_link"]["inputs"]["conditioning"],
            json!(["ref1_link", 0])
        );
        assert_eq!(
            g["guider"]["inputs"]["conditioning"],
            json!(["ref2_link", 0])
        );
    }

    /// 卡片基线也不该出现在 Agent 看得见的能力面里——它不吃 `length_frames`，
    /// 写进某一镜没有意义。
    #[test]
    fn the_card_baselines_are_not_offered_to_the_agent() {
        let names = caps().verified_names();
        assert!(
            !names.iter().any(|n| n.starts_with("flux2_dev/")),
            "卡片基线漏进了可选基线列表：{names:?}"
        );
    }

    /// 超分基线不该出现在 Agent 看得见的能力面里。写进某一镜没有任何意义
    /// ——它不吃 positive、不吃 length_frames，`filename` 还会是空的。
    #[test]
    fn the_upscale_baseline_is_not_offered_to_the_agent() {
        let names = caps().verified_names();
        assert!(
            !names.iter().any(|n| n.starts_with("seedvr2/")),
            "超分基线漏进了可选基线列表：{names:?}"
        );
    }

    /// 能力面对账的数据源换了，规则没换：这个系列一样不吃 negative。
    #[test]
    fn the_fragment_family_still_takes_no_negative() {
        let caps = caps();
        let params = caps.fragments_for("minimax_h3").unwrap().shot_params();
        for want in [
            "positive",
            "width",
            "height",
            "length_frames",
            "fps",
            "seed",
        ] {
            assert!(params.contains(&want.to_string()), "缺 {want} 绑定");
        }
        assert!(
            !params.contains(&"negative".to_string()),
            "片段库没有 negative 绑定——写了会被静默丢弃，能力面必须如实反映"
        );
        assert!(
            !params.contains(&"output_prefix".to_string()),
            "产物落在哪是控制面决定的，不该出现在 Agent 要写的参数里"
        );
    }

    /// 三种典型镜头都能从**真实的片段文件**拼回一张完整的图。
    /// 这一条守的是片段库本身：切错了、元数据缺了，这里立刻红。
    #[test]
    fn the_three_typical_shots_assemble_from_the_real_fragments() {
        let caps = caps();
        let set = caps.fragments_for("minimax_h3").unwrap();
        let base = studio_core::ShotDeclaration {
            shot_id: "S01".into(),
            head: "reference".into(),
            positive: "她走上球场".into(),
            width: 640,
            height: 384,
            length_frames: 22,
            fps: 24.0,
            seed: 1,
            references: Vec::new(),
            guides: Vec::new(),
            first_frame: None,
            last_frame: None,
        };
        let img_ref = |id: &str| studio_core::assembly::Reference {
            kind: studio_core::assembly::Medium::Image,
            asset_id: id.into(),
            with_audio: false,
        };

        // 1 秒空镜：image head，给首帧。
        let mut empty = base.clone();
        empty.head = "image".into();
        empty.first_frame = Some("scene_wide.png".into());
        // 接续镜：reference head + 两条参考 + 两个链式 guide。
        let mut continued = base.clone();
        continued.references = vec![img_ref("char_front.png"), img_ref("scene_wide.png")];
        continued.guides = vec![
            studio_core::assembly::Guide {
                kind: studio_core::assembly::GuideKind::Image,
                at_frame: 0,
                asset_id: "char_front.png".into(),
            },
            studio_core::assembly::Guide {
                kind: studio_core::assembly::GuideKind::Image,
                at_frame: -1,
                asset_id: "scene_wide.png".into(),
            },
        ];
        // 群戏：五条参考。
        let mut crowd = base.clone();
        crowd.references = (0..5).map(|_| img_ref("char_front.png")).collect();

        for (name, shot) in [("空镜", empty), ("接续镜", continued), ("群戏", crowd)] {
            let out = studio_core::assembly::assemble(set, &shot, "test/S01")
                .unwrap_or_else(|e| panic!("{name}组装失败：{}", e.message()));
            let graph = out.graph.as_object().unwrap();
            // 骨架留空的三处必须都被填上，否则提交给 ComfyUI 会被判非法。
            for path in [
                "guider.inputs.conditioning",
                "sampler.inputs.latent_image",
                "save_video.inputs.filename_prefix",
            ] {
                let (node, field) = path.split_once(".inputs.").unwrap();
                assert!(
                    graph[node]["inputs"].get(field).is_some(),
                    "{name}：{path} 没填上"
                );
            }
            // 确定性：同一份声明拼两次逐字节相同。
            let again = studio_core::assembly::assemble(set, &shot, "test/S01").unwrap();
            assert_eq!(out.graph, again.graph, "{name}的组装结果不确定");
        }
    }

    #[test]
    fn ltx_takes_seconds_not_frames() {
        let caps = caps();
        let t2v = caps.get("ltx2_5/t2v").unwrap();
        assert!(t2v.accepts("duration_seconds"));
        assert!(
            !t2v.accepts("length_frames"),
            "LTX 按秒收时长；混用会让时长完全不受控"
        );
        assert!(t2v.accepts("negative"), "这条基线是吃负向提示词的");
    }

    #[test]
    fn unverified_baselines_are_excluded_from_the_choices() {
        let caps = caps();
        let names = caps.verified_names();
        assert!(names.contains(&"ltx2_5/t2v".to_string()));
        for unverified in ["wan2_2/i2v", "wan2_2/flf2v", "wan_animate2/i2v"] {
            assert!(
                !names.contains(&unverified.to_string()),
                "{unverified} 尚未真机核验，不该出现在可选基线里"
            );
            assert!(caps.get(unverified).is_some_and(|w| !w.verified));
        }
    }

    /// 随包分发的黄金样例必须过得了自己这一关——它是 Agent 照着抄的范文。
    fn exemplar_assets() -> Vec<String> {
        studio_core::fixtures::outputs(StageId::VisualAssets)[StageId::VisualAssets.output_key()]
            ["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["asset_id"].as_str().map(String::from))
            .collect()
    }

    #[test]
    fn the_golden_exemplar_passes_the_real_capability_check() {
        let outputs = studio_core::fixtures::outputs(StageId::PromptPack);
        caps()
            .check_prompt_pack(&outputs, &exemplar_assets())
            .expect("黄金样例应当与真实片段库对得上");
    }

    /// 反过来验一次：给样例加一条 negative，就该被挡下。
    #[test]
    fn adding_a_negative_to_the_exemplar_is_rejected() {
        let mut outputs = studio_core::fixtures::outputs(StageId::PromptPack);
        outputs["prompt_pack"]["shots"][0]["negative"] = json!("文字, 水印");
        let e = caps()
            .check_prompt_pack(&outputs, &exemplar_assets())
            .unwrap_err();
        assert_eq!(e.code(), "schema_violation");
        assert!(e.message().contains("negative"), "{}", e.message());
    }

    /// 样例是片段化系列的，写 workflow 就是形状用错。
    #[test]
    fn adding_a_workflow_to_the_exemplar_is_rejected() {
        let mut outputs = studio_core::fixtures::outputs(StageId::PromptPack);
        outputs["prompt_pack"]["shots"][0]["workflow"] = json!("minimax_h3/t2v");
        let e = caps()
            .check_prompt_pack(&outputs, &exemplar_assets())
            .unwrap_err();
        assert!(e.message().contains("现场组装"), "{}", e.message());
    }
}
