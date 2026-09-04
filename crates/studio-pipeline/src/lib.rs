//! 三个确定性阶段的实现：渲染、后期、验收。
//!
//! 这一层把控制面的决策变成实际动作：向 ComfyUI 提交、用 ffmpeg 拼接、
//! 用 ffprobe 核对。**运行本程序的机器不需要 GPU**——推理全在 ComfyUI 那侧。

pub mod subtitles;
pub mod workflow;

use serde_json::{json, Map, Value};
use std::path::PathBuf;
use studio_comfy::Comfy;
use studio_core::{Outputs, Result, StageId, StudioError};
use studio_engine::executor::{ExecContext, StageExecutor};
use studio_media::Media;
use workflow::Workflow;

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
    fn render(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
        let pack = need(&ctx.inputs, "prompt_pack", StageId::Render)?;
        let shots = pack["shots"]
            .as_array()
            .ok_or_else(|| StudioError::internal("提示词包里没有 shots"))?;

        let comfy = Comfy::from_settings(ctx.settings);
        let mut results = Vec::new();

        for (i, shot) in shots.iter().enumerate() {
            if ctx.is_cancelled() {
                return Err(StudioError::internal("渲染被中断"));
            }
            let shot_id = shot["shot_id"].as_str().unwrap_or("shot").to_string();
            let wf_name =
                shot["workflow"]
                    .as_str()
                    .ok_or_else(|| StudioError::ModelContractViolation {
                        detail: format!("{shot_id} 没有指定 workflow"),
                    })?;

            let node = ctx
                .progress_and_step(
                    format!("{}/{} {shot_id} 选节点", i + 1, shots.len()),
                    "pick_node",
                )
                .shot(&shot_id)
                .done(comfy.pick_node())?;

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

            let sub = ctx
                .progress_and_step(
                    format!("{}/{} {shot_id} 提交到 {node}", i + 1, shots.len()),
                    "submit",
                )
                .shot(&shot_id)
                .node(&node)
                .with("debug_request", json!(debug_rel))
                .done(comfy.submit(&node, &graph, "video-studio"))?;

            ctx.say(format!(
                "{}/{} {shot_id} 渲染中（{}）",
                i + 1,
                shots.len(),
                sub.prompt_id
            ));
            let files = comfy.wait(&sub)?;
            let first = files.first().ok_or_else(|| StudioError::ComfyFailed {
                node: node.clone(),
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
                    format!("{}/{} {shot_id} 下载", i + 1, shots.len()),
                    "download",
                )
                .shot(&shot_id)
                .node(&node)
                .prompt(&sub.prompt_id)
                .with("path", json!(rel))
                .done(comfy.download(&node, first, &dest))?;

            let _ = bytes;
            results.push(json!({
                "shot_id": shot_id,
                "node": node,
                "prompt_id": sub.prompt_id,
                "path": rel,
                "duration_seconds": shot.get("length_frames").and_then(|f| f.as_f64()).unwrap_or(0.0)
                    / shot.get("fps").and_then(|f| f.as_f64()).unwrap_or(30.0)
            }));
        }

        Ok(wrap(StageId::Render, json!({ "shots": results })))
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
