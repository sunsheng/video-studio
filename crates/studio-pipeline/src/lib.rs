//! 三个确定性阶段的实现：渲染、后期、验收。
//!
//! 这一层把控制面的决策变成实际动作：向 ComfyUI 提交、用 ffmpeg 拼接、
//! 用 ffprobe 核对。**运行本程序的机器不需要 GPU**——推理全在 ComfyUI 那侧。

pub mod subtitles;
pub mod workflow;

use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use studio_comfy::Comfy;
use studio_core::contract::{AnswerOption, Confirmation, SelectionType};
use studio_core::{CapabilitySet, Outputs, Result, StageId, StudioError, WorkflowCapability};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_media::Media;
use workflow::Workflow;

/// 单镜提交-等待-下载失败后允许重试的次数。第一次用调用方给的节点，
/// 之后每次重试都重新调 `pick_node()`——不认定原节点还活着。
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

    /// 扫一遍基线目录，把每条基线的 `_studio.bindings` 投影成能力面。
    ///
    /// 引擎拿它在提交 `prompt_pack` 时对账。读不出来的基线直接跳过——
    /// 目录本身缺失或损坏是部署问题，会在渲染时以
    /// `model_contract_violation` 报出来，不该在提交阶段变成一堆噪声。
    fn capabilities(&self) -> Option<CapabilitySet> {
        let mut out = Vec::new();
        let families = std::fs::read_dir(&self.baselines).ok()?;
        for family in families.flatten() {
            if !family.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let family_name = family.file_name().to_string_lossy().to_string();
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
                out.push(WorkflowCapability {
                    params: wf.parameters(),
                    verified: wf.is_verified(),
                    unavailable_reason: wf.unavailable_reason().map(String::from),
                    name,
                });
            }
        }
        if out.is_empty() {
            return None;
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Some(CapabilitySet::new(out))
    }
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
    /// 按当下健康的节点数分片并发渲染：每个健康节点固定绑一个 worker 线程，
    /// 各镜头放共享队列，谁先跑完自己手上那镜就去认领下一镜。
    ///
    /// 实测单镜十来分钟，串行渲染 8 镜可能超过一个半小时；8 个健康节点并发
    /// 理论上十几分钟就能全部跑完。产出仍按镜头在提示词包里的原始顺序落回
    /// `shots` 数组——`post` 阶段拼接靠的是这个顺序，不是谁先完工。
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

    /// 按当下健康的节点数分片并发生成：每个健康节点固定绑一个 worker 线程，
    /// 各镜头放共享队列，谁先跑完自己手上那镜就去认领下一镜。
    ///
    /// 实测单镜十来分钟，串行渲染 8 镜可能超过一个半小时；8 个健康节点并发
    /// 理论上十几分钟就能全部跑完。产出仍按镜头在提示词包里的原始顺序落回
    /// `shots` 数组——`post` 阶段拼接靠的是这个顺序，不是谁先完工。
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
        let healthy: Vec<String> = comfy
            .health()
            .into_iter()
            .filter(|h| h.reachable)
            .map(|h| h.url)
            .collect();
        if healthy.is_empty() {
            return Err(StudioError::ComfyUnavailable {
                tried: comfy.nodes().to_vec(),
            });
        }

        let total = shots.len();
        let queue: Mutex<VecDeque<usize>> = Mutex::new((0..total).collect());
        let results: Mutex<Vec<Option<Value>>> = Mutex::new(vec![None; total]);
        let failure: Mutex<Option<StudioError>> = Mutex::new(None);
        let worker_count = healthy.len().min(total.max(1));

        std::thread::scope(|scope| {
            for node in healthy.iter().take(worker_count) {
                let queue = &queue;
                let results = &results;
                let failure = &failure;
                let comfy = &comfy;
                let node = node.as_str();
                scope.spawn(move || loop {
                    if ctx.is_cancelled() || failure.lock().unwrap().is_some() {
                        return;
                    }
                    let idx = queue.lock().unwrap().pop_front();
                    let Some(idx) = idx else { return };
                    let shot = &shots[idx];
                    match self.generate_shot(ctx, comfy, node, idx, total, shot, mode) {
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
        if ctx.is_cancelled() {
            return Err(StudioError::internal("渲染被中断"));
        }
        Ok(results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|v| v.expect("队列排空后每个下标都该有结果或已提前返回错误"))
            .collect())
    }

    /// 一个镜头的提交-等待-下载，失败时最多重试 [`MAX_SHOT_ATTEMPTS`] 次。
    /// 第一次尝试用调用方给的（worker 绑定的）节点；重试不再信任原节点还活着，
    /// 每次都重新调 `pick_node()`，可能落到别的健康节点上。
    #[allow(clippy::too_many_arguments)]
    fn generate_shot(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        preferred_node: &str,
        idx: usize,
        total: usize,
        shot: &Value,
        mode: GenerateMode,
    ) -> Result<Value> {
        let shot_id = shot["shot_id"].as_str().unwrap_or("shot").to_string();
        let wf_name =
            shot["workflow"]
                .as_str()
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!("{shot_id} 没有指定 workflow"),
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
        // 只有 preview 才覆盖尺寸；render 用提示词包原样的宽高，
        // dims 留 None 表示「结果里不需要单独报宽高」。
        let dims = if mode == GenerateMode::Preview {
            let width = shot
                .get("width")
                .and_then(|v| v.as_i64())
                .unwrap_or(PREVIEW_SHORT_EDGE);
            let height = shot
                .get("height")
                .and_then(|v| v.as_i64())
                .unwrap_or(PREVIEW_SHORT_EDGE);
            let (pw, ph) = scale_to_short_edge(width, height, PREVIEW_SHORT_EDGE);
            params.insert("width".to_string(), json!(pw));
            params.insert("height".to_string(), json!(ph));
            Some((pw, ph))
        } else {
            None
        };
        let graph = wf.apply(&params)?;

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
            let node = if attempt == 1 {
                preferred_node.to_string()
            } else {
                // 已经没有健康节点了，重试没有意义，直接把错误交回去。
                comfy.pick_node()?
            };

            match self.generate_shot_once(
                ctx, comfy, &node, idx, total, &shot_id, &graph, &debug_rel, shot, mode, dims,
            ) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if attempt < MAX_SHOT_ATTEMPTS {
                        ctx.say(format!(
                            "[{node}] {}/{total} {shot_id} 第 {attempt} 次尝试失败（{}），重试中",
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

    #[allow(clippy::too_many_arguments)]
    fn generate_shot_once(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        node: &str,
        idx: usize,
        total: usize,
        shot_id: &str,
        graph: &Value,
        debug_rel: &str,
        shot: &Value,
        mode: GenerateMode,
        dims: Option<(i64, i64)>,
    ) -> Result<Value> {
        let sub = ctx
            .progress_and_step(
                format!("[{node}] {}/{total} {shot_id} 提交", idx + 1),
                "submit",
            )
            .shot(shot_id)
            .node(node)
            .with("debug_request", json!(debug_rel))
            .done(comfy.submit(node, graph, "video-studio"))?;

        ctx.say(format!(
            "[{node}] {}/{total} {shot_id} 渲染中（{}）",
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
            .progress_and_step(
                format!("[{node}] {}/{total} {shot_id} 下载", idx + 1),
                "download",
            )
            .shot(shot_id)
            .node(node)
            .prompt(&sub.prompt_id)
            .with("path", json!(rel))
            .done(comfy.download(node, first, &dest))?;
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

        let mut out = json!({ "video": final_rel, "cover": cover_rel });

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
        out["stream_copied"] = json!(stream_copy);

        Ok(wrap(StageId::Post, out))
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

fn check(name: &str, passed: bool, detail: String) -> Value {
    json!({ "name": name, "passed": passed, "detail": detail })
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(128) {
                let Ok(mut stream) = stream else { continue };
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
    /// `.env` 里的 `COMFY_NODES` 指向传入的这些假节点。
    fn scaffold(dir: &std::path::Path, nodes: &[String]) -> (Bundle, Settings) {
        let bundle = Bundle::scaffold(dir).unwrap();
        std::fs::write(
            dir.join(".env"),
            format!(
                "COMFY_NODES={}\nCOMFY_TIMEOUT_SECS=20\nCOMFY_POLL_INTERVAL_SECS=1\n",
                nodes.join(",")
            ),
        )
        .unwrap();
        let settings = Settings::load(None, Some(dir));
        (bundle, settings)
    }

    #[test]
    fn render_returns_comfy_unavailable_when_no_node_is_healthy() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let (bundle, settings) = scaffold(bundle_dir.path(), &["http://127.0.0.1:1".into()]);

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

    /// 首选节点根本联系不上（提交即失败），重试时重新 `pick_node()`，
    /// 落到另一个健康节点上并跑成功——这是 issue 里要求的「干净重试」。
    #[test]
    fn render_shot_retries_on_a_different_node_after_the_preferred_one_fails() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let good = healthy_node();
        let (bundle, settings) = scaffold(bundle_dir.path(), std::slice::from_ref(&good.url));

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
        // 首选节点是个根本没在监听的端口——提交必然失败，逼出重试路径。
        let result = pipeline
            .generate_shot(
                &ctx,
                &comfy,
                "http://127.0.0.1:1",
                0,
                1,
                &shot,
                GenerateMode::Render,
            )
            .unwrap();
        assert_eq!(
            result["node"],
            json!(good.url),
            "重试应当落到唯一健康的节点上"
        );
        assert_eq!(result["shot_id"], json!("sh01"));
    }

    /// 多个健康节点应当并发分担多个镜头，且不管谁先跑完，
    /// 落回 outputs 时仍按提示词包里镜头的原始顺序——`post` 阶段拼接靠这个顺序。
    #[test]
    fn render_runs_shots_concurrently_and_preserves_original_order() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let baselines_dir = tempfile::tempdir().unwrap();
        write_baseline(baselines_dir.path());
        let nodes: Vec<NodeStub> = (0..3).map(|_| healthy_node()).collect();
        let node_urls: Vec<String> = nodes.iter().map(|n| n.url.clone()).collect();
        let (bundle, settings) = scaffold(bundle_dir.path(), &node_urls);

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
        let (bundle, settings) = scaffold(bundle_dir.path(), std::slice::from_ref(&good.url));

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

    #[test]
    fn scale_to_short_edge_keeps_aspect_and_rounds_to_even() {
        assert_eq!(scale_to_short_edge(1080, 1920, 480), (480, 854));
        assert_eq!(scale_to_short_edge(1920, 1080, 480), (854, 480));
        assert_eq!(scale_to_short_edge(1024, 1024, 480), (480, 480));
        // 已经比目标短边还小也照样缩放，不做「已经够小就跳过」的特殊分支。
        assert_eq!(scale_to_short_edge(0, 0, 480), (480, 480));
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

    #[test]
    fn minimax_takes_no_negative_and_no_references() {
        let caps = caps();
        let t2v = caps
            .get("minimax_h3/t2v")
            .expect("默认核心系列的 t2v 应当在");
        assert!(t2v.verified, "minimax_h3/t2v 应当是已核验的");
        assert!(t2v.accepts("positive") && t2v.accepts("length_frames"));
        assert!(
            !t2v.accepts("negative"),
            "这条基线没有 negative 绑定——写了会被静默丢弃，能力面必须如实反映"
        );
        assert!(
            !t2v.accepts("references"),
            "还没有图片输入通道，能力面不能假装有"
        );
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
        assert!(names.contains(&"minimax_h3/t2v".to_string()));
        for unverified in ["wan2_2/i2v", "wan2_2/flf2v", "wan_animate2/i2v"] {
            assert!(
                !names.contains(&unverified.to_string()),
                "{unverified} 尚未真机核验，不该出现在可选基线里"
            );
            assert!(caps.get(unverified).is_some_and(|w| !w.verified));
        }
    }

    /// 随包分发的黄金样例必须过得了自己这一关——它是 Agent 照着抄的范文。
    #[test]
    fn the_golden_exemplar_passes_the_real_capability_check() {
        let outputs = studio_core::fixtures::outputs(StageId::PromptPack);
        caps()
            .check_prompt_pack(&outputs)
            .expect("黄金样例应当与真实基线的能力面对得上");
    }

    /// 反过来验一次：给样例加一条 negative，就该被挡下。
    #[test]
    fn adding_a_negative_to_the_exemplar_is_rejected() {
        let mut outputs = studio_core::fixtures::outputs(StageId::PromptPack);
        outputs["prompt_pack"]["shots"][0]["negative"] = json!("文字, 水印");
        let e = caps().check_prompt_pack(&outputs).unwrap_err();
        assert_eq!(e.code(), "schema_violation");
        assert!(e.message().contains("negative"), "{}", e.message());
    }
}
