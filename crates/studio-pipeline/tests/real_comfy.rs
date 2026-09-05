//! 真机验收：组装出来的图，交给**真的 ComfyUI** 判。
//!
//! 这一组测试有环境前置条件，**CI 里不会跑**：
//!
//! - `COMFY_NODE` 指向一个可达的 ComfyUI 入口
//! - 该实例上装着 `minimax_h3` 与 `seedvr2` 的权重
//! - 本机有 `ffmpeg` / `ffprobe`（现造参考图，核对产物的尺寸帧数音轨）
//!
//! 缺任何一条就跳过并说明原因——不满足前置条件时**假装通过**比红更糟，
//! 但把它变成红也不对：那不是代码的问题。所以跳过 + 打印理由。
//!
//! 为什么非要真机：`node_errors` 为空只证明**图是合法的**，证明不了画面
//! 是对的。turbo 叠加层就栽在这儿——四种组合图校验全过，真机出片一看
//! reference + 4 步是坏的（见 `assets/workflows/minimax_h3/SOURCE-fragments.md`）。
//! 所以这里除了图校验，还要真等出片。

use serde_json::json;
use studio_comfy::Comfy;
use studio_core::assembly::{
    assemble_as, Combination, Fragment, FragmentSet, Guide, GuideKind, Medium, Reference,
    ShotDeclaration,
};
use studio_engine::bundle::Bundle;
use studio_engine::Settings;
use studio_pipeline::workflow::Workflow;

/// 一镜的规格：小而快，只为验接线，不为出好看的画面。
const W: i64 = 640;
const H: i64 = 384;
const FRAMES: i64 = 22; // 17k+5 网格上的第二档
const FPS: f64 = 24.0;

struct Env {
    _dir: tempfile::TempDir,
    bundle: Bundle,
    comfy: Comfy,
    /// 超分那条验收要用它建 `Media`（ffprobe 核对尺寸、帧数、音轨）。
    settings: Settings,
    /// `mut` 是给放开 video 通道那条测试用的：它要临时把
    /// `input.video` 置为已核验才拼得出图，见那条测试的注释。
    set: FragmentSet,
}

/// 备齐前置条件；缺什么就返回 None 并说明。
fn setup() -> Option<Env> {
    let node = std::env::var("COMFY_NODE")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty());
    let Some(node) = node else {
        eprintln!("跳过：没有 COMFY_NODE，这台机器上没有可达的 ComfyUI");
        return None;
    };
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("跳过：本机没有 ffmpeg，造不出参考图");
        return None;
    }

    let dir = tempfile::tempdir().ok()?;
    let bundle = Bundle::scaffold(dir.path()).ok()?;
    let mut env_file =
        format!("COMFY_NODE={node}\nCOMFY_TIMEOUT_SECS=600\nCOMFY_POLL_INTERVAL_SECS=3\n");
    if let Ok(token) = std::env::var("COMFY_TOKEN") {
        env_file.push_str(&format!("COMFY_TOKEN={token}\n"));
    }
    std::fs::write(dir.path().join(".env"), env_file).ok()?;

    let settings = Settings::load(None, Some(bundle.root()));
    let comfy = Comfy::from_settings(&settings);
    if let Err(e) = comfy.ensure_reachable() {
        eprintln!("跳过：ComfyUI 不可达（{}）", e.message());
        return None;
    }
    let set = fragments()?;
    Some(Env {
        _dir: dir,
        bundle,
        comfy,
        settings,
        set,
    })
}

/// 读仓库里**真实的**片段库——测试里手写一份就失去意义了。
fn fragments() -> Option<FragmentSet> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/workflows/minimax_h3/fragments");
    let mut set = FragmentSet::default();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();
        let (kind, frag) = Fragment::parse(&text, &name).ok()?;
        set.insert(kind, frag);
    }
    set.backbone.as_ref()?;
    Some(set)
}

/// 数一段视频有几帧。用 `nb_read_packets` 而不是 `nb_frames`——后者对某些
/// 封装是空的，而这里恰恰要靠帧数把成片和锚点素材分开，读不到就等于没验。
fn probe_frames(path: &std::path::Path) -> i64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_packets",
            "-show_entries",
            "stream=nb_read_packets",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe 起不来");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_end_matches(',')
        .parse()
        .unwrap_or_else(|_| panic!("ffprobe 读不出帧数：{}", path.display()))
}

/// 现造一张纯色参考图并传上去，返回 ComfyUI 那侧的文件名。
fn upload_swatch(env: &Env, name: &str, color: &str) -> String {
    let local = env.bundle.resolve(&format!("media/{name}.png")).unwrap();
    let ok = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c={color}:s={W}x{H}"))
        .args(["-frames:v", "1"])
        .arg(&local)
        .status()
        .expect("ffmpeg 起不来");
    assert!(ok.success(), "造参考图失败");
    let bytes = std::fs::read(&local).unwrap();
    env.comfy
        .upload_image(&format!("{name}.png"), &bytes)
        .expect("参考图传不上去")
}

fn base(shot_id: &str, head: &str) -> ShotDeclaration {
    ShotDeclaration {
        shot_id: shot_id.into(),
        head: head.into(),
        positive: "an empty clay tennis court at dusk".into(),
        width: W,
        height: H,
        length_frames: FRAMES,
        fps: FPS,
        seed: 20260905,
        references: Vec::new(),
        guides: Vec::new(),
        first_frame: None,
        last_frame: None,
    }
}

fn img(asset: &str) -> Reference {
    Reference {
        kind: Medium::Image,
        asset_id: asset.into(),
        with_audio: false,
    }
}

/// SPEC-0014 §8.2 第 1 步：三种典型镜头组装出的图，ComfyUI 认不认。
///
/// 提交成功（拿到 prompt_id、没有 node_errors）就说明接线合法。三镜都很小，
/// 顺带等它们跑完——图合法不等于跑得动，缺权重、显存不够都要在这里暴露。
#[test]
fn three_typical_shots_assemble_and_render_on_a_real_comfyui() {
    let Some(env) = setup() else { return };

    let swatch = upload_swatch(&env, "ref_a", "0x4a7a3f");
    let swatch2 = upload_swatch(&env, "ref_b", "0x8a5a2f");

    // 1 秒空镜：image head 给首帧，锁构图。
    let mut empty = base("S01", "image");
    empty.first_frame = Some(swatch.clone());

    // 接续镜：reference head + 两条参考 + 一个锚在第 0 帧的 guide。
    let mut continued = base("S03", "reference");
    continued.references = vec![img(&swatch), img(&swatch2)];
    continued.guides = vec![Guide {
        kind: GuideKind::Image,
        at_frame: 0,
        asset_id: swatch2.clone(),
    }];

    // 群戏：五条参考，AUTOGROW 序号要连续不重号。
    let mut crowd = base("S07", "reference");
    crowd.references = (0..5).map(|_| img(&swatch)).collect();

    for (name, shot) in [("空镜", empty), ("接续镜", continued), ("群戏", crowd)] {
        let out = assemble_as(
            &env.set,
            &shot,
            &format!("real-acceptance/{}", shot.shot_id),
            Combination::Standard,
        )
        .unwrap_or_else(|e| panic!("{name}组装失败：{}", e.message()));

        let sub = env
            .comfy
            .submit(&out.graph, "real-acceptance")
            .unwrap_or_else(|e| {
                panic!(
                    "{name}被 ComfyUI 判为非法——组装出的接线不对：{}",
                    e.message()
                )
            });
        let files = env
            .comfy
            .wait(&sub)
            .unwrap_or_else(|e| panic!("{name}执行失败：{}", e.message()));
        assert!(!files.is_empty(), "{name}跑完却没有产出文件");
        eprintln!("✅ {name}（{}）：{} 个产物", sub.prompt_id, files.len());
    }
}

/// SPEC-0014 §5.4：preview 的 turbo 组合真机跑得通，且**画面不是坏的**。
///
/// 「画面不是坏的」这里只能机械地验到「跑完了、有产出、时长对得上」——
/// 更细的判断要人眼看。turbo 那次翻车就是图校验和产出检查都过、人眼一看
/// 才发现的，所以这条测试断言的是下限，不是上限。
#[test]
fn the_preview_turbo_combination_runs_on_a_real_comfyui() {
    let Some(env) = setup() else { return };

    for head in ["reference", "image"] {
        let mut shot = base("PV1", head);
        if head == "image" {
            shot.first_frame = Some(upload_swatch(&env, "pv_first", "0x3f5a7a"));
        }
        let turbo = assemble_as(
            &env.set,
            &shot,
            &format!("real-acceptance/turbo-{head}"),
            Combination::PreviewTurbo,
        )
        .unwrap_or_else(|e| panic!("{head} 的 turbo 组合拼不起来：{}", e.message()));
        assert!(
            turbo.notes.is_empty(),
            "{head} 的 turbo 叠加层没挂上：{:?}",
            turbo.notes
        );
        // 步数确实降下来了，调度器也跟着换了——低步数下这个档位是成败关键。
        let sched = &turbo.graph["scheduler"]["inputs"];
        assert!(sched["steps"].as_i64().unwrap() < 20, "{head} 的步数没降");
        assert_eq!(sched["scheduler"], json!("simple"), "{head} 的调度器没换");

        let sub = env
            .comfy
            .submit(&turbo.graph, "real-acceptance")
            .unwrap_or_else(|e| panic!("{head} 的 turbo 图被判非法：{}", e.message()));
        let files = env
            .comfy
            .wait(&sub)
            .unwrap_or_else(|e| panic!("{head} 的 turbo 执行失败：{}", e.message()));
        assert!(!files.is_empty(), "{head} 的 turbo 跑完却没有产出");
        eprintln!(
            "✅ {head} turbo（{}）：{} 个产物",
            sub.prompt_id,
            files.len()
        );
    }
}

/// 未核验的通道要**结构化阻塞**，不能悄悄退化成别的东西。
///
/// video 通道核验之后，仍未核验的是 audio——规则本身还得有人守着。
/// `clip` 锚点曾经被错误地映射到 LoadImage：图能过校验、也能出片，只是锚的
/// 是一张静帧而不是一段。这条测试守着「宁可挡下，不要静默降级」。
#[test]
fn an_unverified_input_channel_is_refused_rather_than_downgraded() {
    let Some(env) = setup() else { return };
    let input_audio = env.set.inputs.get("audio").expect("片段库缺 input.audio");
    if input_audio.verified {
        eprintln!("跳过：所有输入通道都核验过了，这条测试的前提不再成立");
        return;
    }

    let mut shot = base("S09", "reference");
    shot.guides = vec![Guide {
        kind: GuideKind::Audio,
        at_frame: 0,
        asset_id: "S08.tail5".into(),
    }];
    let err = assemble_as(&env.set, &shot, "x/S09", Combination::Standard)
        .expect_err("未核验的通道不该拼得出图");
    assert_eq!(err.code(), "model_contract_violation");
    assert!(
        err.message().contains("尚未核验"),
        "要说清是没核验：{}",
        err.message()
    );
}

/// 造一段短视频并传上去，返回 ComfyUI 那侧的文件名。
///
/// `frames` 落在 `17k+5` 网格上——clip 锚点的长度吃同一套网格（SPEC-0014 V5）。
///
/// **锚点必须比镜头短。** 等长的锚点等于把整镜钉死，模型只会把它复现出来，
/// 提示词一个字都不生效——那种配置证明不了这条通道能用。踩过一次：22 帧锚点
/// 挂 22 帧镜头，出来的就是锚点本身。
fn upload_clip(env: &Env, name: &str, frames: i64) -> String {
    let local = env.bundle.resolve(&format!("media/{name}.mp4")).unwrap();
    let seconds = frames as f64 / FPS;
    let ok = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
        // 渐变色块：帧与帧之间有变化，才看得出接进去的是一段而不是一张静图。
        .arg(format!(
            "gradients=s={W}x{H}:c0=0x1a3a5a:c1=0x7a5a2a:duration={seconds}:rate={FPS}"
        ))
        .args(["-t", &format!("{seconds}"), "-pix_fmt", "yuv420p"])
        .arg(&local)
        .status()
        .expect("ffmpeg 起不来");
    assert!(ok.success(), "造锚点视频失败");
    let bytes = std::fs::read(&local).unwrap();
    // `upload_image` 打的是 `/upload/image`——那个端点收不收 mp4，只有真机
    // 答得了。收不了的话这里要**把服务端的原话带出来**，否则得再跑一轮才
    // 知道是被拒了还是别的问题。
    env.comfy
        .upload_image(&format!("{name}.mp4"), &bytes)
        .unwrap_or_else(|e| {
            panic!(
                "锚点视频（{} 字节）传不上去：{}\n\
                 如果是端点不收视频，就得给 studio-comfy 补一条视频上传的路径，\n\
                 而不是把这条测试改成传图片——传图片验不了 clip 通道。",
                bytes.len(),
                e.message()
            )
        })
}

/// **这条测试就是放开 `input.video` 的依据。**
///
/// `clip` 锚点和 `kind: video` 的参考都走 `LoadVideo` + `GetVideoComponents`，
/// 那条通道的 `bindings_verified` 现在是 `false`——所以组装器会拒绝拼图。
/// 测试里**临时把它置为已核验**再拼：这不是绕过规矩，是产出「能不能改成
/// true」这个问题的答案。真机跑通了才有资格去改片段文件里那个字段。
///
/// 跑法：
/// ```text
/// COMFY_NODE=<入口> COMFY_TOKEN=<token> \
///   cargo test -p studio-pipeline --test real_comfy video_channel -- --nocapture
/// ```
#[test]
fn the_video_input_channel_renders_on_a_real_comfyui() {
    let Some(mut env) = setup() else { return };
    let already = env.set.inputs["video"].verified;
    // 未核验时先解锁，好让组装器肯出图；已核验就照原样跑，当回归测试用。
    if !already {
        env.set.inputs.get_mut("video").unwrap().verified = true;
    }

    // 5 帧锚点接在 39 帧镜头的开头：前 5 帧跟着锚点走，其余由提示词接管。
    // 两个数都在 17k+5 网格上。
    const ANCHOR_FRAMES: i64 = 5;
    const LONG_SHOT: i64 = 39;
    let clip = upload_clip(&env, "anchor_clip", ANCHOR_FRAMES);
    eprintln!("锚点视频上传后的文件名：{clip}（{ANCHOR_FRAMES} 帧）");

    // 1. clip 锚点：把一小段帧序列锚在第 0 帧，镜头其余部分接着往下走。
    let mut anchored = base("V01", "reference");
    anchored.length_frames = LONG_SHOT;
    anchored.guides = vec![Guide {
        kind: GuideKind::Clip,
        at_frame: 0,
        asset_id: clip.clone(),
    }];

    // 2. video 参考：同一条通道，放开之后这条也跟着可用，一并验。
    let mut with_video_ref = base("V02", "reference");
    with_video_ref.references = vec![Reference {
        kind: Medium::Video,
        asset_id: clip.clone(),
        with_audio: false,
    }];

    for (name, shot) in [("clip 锚点", anchored), ("video 参考", with_video_ref)] {
        let out = assemble_as(
            &env.set,
            &shot,
            &format!("real-acceptance/{}", shot.shot_id),
            Combination::Standard,
        )
        .unwrap_or_else(|e| panic!("{name}组装失败：{}", e.message()));

        // 接的必须是 GetVideoComponents 的 IMAGE 输出，不是 LoadImage——
        // 两边类型都是 IMAGE，接错了图照样合法，只是喂进去一张静帧。
        let g = &out.graph;
        let load = g
            .as_object()
            .unwrap()
            .iter()
            .find(|(k, _)| k.ends_with("_load"))
            .expect("图里应当有一个素材加载节点");
        assert_eq!(
            load.1["class_type"],
            json!("LoadVideo"),
            "{name}：走的不是 LoadVideo"
        );

        let sub = env
            .comfy
            .submit(&out.graph, "real-acceptance")
            .unwrap_or_else(|e| {
                panic!(
                    "{name}被 ComfyUI 判为非法——video 通道的接线不对：{}",
                    e.message()
                )
            });
        let files = env
            .comfy
            .wait(&sub)
            .unwrap_or_else(|e| panic!("{name}执行失败：{}", e.message()));
        assert!(!files.is_empty(), "{name}跑完却没有产出文件");

        // **必须下载下来核对。** `LoadVideo` 会把输入素材回显进 history 的
        // outputs，节点 id 排序下它还排在 `save_video` 前面；只断言「有产物」
        // 的话，拿到锚点素材当成成片也照样绿。核对帧数就能分开——锚点 5 帧，
        // 镜头 39 帧。
        let dest = env
            .bundle
            .resolve(&format!("media/out_{}.mp4", shot.shot_id))
            .unwrap();
        env.comfy
            .download(&files[0], &dest)
            .unwrap_or_else(|e| panic!("{name}的产物下不下来：{}", e.message()));
        let frames = probe_frames(&dest);
        assert_eq!(
            frames, shot.length_frames,
            "{name}拿到的不是这一镜的成片（{frames} 帧，应当是 {} 帧）——\
             多半是把 LoadVideo 回显的输入素材当成产物了（锚点只有 {ANCHOR_FRAMES} 帧）",
            shot.length_frames
        );
        eprintln!(
            "✅ {name}（{}）：{} 个产物，成片 {frames} 帧",
            sub.prompt_id,
            files.len()
        );
    }

    if !already {
        eprintln!(
            "\n两条都跑通了。现在可以把 assets/workflows/minimax_h3/fragments/\
             input.video.json 的 bindings_verified 改成 true，并在 SOURCE-fragments.md \
             里记下这次的 run。**改之前先人眼看一遍产出**——跑完出片证明不了画面是对的。"
        );
    }
}

/// 造一段纯音并传上去。
///
/// **用正弦而不是随便什么声音，是为了让结果可判。** 音频听不了，但
/// 「输出里有没有这个频率」看频谱图就看得出来——锚点有没有真的生效，
/// 是个能验的问题；「生成的声音好不好听」不是，别假装验了。
fn upload_tone(env: &Env, name: &str, hz: u32, seconds: f64) -> String {
    let local = env.bundle.resolve(&format!("media/{name}.wav")).unwrap();
    let ok = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency={hz}:duration={seconds}:sample_rate=44100"
        ))
        .args(["-c:a", "pcm_s16le"])
        .arg(&local)
        .status()
        .expect("ffmpeg 起不来");
    assert!(ok.success(), "造纯音失败");
    let bytes = std::fs::read(&local).unwrap();
    env.comfy
        .upload_image(&format!("{name}.wav"), &bytes)
        .unwrap_or_else(|e| panic!("纯音（{} 字节）传不上去：{}", bytes.len(), e.message()))
}

/// audio 通道：**锚点能用，参考不能用**——这条守着这个区别。
///
/// 2026-09-05 真机实测（1kHz 纯音，Goertzel 量 1kHz 与邻频的能量比）：
///
/// | 走法 | 输出里 1kHz / 邻频 | 结论 |
/// |---|---|---|
/// | `guide.audio` 锚点 | ~4000 倍 | 素材进去了，而且主导了输出 |
/// | `references: kind=audio` | 0.5–1.9 倍 | 一点痕迹都没有 |
///
/// 参考那条**接线是对的**（`ref_audios: {"ref_audio_1": [...]}`，`audio_vae`
/// 也接了，图能跑、有音轨出来），就是模型不理它。所以 `input.audio` 通道
/// 标为已核验，而 `ref_audios` 槽位单独标 `verified: false`——两者不分开的话
/// 只能二选一：要么挡掉已经验通的锚点，要么把没生效的参考说成可用。
///
/// **这条测试验不了「声音好不好听」**，那要人听。它验的是「素材有没有真的
/// 进到输出里」，那个用频率能量比是可判的。
#[test]
fn audio_anchors_work_but_audio_references_are_still_refused() {
    let Some(env) = setup() else { return };
    const LONG_SHOT: i64 = 39;

    // 一、锚点：通道验过了，应当拼得出、跑得完。
    let tone = upload_tone(&env, "anchor_tone", 1000, 0.5);
    let mut anchored = base("A01", "reference");
    anchored.length_frames = LONG_SHOT;
    anchored.guides = vec![Guide {
        kind: GuideKind::Audio,
        at_frame: 0,
        asset_id: tone.clone(),
    }];
    let out = assemble_as(
        &env.set,
        &anchored,
        "real-acceptance/A01",
        Combination::Standard,
    )
    .unwrap_or_else(|e| panic!("audio 锚点组装失败：{}", e.message()));
    let load = out
        .graph
        .as_object()
        .unwrap()
        .iter()
        .find(|(k, _)| k.ends_with("_load"))
        .expect("图里应当有一个素材加载节点");
    assert_eq!(
        load.1["class_type"],
        json!("LoadAudio"),
        "走的不是 LoadAudio"
    );

    let sub = env
        .comfy
        .submit(&out.graph, "real-acceptance")
        .unwrap_or_else(|e| panic!("audio 锚点被判非法：{}", e.message()));
    let files = env
        .comfy
        .wait(&sub)
        .unwrap_or_else(|e| panic!("audio 锚点执行失败：{}", e.message()));
    assert!(!files.is_empty(), "audio 锚点跑完却没有产出");
    eprintln!("✅ audio 锚点（{}）：{} 个产物", sub.prompt_id, files.len());

    // 二、参考：槽位没核验，**必须在组装时就被挡下**，不许拼出来去烧 GPU。
    let mut with_audio_ref = base("A02", "reference");
    with_audio_ref.length_frames = LONG_SHOT;
    with_audio_ref.references = vec![Reference {
        kind: Medium::Audio,
        asset_id: tone,
        with_audio: false,
    }];
    let err = assemble_as(
        &env.set,
        &with_audio_ref,
        "real-acceptance/A02",
        Combination::Standard,
    )
    .expect_err("ref_audios 没核验，不该拼得出图");
    assert_eq!(err.code(), "model_contract_violation");
    assert!(
        err.message().contains("尚未核验"),
        "要说清是没核验：{}",
        err.message()
    );
    assert!(
        err.message().contains("没生效"),
        "要说清是「进去了但模型不理」，不是「进不去」：{}",
        err.message()
    );
}

/// 同一份声明组装两次逐字节相同——`studio.retry_stage`（内容没问题，
/// 原样重跑）靠这条成立，落盘的 debug 请求也靠它对得上。
#[test]
fn assembly_is_deterministic_against_the_real_fragment_library() {
    let Some(set) = fragments() else {
        eprintln!("跳过：读不到片段库");
        return;
    };
    let mut shot = base("S03", "reference");
    shot.references = vec![img("a.png"), img("b.png"), img("c.png")];
    shot.guides = vec![
        Guide {
            kind: GuideKind::Image,
            at_frame: 0,
            asset_id: "a.png".into(),
        },
        Guide {
            kind: GuideKind::Image,
            at_frame: -1,
            asset_id: "b.png".into(),
        },
    ];
    let once = assemble_as(&set, &shot, "d/S03", Combination::Standard).unwrap();
    let twice = assemble_as(&set, &shot, "d/S03", Combination::Standard).unwrap();
    assert_eq!(
        serde_json::to_string(&once.graph).unwrap(),
        serde_json::to_string(&twice.graph).unwrap()
    );
    assert_eq!(once.used, twice.used);
}

/// 每一份片段都读得出来、元数据完整——片段库本身的体检，不需要 ComfyUI。
#[test]
fn every_fragment_parses_with_complete_metadata() {
    let Some(set) = fragments() else {
        eprintln!("跳过：读不到片段库");
        return;
    };
    let backbone = set.backbone.as_ref().expect("缺骨架");
    assert!(!backbone.must_be_filled.is_empty(), "骨架没声明必填位置");
    assert!(!set.heads.is_empty() && !set.guides.is_empty() && !set.inputs.is_empty());

    let all = std::iter::once(backbone)
        .chain(set.heads.values())
        .chain(set.guides.values())
        .chain(set.inputs.values())
        .chain(set.overlays.values());
    for f in all {
        assert!(!f.nodes.is_empty(), "片段 {} 一个节点都没有", f.id);
        // 没核验的必须写明原因，否则错误消息里只有一句「原因未记录」。
        assert!(
            f.verified || f.unavailable_reason.is_some(),
            "片段 {} 未核验却没写原因",
            f.id
        );
    }
    // 每个 head 都要有 conditioning / latent 两个输出端口，否则接不上骨架。
    for h in set.heads.values() {
        assert!(
            h.outputs.contains_key("conditioning"),
            "head {} 缺 conditioning",
            h.id
        );
        assert!(h.outputs.contains_key("latent"), "head {} 缺 latent", h.id);
    }
    // 叠加层按 head 配套，挂到没有对应 head 的地方就是配错了。
    let mut overlays: Vec<&String> = set.overlays.keys().collect();
    overlays.sort();
    for id in overlays {
        assert!(set.heads.contains_key(id), "叠加层 {id} 没有对应的 head");
    }
}

/// SPEC-0015 §8.2：成片超分链路的真机验收。
///
/// 真渲一镜（原生画布短边 768 的竖屏），再用 `seedvr2/upscale` 基线把它超到
/// 交付规格 1080×1920，核对**尺寸、帧数、音轨**三样。
///
/// 帧数和音轨是重点：超分链路把视频拆成 `IMAGE` 序列再 `CreateVideo` 拼回去，
/// 拆错了会掉帧，`audio` 那根线接漏了会把声音丢掉——两样都是「图照样合法、
/// 跑照样成功」的静默错误。
///
/// 跑法：
/// ```text
/// COMFY_NODE=<入口> COMFY_TOKEN=<token> \
///   cargo test -p studio-pipeline --test real_comfy upscales_a_shot -- --nocapture
/// ```
#[test]
fn upscaling_a_shot_lands_on_the_delivery_spec() {
    let Some(env) = setup() else { return };

    // 竖屏原生画布：短边 768，长边 1344——正是 MiniMax 的「9:16」。
    const SRC_W: i64 = 768;
    const SRC_H: i64 = 1344;
    const OUT_W: i64 = 1080;
    const OUT_H: i64 = 1920;

    let swatch = upload_swatch(&env, "up_ref", "0x35506e");
    let mut shot = base("U01", "image");
    shot.width = SRC_W;
    shot.height = SRC_H;
    shot.first_frame = Some(swatch);
    let out = assemble_as(
        &env.set,
        &shot,
        "real-acceptance/U01",
        Combination::Standard,
    )
    .unwrap_or_else(|e| panic!("竖屏镜头组装失败：{}", e.message()));
    let sub = env
        .comfy
        .submit(&out.graph, "real-acceptance")
        .unwrap_or_else(|e| panic!("竖屏镜头被判为非法：{}", e.message()));
    let files = env
        .comfy
        .wait(&sub)
        .unwrap_or_else(|e| panic!("竖屏镜头渲染失败：{}", e.message()));
    let src = env.bundle.resolve("media/U01.mp4").unwrap();
    env.comfy.download(&files[0], &src).unwrap();
    assert_eq!(probe_frames(&src), FRAMES, "源片帧数就不对，后面没法比");
    eprintln!("✅ 源片：{SRC_W}x{SRC_H} {FRAMES} 帧");

    // 超分：走仓库里那份基线，不在测试里手写图。
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/workflows");
    let wf = Workflow::load(&dir, "seedvr2/upscale").expect("超分基线读不出来");
    wf.require_verified().expect("超分基线应当已核验");

    let bytes = std::fs::read(&src).unwrap();
    let remote = env
        .comfy
        .upload_image("U01.mp4", &bytes)
        .expect("成片传不上去");
    let mut params = serde_json::Map::new();
    params.insert("filename".into(), json!(remote));
    params.insert("width".into(), json!(OUT_W));
    params.insert("height".into(), json!(OUT_H));
    params.insert("seed".into(), json!(20260905));
    params.insert("output_prefix".into(), json!("real-acceptance/U01-up"));
    let graph = wf.apply(&params).unwrap();

    let sub = env
        .comfy
        .submit(&graph, "real-acceptance")
        .unwrap_or_else(|e| panic!("超分图被判为非法：{}", e.message()));
    let files = env
        .comfy
        .wait(&sub)
        .unwrap_or_else(|e| panic!("超分执行失败：{}", e.message()));
    assert_eq!(
        files.len(),
        1,
        "超分图里有 LoadVideo，输入回显不该被当成产物"
    );
    let up = env.bundle.resolve("media/U01-up.mp4").unwrap();
    env.comfy.download(&files[0], &up).unwrap();

    let info = studio_media::Media::new(&env.settings).probe(&up).unwrap();
    assert_eq!(
        (info.width as i64, info.height as i64),
        (OUT_W, OUT_H),
        "超分后的尺寸不是交付规格"
    );
    assert_eq!(probe_frames(&up), FRAMES, "超分掉帧了");
    assert!(
        info.has_audio,
        "超分把音轨丢了——CreateVideo 的 audio 那根线"
    );
    eprintln!(
        "✅ 超分：{}x{} {} 帧，音轨 {}",
        info.width,
        info.height,
        probe_frames(&up),
        info.audio_codec.as_deref().unwrap_or("无")
    );

    // 拼接那一步的前提：超分后的各镜参数一致，仍然能直接复制流。
    let second = env.bundle.resolve("media/U01-up-copy.mp4").unwrap();
    std::fs::copy(&up, &second).unwrap();
    assert!(
        studio_media::Media::new(&env.settings)
            .can_stream_copy(&[up.clone(), second])
            .unwrap(),
        "超分后的片段拼接不能直接复制流了——post 会退成重编码，慢且掉画质"
    );

    // bundle 是临时目录，测试一结束就没了。产物拷到 target/ 下留着——
    // 「跑完不等于画面对」，下一步是人眼看，看不到文件就等于没这一步。
    let keep =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/real-acceptance");
    std::fs::create_dir_all(&keep).unwrap();
    std::fs::copy(&src, keep.join("upscale-before.mp4")).unwrap();
    std::fs::copy(&up, keep.join("upscale-after.mp4")).unwrap();
    eprintln!(
        "\n跑完不等于画面对。人眼看一遍：\n  {}\n  {}",
        keep.join("upscale-after.mp4").display(),
        keep.join("upscale-before.mp4").display()
    );
}
