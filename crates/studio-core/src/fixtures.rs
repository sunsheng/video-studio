//! 一部完整作品的样例产物，覆盖九个阶段。
//!
//! 素材取自 2026-09-03 那次真实会话：「10 秒 5 个镜头的千岛湖游玩 vlog，
//! 主角 20 岁女性，长发黑发白裙板鞋，欢乐，30 度侧脸」。
//! 时长按镜头内容智能分配：1.4 / 2.0 / 2.4 / 2.0 / 2.2 = 10 秒。
//!
//! 这些样例同时被引擎测试和端到端测试使用，所以它们必须真的能通过 schema 校验——
//! 见 `schema::validate` 的测试。

use crate::contract::{AnswerOption, Confirmation, SelectionType};
use crate::stage::StageId;
use crate::Outputs;
use serde_json::json;

pub const CONCEPT: &str = "10秒5个镜头的千岛湖游玩vlog；主角20岁女性，长发、黑发、白裙、板鞋；欢乐；以30度侧脸为主要机位";
pub const TITLE: &str = "千岛湖，把快乐装进十秒";

fn wrap(stage: StageId, v: serde_json::Value) -> Outputs {
    let mut m = Outputs::new();
    m.insert(stage.output_key().to_string(), v);
    m
}

/// 该阶段一份合法的样例产物。
pub fn outputs(stage: StageId) -> Outputs {
    match stage {
        StageId::Idea => wrap(stage, json!({
            "title": TITLE,
            "logline": "20岁黑长发女孩穿白裙和板鞋，在千岛湖完成一段轻快、连续、以30度侧脸为视觉标识的五镜头游玩Vlog。",
            "platform": "抖音竖屏短视频",
            "audience": "18-35岁周末短途旅行、自然风景和轻松Vlog受众",
            "theme": "湖光、微风、轻松出游的即时快乐",
            "tone": "欢乐、清爽、轻旅行、自然阳光",
            "duration_seconds": 10,
            "shot_count": 5,
            "aspect_ratio": "9:16",
            "delivery_spec": "1080x1920, 30fps, H.264/AAC",
            "hook_0_3s": "船头掠过碧绿湖面切入女孩30度侧脸，她转头笑着把千岛湖映入画面。",
            "story_beats": [
                "镜头1：船头湖面建立地点与侧脸笑容",
                "镜头2：沿湖栈道小跑，白裙被风吹起",
                "镜头3：观景台举起手机取景，湖中群岛展开",
                "镜头4：递出冷饮与镜头轻碰杯，保持30度侧脸",
                "镜头5：夕阳前回头挥手，湖面远山收束"
            ],
            "success_metrics": [
                "五镜头严格合计10秒，转场可审计",
                "五镜头均能识别同一位20岁女性、黑长发、白裙、板鞋",
                "30度侧脸至少出现在4/5镜头",
                "9:16竖屏，无可读Logo与水印"
            ],
            "rights_and_safety_risks": [
                { "risk": "景区品牌或游客肖像进入画面", "level": "可规避", "mitigation": "无Logo构图，背景人物虚化" },
                { "risk": "在船边或栈道边缘做危险动作", "level": "可规避", "mitigation": "动作限于低速轻快行走，保持安全距离" }
            ],
            "assumptions": ["将用户原文『20色女性』按『20岁女性』理解"],
            "explicit_exclusions": ["不出现翻越、跳水、船舷外探身"],
            "protagonist": {
                "assumed_age": 20, "gender": "female", "hair": "长黑发",
                "wardrobe": "白裙、低帮板鞋、奶油色小斜挎包",
                "camera_signature": "约30度侧脸，面部可读"
            }
        })),

        StageId::Selection => wrap(stage, json!({
            "recommendation": "fast_vlog",
            "feasibility": {
                "score": "high",
                "rationale": "10秒、五镜头、单角色、单主色服装和明确侧脸角度都适合分镜化生成。",
                "model_control": "每镜头只保留一个主动作和一个主镜头运动，角色外观由 C01 锁定。",
                "production_cost": "低到中等"
            },
            "audience_fit": {
                "hook_strength": "强；前2秒用湖面掠过加30度侧脸笑容建立地点认知。",
                "benefit": "清爽风景、轻松穿搭与周末出游情绪",
                "retention_plan": "地点钩子→人物动作→湖岛展开→互动碰杯→挥手回收"
            },
            "publishing_risks": {
                "avoidable": ["景区Logo入镜", "危险动作误导"],
                "unacceptable": ["伪造真实人物肖像"],
                "user_decision": ["是否使用流行歌曲"]
            },
            "tradeoffs": "牺牲复杂行程信息和长旁白，换取10秒内更清楚的快乐情绪与可执行性。",
            "acceptance_metrics": ["地点3秒内可辨识", "角色五镜头一致", "总时长误差为0"]
        })),

        StageId::Script => wrap(stage, json!({
            "title": TITLE,
            "total_duration_seconds": 10,
            "shot_count": 5,
            "timing_rule": "按动作复杂度和信息量分配时长；五段连续片段无重叠，精确合计10秒",
            "language": "none",
            "story_arc": [
                { "beat_id": "beat_01", "start": 0,   "end": 1.4,  "duration_seconds": 1.4, "purpose": "地点钩子",
                  "visual": "船头掠过清透湖面，女孩以约30度侧脸快速入画并转头露出明亮笑容", "audio": "湖水轻拍船身，短促上扬音" },
                { "beat_id": "beat_02", "start": 1.4, "end": 3.4,  "duration_seconds": 2.0, "purpose": "人物动作",
                  "visual": "女孩沿湖边安全步道轻快小跑两步，白裙被风吹起", "audio": "轻快脚步、风声、裙摆沙沙声" },
                { "beat_id": "beat_03", "start": 3.4, "end": 5.8,  "duration_seconds": 2.4, "purpose": "景色展开",
                  "visual": "她在观景台举起手机取景，镜头从30度侧脸微移到湖中群岛", "audio": "相机快门声叠入轻亮木琴点" },
                { "beat_id": "beat_04", "start": 5.8, "end": 7.8,  "duration_seconds": 2.0, "purpose": "互动快乐",
                  "visual": "保持30度侧脸笑着举起冷饮，与镜头前的另一只手轻碰杯", "audio": "清脆碰杯声、短笑声、湖风" },
                { "beat_id": "beat_05", "start": 7.8, "end": 10.0, "duration_seconds": 2.2, "purpose": "情绪收束",
                  "visual": "夕阳暖光下女孩回头挥手，白裙和长发被风带动", "audio": "轻快挥手拟音，环境声自然尾收" }
            ],
            "segments": [
                { "segment_id": "s01", "start": 0,   "end": 1.4,  "speaker": "ambient", "text": "", "subtitle_text": "", "source": "核心模型原生环境声" },
                { "segment_id": "s02", "start": 1.4, "end": 3.4,  "speaker": "ambient", "text": "", "subtitle_text": "", "source": "核心模型原生环境声" },
                { "segment_id": "s03", "start": 3.4, "end": 5.8,  "speaker": "ambient", "text": "", "subtitle_text": "", "source": "核心模型原生环境声" },
                { "segment_id": "s04", "start": 5.8, "end": 7.8,  "speaker": "ambient", "text": "", "subtitle_text": "", "source": "核心模型原生环境声" },
                { "segment_id": "s05", "start": 7.8, "end": 10.0, "speaker": "ambient", "text": "", "subtitle_text": "", "source": "核心模型原生环境声" }
            ],
            "subtitle_policy": { "policy": "本版无口播、无字幕", "generated_from": [] },
            "audio_policy": { "primary": "核心模型原生音频", "external_music": "disabled",
                              "fallback": "原生音频不可用则结构化阻塞" },
            "safety_notes": ["步道动作保持低速，远离护栏和水边边缘"]
        })),

        StageId::Storyboard => wrap(stage, json!({
            "title": TITLE,
            "aspect_ratio": "9:16",
            "total_duration_seconds": 10,
            "shot_count": 5,
            "timing_basis": "时长由动作完成度和信息密度决定，不平均切分",
            "character_lock": {
                "subject": "20岁女性，长黑发",
                "wardrobe": "白裙、低帮板鞋、奶油色小斜挎包",
                "camera_signature": "主要使用约30度侧脸，面部可读",
                "safety": "不靠近水边危险边缘，不翻越护栏"
            },
            "shots": [
                { "shot_id": "sh01", "start": 0,   "end": 1.4, "duration_seconds": 1.4, "purpose": "地点钩子",
                  "shot_size": "广角远景", "angle": "略低机位", "camera_motion": "单一缓慢前推",
                  "lighting_color": "上午冷白自然光", "subject": "女孩30度侧脸入画",
                  "foreground": "船头栏杆", "midground": "清透湖面", "background": "层叠群岛与远山",
                  "action_chain": "船头掠过 -> 女孩转头 -> 露出笑容",
                  "first_frame": "湖面与船头", "last_frame": "侧脸笑容定格",
                  "sound": "湖水轻拍船身", "transition_to_next": "以水声作 J-cut" },
                { "shot_id": "sh02", "start": 1.4, "end": 3.4, "duration_seconds": 2.0, "purpose": "人物动作",
                  "shot_size": "中景", "angle": "平视侧前方30度", "camera_motion": "单一横向跟移",
                  "lighting_color": "顺光明亮", "subject": "女孩沿步道小跑",
                  "foreground": "步道栏杆虚化", "midground": "女孩全身", "background": "湖面与远岛",
                  "action_chain": "起步 -> 小跑两步 -> 裙摆扬起",
                  "first_frame": "脚步落下", "last_frame": "裙摆最高点",
                  "sound": "轻快脚步与风声", "transition_to_next": "顺动作切" },
                { "shot_id": "sh03", "start": 3.4, "end": 5.8, "duration_seconds": 2.4, "purpose": "景色展开",
                  "shot_size": "中近景转远景", "angle": "侧前方30度", "camera_motion": "单一缓慢摇移",
                  "lighting_color": "正午偏暖", "subject": "女孩举起手机取景",
                  "foreground": "手机边框", "midground": "女孩侧脸", "background": "湖中群岛",
                  "action_chain": "举起手机 -> 镜头随视线摇向群岛 -> 按下快门",
                  "first_frame": "抬手", "last_frame": "群岛全景",
                  "sound": "快门声与木琴点", "transition_to_next": "快门声作硬切" },
                { "shot_id": "sh04", "start": 5.8, "end": 7.8, "duration_seconds": 2.0, "purpose": "互动快乐",
                  "shot_size": "近景", "angle": "侧前方30度", "camera_motion": "固定机位",
                  "lighting_color": "下午暖光", "subject": "女孩举起冷饮碰杯",
                  "foreground": "另一只手的杯子", "midground": "女孩胸像", "background": "湖岛虚化",
                  "action_chain": "举杯 -> 轻碰 -> 笑出声",
                  "first_frame": "两杯靠近", "last_frame": "笑容与杯壁水珠",
                  "sound": "清脆碰杯声与短笑声", "transition_to_next": "笑声延续到下一镜" },
                { "shot_id": "sh05", "start": 7.8, "end": 10.0, "duration_seconds": 2.2, "purpose": "情绪收束",
                  "shot_size": "中远景", "angle": "平视背面转侧面", "camera_motion": "单一缓慢升高",
                  "lighting_color": "夕阳暖金", "subject": "女孩回头挥手",
                  "foreground": "草叶", "midground": "女孩全身", "background": "湖面与远山",
                  "action_chain": "转身 -> 回头 -> 挥手",
                  "first_frame": "背影", "last_frame": "挥手定格与天空留白",
                  "sound": "挥手拟音与环境尾音", "transition_to_next": "淡出黑场" }
            ]
        })),

        StageId::VisualAssets => wrap(stage, json!({
            "backend": "comfyui",
            "core_model_family": "minimax_h3",
            "strategy": "先用核心系列生成视觉开发片段，再用 ffmpeg 抽取参考帧",
            "fallback_policy": "核心系列不可用时结构化阻塞；不自动切换其他系列",
            "consistency_lock": {
                "character": "C01：20岁女性，长黑发，白裙，低帮板鞋，奶油色小斜挎包",
                "camera": "五镜头优先约30度侧脸，侧脸方向与主光方向连续",
                "environment": "千岛湖清透湖面、层叠群岛、远山",
                "typography": "禁止字幕、Logo、水印和可读文字"
            },
            "requests": [
                { "asset_id": "C01", "asset_kind": "character_card", "status": "planned", "width": 1024, "height": 1536,
                  "prompt": "20岁东亚女性，长黑发，白色连衣裙，低帮白色板鞋，奶油色小斜挎包，自然妆容，明亮笑容，约30度侧脸，真实自然光，干净背景，多角度一致性参考，无文字水印",
                  "applies_to": ["sh01", "sh02", "sh03", "sh04", "sh05"], "references": [] },
                { "asset_id": "SC01", "asset_kind": "scene_card", "status": "planned", "width": 1024, "height": 1536,
                  "prompt": "千岛湖清透湖面与游船船头，层叠群岛与远山，上午冷白自然光，竖构图，无文字水印",
                  "applies_to": ["sh01"], "references": [] },
                { "asset_id": "SC02", "asset_kind": "scene_card", "status": "planned", "width": 1024, "height": 1536,
                  "prompt": "湖边安全木质步道与护栏，两侧绿植，湖面在侧，顺光明亮，竖构图，无文字水印",
                  "applies_to": ["sh02"], "references": [] },
                { "asset_id": "SC03", "asset_kind": "scene_card", "status": "planned", "width": 1024, "height": 1536,
                  "prompt": "千岛湖观景台俯瞰群岛，开阔天空，下午暖光，竖构图，无文字水印",
                  "applies_to": ["sh03", "sh04", "sh05"], "references": [] },
                { "asset_id": "P01", "asset_kind": "prop_card", "status": "planned", "width": 1024, "height": 1024,
                  "prompt": "无品牌透明冷饮杯与无品牌手机，单物体参考图，自然光，无文字水印",
                  "applies_to": ["sh03", "sh04"], "references": ["C01"] }
            ]
        })),

        StageId::PromptPack => wrap(stage, json!({
            "core_model_family": "minimax_h3",
            "shots": [
                { "shot_id": "sh01", "workflow": "minimax_h3/t2v", "width": 1080, "height": 1920,
                  "length_frames": 42, "fps": 30, "seed": 101001, "references": ["C01", "SC01"],
                  "positive": "船头掠过清透湖面，20岁长黑发女孩白裙板鞋以约30度侧脸快速入画并转头露出明亮笑容，层叠群岛与远山，上午冷白自然光，竖屏9:16，电影质感",
                  "negative": "文字, 水印, logo, 多人, 畸形手部, 过曝, 低分辨率" },
                { "shot_id": "sh02", "workflow": "minimax_h3/i2v", "width": 1080, "height": 1920,
                  "length_frames": 60, "fps": 30, "seed": 101002, "references": ["C01", "SC02"],
                  "positive": "同一位长黑发白裙板鞋女孩沿湖边木质步道轻快小跑两步，白裙与长发被风吹起，约30度侧脸，顺光明亮，横向跟移，竖屏9:16",
                  "negative": "文字, 水印, logo, 危险动作, 翻越护栏, 畸形肢体" },
                { "shot_id": "sh03", "workflow": "minimax_h3/i2v", "width": 1080, "height": 1920,
                  "length_frames": 72, "fps": 30, "seed": 101003, "references": ["C01", "SC03", "P01"],
                  "positive": "女孩在观景台举起无品牌手机取景，镜头从30度侧脸缓慢摇移到湖中群岛，按下快门，下午暖光，竖屏9:16",
                  "negative": "文字, 水印, 品牌标识, 多人, 畸形手部" },
                { "shot_id": "sh04", "workflow": "minimax_h3/i2v", "width": 1080, "height": 1920,
                  "length_frames": 60, "fps": 30, "seed": 101004, "references": ["C01", "P01"],
                  "positive": "近景，女孩保持约30度侧脸笑着举起无品牌冷饮杯与画外另一只手轻碰杯，湖岛虚化背景，下午暖光，固定机位，竖屏9:16",
                  "negative": "文字, 水印, 品牌标识, 畸形手部, 液体飞溅过度" },
                { "shot_id": "sh05", "workflow": "minimax_h3/i2v", "width": 1080, "height": 1920,
                  "length_frames": 66, "fps": 30, "seed": 101005, "references": ["C01", "SC03"],
                  "positive": "夕阳暖金光下女孩回头挥手，白裙与长发被风带动，湖面与远山在身后，镜头缓慢升高，竖屏9:16",
                  "negative": "文字, 水印, logo, 多人, 过曝" }
            ]
        })),

        StageId::Render => wrap(stage, json!({
            "shots": [
                { "shot_id": "sh01", "node": "http://127.0.0.1:9001", "prompt_id": "p-sh01", "path": "media/sh01.mp4", "duration_seconds": 1.4 },
                { "shot_id": "sh02", "node": "http://127.0.0.1:9002", "prompt_id": "p-sh02", "path": "media/sh02.mp4", "duration_seconds": 2.0 },
                { "shot_id": "sh03", "node": "http://127.0.0.1:9003", "prompt_id": "p-sh03", "path": "media/sh03.mp4", "duration_seconds": 2.4 },
                { "shot_id": "sh04", "node": "http://127.0.0.1:9004", "prompt_id": "p-sh04", "path": "media/sh04.mp4", "duration_seconds": 2.0 },
                { "shot_id": "sh05", "node": "http://127.0.0.1:9005", "prompt_id": "p-sh05", "path": "media/sh05.mp4", "duration_seconds": 2.2 }
            ]
        })),

        StageId::Post => wrap(stage, json!({
            "video": "media/final.mp4",
            "cover": "media/cover.jpg",
            "subtitles": "media/subtitles.srt",
            "duration_seconds": 10.0,
            "aspect_ratio": "9:16"
        })),

        StageId::Review => wrap(stage, json!({
            "passed": true,
            "checks": [
                { "name": "总时长", "passed": true, "detail": "ffprobe 实测 10.00 秒，与剧本一致" },
                { "name": "画幅", "passed": true, "detail": "ffprobe 实测 1080x1920，9:16" },
                { "name": "镜头数与转场", "passed": true, "detail": "五段拼接，四处硬切可审计" }
            ]
        })),
    }
}

/// 该阶段的确认门样例。无门阶段返回 None。
pub fn confirmation(stage: StageId) -> Option<Confirmation> {
    let (prompt, approve_label) = match stage {
        StageId::Selection => ("是否按推荐方案推进：10秒五镜头轻快旅行Vlog，原生环境声，不使用外部歌曲？", "确认方案，进入剧本"),
        StageId::Script => ("是否确认按镜头内容智能分配时长的10秒剧本：1.4 / 2.0 / 2.4 / 2.0 / 2.2 秒？", "确认剧本，进入分镜"),
        StageId::Storyboard => ("是否确认这版五镜头分镜？五镜头均保持约30度侧脸与角色连续性。", "确认分镜，进入视觉资产"),
        StageId::VisualAssets => ("是否确认这组视觉资产计划？确认后按核心系列生成参考帧。", "确认资产计划，进入提示词"),
        StageId::PromptPack => ("是否确认这份逐镜头提示词？确认后开始占用 GPU 渲染。", "确认提示词，开始渲染"),
        _ => return None,
    };
    Some(Confirmation {
        prompt: prompt.to_string(),
        selection_type: SelectionType::Single,
        options: vec![
            AnswerOption::new("approve", approve_label),
            AnswerOption::revise("revise", "先修改再确认"),
        ],
    })
}

/// 阶段提交时的一句话摘要。
pub fn summary(stage: StageId) -> &'static str {
    match stage {
        StageId::Idea => "已把千岛湖十秒五镜头游玩Vlog整理为可执行brief",
        StageId::Selection => "推荐轻快纪实旅行Vlog方案，优先地点辨识与角色连续性",
        StageId::Script => "已按镜头内容智能分配时长，合计10秒",
        StageId::Storyboard => "五镜头分镜完成，锁定30度侧脸与安全动作",
        StageId::VisualAssets => "规划统一角色卡、三张场景卡与一张道具卡",
        StageId::PromptPack => "逐镜头提示词与workflow参数编译完成",
        StageId::Render => "五个镜头渲染完成",
        StageId::Post => "拼接、字幕与封面完成",
        StageId::Review => "验收通过",
    }
}
