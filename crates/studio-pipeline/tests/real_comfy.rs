//! 真机验收：组装出来的图，交给**真的 ComfyUI** 判。
//!
//! 这一组测试有环境前置条件，**CI 里不会跑**：
//!
//! - `COMFY_NODE` 指向一个可达的 ComfyUI 入口
//! - 该实例上装着 `minimax_h3` 的权重
//! - 本机有 `ffmpeg`（用来现造参考图）
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

/// 一镜的规格：小而快，只为验接线，不为出好看的画面。
const W: i64 = 640;
const H: i64 = 384;
const FRAMES: i64 = 22; // 17k+5 网格上的第二档
const FPS: f64 = 24.0;

struct Env {
    _dir: tempfile::TempDir,
    bundle: Bundle,
    comfy: Comfy,
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
/// `clip` 锚点要的是帧序列，走 LoadVideo；那条通道还没跑通过一整镜。
/// 以前它被错误地映射到 LoadImage——图能过校验、也能出片，只是锚的是
/// 一张静帧而不是一段。这条测试守着「宁可挡下，不要静默降级」。
#[test]
fn an_unverified_input_channel_is_refused_rather_than_downgraded() {
    let Some(env) = setup() else { return };
    let input_video = env.set.inputs.get("video").expect("片段库缺 input.video");
    if input_video.verified {
        eprintln!("跳过：input.video 已经核验过了，这条测试的前提不再成立");
        return;
    }

    let mut shot = base("S09", "reference");
    shot.guides = vec![Guide {
        kind: GuideKind::Clip,
        at_frame: 0,
        asset_id: "S08.tail22".into(),
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
/// 帧数落在 `17k+5` 网格上——clip 锚点的长度吃同一套网格（SPEC-0014 V5）。
fn upload_clip(env: &Env, name: &str) -> String {
    let local = env.bundle.resolve(&format!("media/{name}.mp4")).unwrap();
    let seconds = FRAMES as f64 / FPS;
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

    let clip = upload_clip(&env, "anchor_clip");
    eprintln!("锚点视频上传后的文件名：{clip}");

    // 1. clip 锚点：把一段帧序列锚在第 0 帧。
    let mut anchored = base("V01", "reference");
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
        eprintln!("✅ {name}（{}）：{} 个产物", sub.prompt_id, files.len());
    }

    if !already {
        eprintln!(
            "\n两条都跑通了。现在可以把 assets/workflows/minimax_h3/fragments/\
             input.video.json 的 bindings_verified 改成 true，并在 SOURCE-fragments.md \
             里记下这次的 run。**改之前先人眼看一遍产出**——跑完出片证明不了画面是对的。"
        );
    }
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
