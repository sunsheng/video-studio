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
use studio_core::{Outputs, Result, StageId, StudioError};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_media::Media;
use workflow::Workflow;

/// 单镜提交-等待-下载失败后允许重试的次数。第一次用调用方给的节点，
/// 之后每次重试都重新调 `pick_node()`——不认定原节点还活着。
const MAX_SHOT_ATTEMPTS: u32 = 3;

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
            StageId::Render => self.render(ctx),
            StageId::Post => self.post(ctx),
            StageId::Review => self.review(ctx),
            other => Err(StudioError::internal(format!(
                "{other} 不是确定性阶段，不该走到这里"
            ))),
        }
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
        let pack = need(&ctx.inputs, "prompt_pack", StageId::Render)?;
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
                    match self.render_shot(ctx, comfy, node, idx, total, shot) {
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
        let results = results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|v| v.expect("队列排空后每个下标都该有结果或已提前返回错误"))
            .collect::<Vec<_>>();

        Ok(wrap(StageId::Render, json!({ "shots": results })))
    }

    /// 一个镜头的提交-等待-下载，失败时最多重试 [`MAX_SHOT_ATTEMPTS`] 次。
    /// 第一次尝试用调用方给的（worker 绑定的）节点；重试不再信任原节点还活着，
    /// 每次都重新调 `pick_node()`，可能落到别的健康节点上。
    fn render_shot(
        &self,
        ctx: &ExecContext<'_>,
        comfy: &Comfy,
        preferred_node: &str,
        idx: usize,
        total: usize,
        shot: &Value,
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
        let graph = wf.apply(&params)?;

        // 落一份可以直接 curl 复现的请求体：节点故障时不用整套跑起来就能单独调试
        // 这一镜——`curl -X POST <node>/prompt -H "Content-Type: application/json"
        // --data @<bundle>/debug/<shot_id>.request.json`。写失败不影响主流程。
        let debug_rel = format!("debug/{shot_id}.request.json");
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

            match self.render_shot_once(ctx, comfy, &node, idx, total, &shot_id, &graph, &debug_rel, shot) {
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
    fn render_shot_once(
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
        let rel = format!("media/{shot_id}.{ext}");
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

        Ok(json!({
            "shot_id": shot_id,
            "node": node,
            "prompt_id": sub.prompt_id,
            "path": rel,
            "duration_seconds": shot.get("length_frames").and_then(|f| f.as_f64()).unwrap_or(0.0)
                / shot.get("fps").and_then(|f| f.as_f64()).unwrap_or(30.0)
        }))
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
            .render_shot(&ctx, &comfy, "http://127.0.0.1:1", 0, 1, &shot)
            .unwrap();
        assert_eq!(result["node"], json!(good.url), "重试应当落到唯一健康的节点上");
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
}
