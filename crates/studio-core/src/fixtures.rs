//! 一部完整作品的样例产物，覆盖十个阶段。
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

pub const CONCEPT: &str =
    "10秒5个镜头的千岛湖游玩vlog；主角20岁女性，长发、黑发、白裙、板鞋；欢乐；以30度侧脸为主要机位";
pub const TITLE: &str = "千岛湖，把快乐装进十秒";

/// 角色卡的身份锁：视频用的那段身份锁**逐字包含在内**，再补上视频提示词
/// 用不到、但出图必须锁死的东西（脸型、肤色、瞳色）。
///
/// 前半段必须与 [`IDENTITY_LOCK`] 逐字相同——校验会查这一条。
const CHARACTER_IDENTITY: &str = "20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，鹅蛋脸，浅麦色皮肤，深棕色瞳孔，左眉尾有一颗小痣";

/// 所有卡片视图共享的拍摄约束。卡片是**测量用的参考素材**，不是好看的剧照——
/// 戏剧性打光留给成片。
const CARD_CONSTRAINTS: &str =
    "中性灰底，均匀柔光，无阴影投射，主体完整入画不裁切，画面中不出现任何文字、标志或水印";

/// 组装一张多视图卡片。
///
/// `views` 的第一项是主视图：它先出、单独出，其余视图都以它为参考图。
/// 每个视图的提示词都以同一段 `identity` **逐字**开头——一致性靠复制，
/// 不靠每个视图重新描述一遍。
fn card(
    asset_id: &str,
    asset_kind: &str,
    identity: &str,
    aspect: &str,
    applies_to: &[&str],
    views: &[(&str, &str)],
) -> serde_json::Value {
    let anchor = views[0].0;
    let items: Vec<serde_json::Value> = views
        .iter()
        .enumerate()
        .map(|(i, (view, angle))| {
            let mut v = json!({
                "view": view,
                "is_anchor": i == 0,
                "aspect": aspect,
                "prompt": format!("{identity}。{angle}。{CARD_CONSTRAINTS}。画幅 {aspect}。"),
                "status": "planned",
            });
            if i > 0 {
                v["derived_from"] = json!(anchor);
            }
            v
        })
        .collect();
    json!({
        "asset_id": asset_id,
        "asset_kind": asset_kind,
        "identity_prompt": identity,
        "applies_to": applies_to,
        "views": items,
    })
}

fn wrap(stage: StageId, v: serde_json::Value) -> Outputs {
    let mut m = Outputs::new();
    m.insert(stage.output_key().to_string(), v);
    m
}

/// 该阶段一份合法的样例产物。
pub fn outputs(stage: StageId) -> Outputs {
    match stage {
        StageId::Idea => wrap(
            stage,
            json!({
                "title": TITLE,
                "logline": "20岁黑长发女孩在千岛湖的十秒五镜头游玩Vlog。三个方案在「怎么讲」上分岔，平台、时长、画幅不变。",
                "platform": "抖音竖屏短视频",
                "audience": "18-35岁周末短途旅行、自然风景和轻松Vlog受众",
                "theme": "湖光、微风、轻松出游的即时快乐",
                "tone": "欢乐、清爽、轻旅行、自然阳光",
                "duration_seconds": 10,
                "shot_count": 5,
                "aspect_ratio": "9:16",
                "delivery_spec": "1080x1920, 30fps, H.264/AAC",
                "concepts": [
                    {
                        "concept_id": "c1",
                        "logline": "跟着她的脚走完一段湖边路，十秒里只做一件事：把「在这儿很自在」拍出来。",
                        "angle": "以人物动作为主线，风景是背景",
                        "hook_0_3s": "船头切开清透湖面，她以约30度侧脸快速入画并转头笑出来——0.6 秒内认出地点和人。",
                        "story_beats": [
                            "镜头1：船头湖面建立地点与侧脸笑容",
                            "镜头2：沿湖栈道小跑，白裙被风吹起",
                            "镜头3：观景台举起手机取景，湖中群岛展开",
                            "镜头4：递出冷饮与镜头轻碰杯，保持30度侧脸",
                            "镜头5：夕阳前回头挥手，湖面远山收束"
                        ],
                        "tradeoff": "牺牲行程信息与地标全貌，换十秒内更连贯的人物情绪"
                    },
                    {
                        "concept_id": "c2",
                        "logline": "从一只沾着水的板鞋开始，一路往上，最后才让人看见她的脸。",
                        "angle": "先藏人后露脸，用局部特写攒出好奇",
                        "hook_0_3s": "特写：一只白色板鞋踩上湿木板，水从鞋边挤出来——看不见人，只听见笑声。",
                        "story_beats": [
                            "镜头1：板鞋踩上湿木栈道的特写，画外一声轻笑",
                            "镜头2：裙摆与手中冷饮杯的中近景，仍不露脸",
                            "镜头3：她把手机举高取景，镜头顺手臂上摇",
                            "镜头4：终于给到30度侧脸，她正对着湖笑",
                            "镜头5：拉远，人和千岛湖一起进画"
                        ],
                        "tradeoff": "前四秒不给地点，靠好奇心留人；地点辨识度让给了悬念"
                    },
                    {
                        "concept_id": "c3",
                        "logline": "十秒里只有湖，人只是最后一秒的一个剪影。",
                        "angle": "风景片，人物退到点缀位置",
                        "hook_0_3s": "船头劈开的水把上午的光切成两半，群岛在雾里一层层退开。",
                        "story_beats": [
                            "镜头1：船头切水的广角，光在水面碎开",
                            "镜头2：群岛层层退开的横摇",
                            "镜头3：水面倒影里的云",
                            "镜头4：栈道尽头的空景",
                            "镜头5：夕阳里她的背影剪影，抬手挥了一下"
                        ],
                        "tradeoff": "牺牲人物代入感，换更强的画面质感；对「跟着谁玩」这个期待没有回应"
                    }
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
            }),
        ),

        StageId::Selection => wrap(
            stage,
            json!({
                "candidates": [
                    {
                        "concept_id": "c1",
                        "feasibility": {
                            "score": "high",
                            "rationale": "单角色、单套服装、明确的侧脸角度，五镜都能各自独立生成；\
                                          每镜一个主动作一个主运镜，是三个方案里最好控的。"
                        },
                        "audience_fit": {
                            "hook_strength": "strong",
                            "rationale": "0.6 秒就同时给出地点和人，刷到的人不需要等就知道这是什么。"
                        },
                        "risks": ["动作类镜头多，模型可能把小跑拍成滑步"],
                        "verdict": "recommended"
                    },
                    {
                        "concept_id": "c2",
                        "feasibility": {
                            "score": "medium",
                            "rationale": "四个镜头都要精确控制「不露脸」，模型很容易自作主张把脸转过来；\
                                          一旦露早了，整个悬念结构就塌了。"
                        },
                        "audience_fit": {
                            "hook_strength": "medium",
                            "rationale": "局部特写开场对停留有帮助，但前四秒不给地点，\
                                          冲着「千岛湖」刷进来的人可能提前划走。"
                        },
                        "risks": ["露脸时机不可控", "前四秒没有地点信息，完播率风险"],
                        "verdict": "viable"
                    },
                    {
                        "concept_id": "c3",
                        "feasibility": {
                            "score": "high",
                            "rationale": "纯风景，没有人物一致性问题，是三个里最容易出片的。"
                        },
                        "audience_fit": {
                            "hook_strength": "weak",
                            "rationale": "用户明确要的是「游玩 vlog」，风景片对不上这个期待；\
                                          好看，但看完不知道跟着谁玩了一趟。"
                        },
                        "risks": ["与用户原始需求偏离"],
                        "verdict": "not_advised"
                    }
                ],
                "recommendation": "c1",
                "tradeoffs": "选 c1 就放弃了 c2 那种「憋一下再给脸」的张力——\
                              c1 第一秒就把牌全摊开，后面靠动作和情绪撑，不靠悬念。\
                              代价是它更平，没有一个让人「哦」一声的转折点。",
                "publishing_risks": {
                    "avoidable": ["景区Logo入镜", "危险动作误导"],
                    "unacceptable": ["伪造真实人物肖像"],
                    "user_decision": ["是否使用流行歌曲"]
                },
                "acceptance_metrics": ["地点3秒内可辨识", "角色五镜头一致", "总时长误差为0"]
            }),
        ),

        StageId::Script => wrap(
            stage,
            json!({
                "title": TITLE,
                "total_duration_seconds": 10,
                "shot_count": 5,
                "timing_rule": "按动作复杂度和信息量分配时长；五段连续片段无重叠，精确合计10秒",
                "hook_at_seconds": 0.6,
                "language": "none",
                "story_arc": [
                    { "beat_id": "beat_01", "beat_type": "hook", "start": 0,   "end": 1.4,  "duration_seconds": 1.4, "purpose": "0.6 秒内让人认出这是千岛湖，并给出一张会笑的脸",
                      "visual": "船头切开清透湖面，女孩以约30度侧脸快速入画并转头露出笑容", "audio": "湖水拍打船身的持续哗声" },
                    { "beat_id": "beat_02", "beat_type": "setup", "start": 1.4, "end": 3.4,  "duration_seconds": 2.0, "purpose": "交代人物状态：她在这里是自在的",
                      "visual": "女孩沿湖边木质步道轻快小跑两步，白裙被风吹起", "audio": "板鞋落在木板上的两声闷响与风声" },
                    { "beat_id": "beat_03", "beat_type": "develop", "start": 3.4, "end": 5.8,  "duration_seconds": 2.4, "purpose": "把视线从人交给景，这是全片信息量最大的一拍",
                      "visual": "她在观景台举起手机取景，镜头从30度侧脸摇向湖中群岛", "audio": "快门声，随后远处游船汽笛" },
                    { "beat_id": "beat_04", "beat_type": "payoff", "start": 5.8, "end": 7.8,  "duration_seconds": 2.0, "purpose": "把风景兑现成可分享的快乐：有人在跟她一起",
                      "visual": "保持30度侧脸笑着举起冷饮，与画外另一只手轻碰杯", "audio": "两只玻璃杯轻碰的脆响与短促笑声" },
                    { "beat_id": "beat_05", "beat_type": "resolve", "start": 7.8, "end": 10.0, "duration_seconds": 2.2, "purpose": "收在一个可以停住的画面上，留出天空的空白",
                      "visual": "夕阳暖光下女孩回头挥手，白裙和长发被风带动", "audio": "衣料摩擦声与远处水浪低频，自然尾收" }
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
            }),
        ),

        StageId::Storyboard => wrap(
            stage,
            json!({
                "title": TITLE,
                "aspect_ratio": "9:16",
                "total_duration_seconds": 10,
                "shot_count": 5,
                "timing_basis": "时长由动作完成度和信息密度决定，不平均切分",
                "character_lock": {
                    "identity_lock": IDENTITY_LOCK,
                    "camera_signature": "主要使用约30度侧脸，面部可读",
                    "safety": "不靠近水边危险边缘，不翻越护栏"
                },
                "shots": [
                    { "shot_id": "sh01", "start": 0, "end": 1.4, "duration_seconds": 1.4,
                      "purpose": "地点钩子：0.6 秒内认出千岛湖，同时给出一张会笑的脸",
                      "shot_function": "advance_action",
                      "three_facts": [
                          "船行带起的风把碎发吹到她嘴角",
                          "她抬手把碎发别到耳后，指尖在耳廓停住",
                          "船头切开水面的持续哗声"
                      ],
                      "shot_size": "wide", "angle": "low", "camera_motion": "push_in",
                      "lighting_source": "daylight", "lighting_key": "soft", "color_tone": "上午冷白，低对比",
                      "subject": "女孩以约30度侧脸入画",
                      "foreground": "船头栏杆", "midground": "清透湖面", "background": "层叠群岛与远山",
                      "action_chain": "船头切开水面 -> 她转头 -> 笑容展开",
                      "first_frame": "湖面与船头", "last_frame": "侧脸笑容定格",
                      "audio": { "ambient": "湖水拍打船身的持续哗声，低频风声", "foley": "碎发拂过脸颊、衣料轻响", "music": "none" },
                      "sound": "湖水拍打船身", "transition_to_next": "以水声作 J-cut" },
                    { "shot_id": "sh02", "start": 1.4, "end": 3.4, "duration_seconds": 2.0,
                      "purpose": "交代人物状态：她在这里是自在的",
                      "shot_function": "advance_action",
                      "three_facts": [
                          "湖风从左侧推来，裙摆和发梢一起向右扬",
                          "落地时脚踝先内扣再蹬直，重心前倾半步",
                          "板鞋鞋底拍在木板上的两声闷响"
                      ],
                      "shot_size": "medium", "angle": "eye_level", "camera_motion": "tracking",
                      "lighting_source": "daylight", "lighting_key": "soft", "color_tone": "顺光明亮，低对比",
                      "subject": "女孩沿木质步道小跑",
                      "foreground": "步道栏杆虚化", "midground": "女孩全身", "background": "湖面与远岛",
                      "action_chain": "起步 -> 小跑两步 -> 裙摆扬到最高点",
                      "first_frame": "脚步落下", "last_frame": "裙摆最高点",
                      "audio": { "ambient": "开阔湖面的风声", "foley": "板鞋踩木板的两声闷响、裙摆抖动", "music": "none" },
                      "sound": "脚步与风声", "transition_to_next": "顺动作切" },
                    { "shot_id": "sh03", "start": 3.4, "end": 5.8, "duration_seconds": 2.4,
                      "purpose": "把视线从人交给景，全片信息量最大的一镜",
                      "shot_function": "change_emotion",
                      "three_facts": [
                          "正午的光很硬，手机屏幕反着湖面的白",
                          "她眯了一下眼，拇指在快门键上停了半秒",
                          "快门声，随后是远处游船的汽笛"
                      ],
                      "shot_size": "medium_close", "angle": "eye_level", "camera_motion": "pan_right",
                      "lighting_source": "daylight", "lighting_key": "hard", "color_tone": "正午偏暖，高对比",
                      "subject": "女孩举起手机取景",
                      "foreground": "手机边框", "midground": "女孩侧脸", "background": "湖中群岛",
                      "action_chain": "举起手机 -> 镜头随视线摇向群岛 -> 按下快门",
                      "first_frame": "抬手", "last_frame": "群岛全景",
                      "audio": { "ambient": "观景台上的风与零星人声", "foley": "手机快门声", "music": "none" },
                      "sound": "快门声", "transition_to_next": "快门声作硬切" },
                    { "shot_id": "sh04", "start": 5.8, "end": 7.8, "duration_seconds": 2.0,
                      "purpose": "把风景兑现成可分享的快乐：有人跟她在一起",
                      "shot_function": "change_emotion",
                      "three_facts": [
                          "杯壁的冷凝水滑到虎口",
                          "指节收紧握住杯身，笑的时候肩膀轻轻一耸",
                          "两只玻璃杯轻碰的一声脆响"
                      ],
                      "shot_size": "close", "angle": "eye_level", "camera_motion": "static",
                      "lighting_source": "daylight", "lighting_key": "side", "color_tone": "下午暖金",
                      "subject": "女孩举起冷饮与画外的手碰杯",
                      "foreground": "画外伸入的另一只杯子", "midground": "女孩胸像", "background": "湖岛虚化",
                      "action_chain": "举杯 -> 轻碰 -> 笑出声",
                      "first_frame": "两杯靠近", "last_frame": "笑容与杯壁水珠",
                      "audio": { "ambient": "湖风与远处水声", "foley": "玻璃杯轻碰的脆响、短促笑声", "music": "none" },
                      "sound": "碰杯声与笑声", "transition_to_next": "笑声延续到下一镜" },
                    { "shot_id": "sh05", "start": 7.8, "end": 10.0, "duration_seconds": 2.2,
                      "purpose": "收在一个能停住的画面上，给天空留白",
                      "shot_function": "change_emotion",
                      "three_facts": [
                          "逆光把发丝勾出暖金边，风把裙摆吹成一道弧",
                          "她转身时先动眼神再动脖子，抬手挥到肩高",
                          "衣料摩擦声与远处水浪的低频"
                      ],
                      "shot_size": "medium_wide", "angle": "eye_level", "camera_motion": "pedestal_up",
                      "lighting_source": "daylight", "lighting_key": "back", "color_tone": "夕阳暖金，逆光",
                      "subject": "女孩回头挥手",
                      "foreground": "草叶", "midground": "女孩全身", "background": "湖面与远山",
                      "action_chain": "转身 -> 回头 -> 挥手",
                      "first_frame": "背影", "last_frame": "挥手定格，天空留白",
                      "audio": { "ambient": "傍晚的风与远处水浪低频", "foley": "衣料摩擦、手臂划过空气", "music": "none" },
                      "sound": "挥手拟音与环境尾音", "transition_to_next": "淡出黑场" }
                ]
            }),
        ),

        StageId::VisualAssets => wrap(
            stage,
            json!({
                "backend": "comfyui",
                "core_model_family": "minimax_h3",
                "strategy": "先用核心系列生成视觉开发片段，再用 ffmpeg 抽取参考帧",
                "fallback_policy": "核心系列不可用时结构化阻塞；不自动切换其他系列",
                "consistency_lock": {
                    "character": IDENTITY_LOCK,
                    "camera": "五镜头优先约30度侧脸，侧脸方向与主光方向连续",
                    "environment": "千岛湖清透湖面、层叠群岛、远山",
                    "typography": "禁止字幕、Logo、水印和可读文字"
                },
                "assets": [
                    card(
                        "C01", "character_card", CHARACTER_IDENTITY, "9:16",
                        &["sh01", "sh02", "sh03", "sh04", "sh05"],
                        &[
                            ("front_full", "正面全身，自然站姿，双手自然垂放，中性表情，双脚完整入画"),
                            ("three_quarter", "四分之三侧身全身，身体转约30度，脸朝镜头，全身入画"),
                            ("profile", "正侧面全身，鼻尖朝画左，全身入画"),
                            ("back", "背面全身，可见发尾长度与连衣裙背面剪裁、挎包背带走向"),
                            ("face_close", "面部特写，中性表情，双眼平视镜头，可见发际线与耳廓"),
                        ],
                    ),
                    card(
                        "SC01", "scene_card", "千岛湖清透湖面与游船船头，层叠群岛与远山，上午冷白自然光", "9:16",
                        &["sh01"],
                        &[
                            ("establishing", "广角建立镜头，船头与湖面各占一半，群岛在远景"),
                            ("key_angle", "船头栏杆后方的主机位，视线越过栏杆看向湖面"),
                            ("reverse_angle", "反打：从船头朝船舱方向，湖面退到画外"),
                            ("detail", "船头切开水面的浪线与栏杆金属表面的水痕特写"),
                        ],
                    ),
                    card(
                        "SC02", "scene_card", "湖边木质步道与护栏，两侧绿植，湖面在侧，顺光明亮", "9:16",
                        &["sh02"],
                        &[
                            ("establishing", "广角建立镜头，步道从画面下方延伸到远处"),
                            ("key_angle", "与步道同向的跟拍机位，护栏在画左"),
                            ("reverse_angle", "反打：从步道尽头回看来路"),
                            ("detail", "木板拼缝、护栏立柱与脚下青苔的特写"),
                        ],
                    ),
                    card(
                        "SC03", "scene_card", "千岛湖观景台，俯瞰层叠群岛与开阔天空，下午暖光", "9:16",
                        &["sh03", "sh04", "sh05"],
                        &[
                            ("establishing", "广角建立镜头，观景台护栏在前景，群岛铺满中远景"),
                            ("key_angle", "站在护栏内侧朝群岛的主机位"),
                            ("reverse_angle", "反打：从群岛方向回看观景台与台阶"),
                            ("detail", "护栏石材纹理与远处水面反光的特写"),
                        ],
                    ),
                    card(
                        "P01", "prop_card", "无品牌透明冷饮杯，杯壁带冷凝水，无任何文字与标志", "1:1",
                        &["sh03", "sh04"],
                        &[
                            ("front", "正面单物体，杯身完整入画"),
                            ("side", "侧面，可见杯壁厚度与杯底"),
                            ("in_use", "一只手握住杯身中段，拇指压在杯壁，可见握持角度"),
                            ("scale_reference", "与成年人手掌并排，交代相对大小"),
                        ],
                    )
                ]
            }),
        ),

        StageId::PromptPack => wrap(
            stage,
            json!({
                "core_model_family": "minimax_h3",
                "identity_lock": {
                    "character": IDENTITY_LOCK,
                    "environment": "千岛湖清透湖面、层叠群岛、远山",
                    "typography": "画面中不出现任何文字、标志或水印"
                },
                "shots": [
                    { "shot_id": "sh01", "head": "reference", "width": 768, "height": 1344,
                      "length_frames": 56, "fps": 24, "seed": 101001,
                      "references": [{ "kind": "image", "asset_id": "C01" },
                                     { "kind": "image", "asset_id": "SC01" }],
                      "positive": "船头切开清透湖面，一位20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，以约30度侧脸快速入画并转头露出笑容。船行的风把碎发吹到她嘴角，她抬手把碎发别到耳后。层叠群岛与远山在后景。上午冷白自然光，柔光顺照，低对比。[Push in] 镜头缓慢前推。一镜到底，不切场景，人物保持在画面中央，画面中不出现任何文字、标志或水印。",
                      "audio": "环境声：湖水拍打船身的持续哗声，低频风声。拟音：碎发拂过脸颊、衣料轻响。无对白，无音乐。" },
                    { "shot_id": "sh02", "head": "reference", "width": 768, "height": 1344,
                      "length_frames": 39, "fps": 24, "seed": 101002,
                      "references": [{ "kind": "image", "asset_id": "C01" },
                                     { "kind": "image", "asset_id": "SC02" }],
                      "guides": [{ "kind": "image", "at_frame": 0, "asset_id": "sh01.tail" }],
                      "positive": "一位20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，沿湖边木质步道轻快小跑两步。湖风从左侧推来，裙摆和发梢一起向右扬；落地时脚踝先内扣再蹬直，重心前倾半步。顺光明亮，低对比，背景是湖面与远岛。[Tracking shot] 镜头横向跟移，与她同速。一镜到底，不切场景，人物保持在画面中央，画面中不出现任何文字、标志或水印。",
                      "audio": "环境声：开阔湖面的风声。拟音：板鞋踩在木板上的两声闷响、裙摆抖动。无对白，无音乐。" },
                    { "shot_id": "sh03", "head": "reference", "width": 768, "height": 1344,
                      "length_frames": 56, "fps": 24, "seed": 101003,
                      "references": [{ "kind": "image", "asset_id": "C01" },
                                     { "kind": "image", "asset_id": "SC03" },
                                     { "kind": "image", "asset_id": "P01" }],
                      "positive": "一位20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，在观景台举起一部无品牌手机取景，眯了一下眼，拇指在快门键上停半秒后按下。正午硬光，手机屏幕反着湖面的白，高对比。[Pan right] 镜头向右缓慢横摇，从她的侧脸摇到湖中群岛。一镜到底，不切场景，画面中不出现任何文字、标志或水印。",
                      "audio": "环境声：观景台上的风与零星人声。拟音：手机快门声，随后远处游船汽笛。无对白，无音乐。" },
                    { "shot_id": "sh04", "head": "reference", "width": 768, "height": 1344,
                      "length_frames": 39, "fps": 24, "seed": 101004,
                      "references": [{ "kind": "image", "asset_id": "C01" },
                                     { "kind": "image", "asset_id": "P01" }],
                      "positive": "近景：一位20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，举起一只无品牌透明冷饮杯，与画外伸入的另一只杯子轻碰。杯壁的冷凝水滑到虎口，指节收紧握住杯身，笑的时候肩膀轻轻一耸。下午暖金侧光，背景的湖岛虚化。[Static shot] 固定机位，镜头不移动。一镜到底，不切场景，画面中不出现任何文字、标志或水印。",
                      "audio": "环境声：湖风与远处水声。拟音：两只玻璃杯轻碰的一声脆响、短促笑声。无对白，无音乐。" },
                    { "shot_id": "sh05", "head": "reference", "width": 768, "height": 1344,
                      "length_frames": 56, "fps": 24, "seed": 101005,
                      "references": [{ "kind": "image", "asset_id": "C01" },
                                     { "kind": "image", "asset_id": "SC03" }],
                      "positive": "夕阳逆光下，一位20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包，转身回头向镜头挥手，先动眼神再动脖子，抬手挥到肩高。逆光把发丝勾出暖金边，风把裙摆吹成一道弧，湖面与远山在身后。[Pedestal up] 镜头缓慢升高。一镜到底，不切场景，画面中不出现任何文字、标志或水印。",
                      "audio": "环境声：傍晚的风与远处水浪低频。拟音：衣料摩擦、手臂划过空气。无对白，无音乐。" }
                ]
            }),
        ),

        StageId::Preview => wrap(
            stage,
            json!({
                "shots": [
                    { "shot_id": "sh01", "node": "http://127.0.0.1:9001", "prompt_id": "pv-sh01", "path": "media/preview/sh01.mp4", "width": 480, "height": 854, "duration_seconds": 1.4 },
                    { "shot_id": "sh02", "node": "http://127.0.0.1:9002", "prompt_id": "pv-sh02", "path": "media/preview/sh02.mp4", "width": 480, "height": 854, "duration_seconds": 2.0 },
                    { "shot_id": "sh03", "node": "http://127.0.0.1:9003", "prompt_id": "pv-sh03", "path": "media/preview/sh03.mp4", "width": 480, "height": 854, "duration_seconds": 2.4 },
                    { "shot_id": "sh04", "node": "http://127.0.0.1:9004", "prompt_id": "pv-sh04", "path": "media/preview/sh04.mp4", "width": 480, "height": 854, "duration_seconds": 2.0 },
                    { "shot_id": "sh05", "node": "http://127.0.0.1:9005", "prompt_id": "pv-sh05", "path": "media/preview/sh05.mp4", "width": 480, "height": 854, "duration_seconds": 2.2 }
                ]
            }),
        ),

        StageId::Render => wrap(
            stage,
            json!({
                "shots": [
                    { "shot_id": "sh01", "node": "http://127.0.0.1:9001", "prompt_id": "p-sh01", "path": "media/sh01.mp4", "duration_seconds": 1.4 },
                    { "shot_id": "sh02", "node": "http://127.0.0.1:9002", "prompt_id": "p-sh02", "path": "media/sh02.mp4", "duration_seconds": 2.0 },
                    { "shot_id": "sh03", "node": "http://127.0.0.1:9003", "prompt_id": "p-sh03", "path": "media/sh03.mp4", "duration_seconds": 2.4 },
                    { "shot_id": "sh04", "node": "http://127.0.0.1:9004", "prompt_id": "p-sh04", "path": "media/sh04.mp4", "duration_seconds": 2.0 },
                    { "shot_id": "sh05", "node": "http://127.0.0.1:9005", "prompt_id": "p-sh05", "path": "media/sh05.mp4", "duration_seconds": 2.2 }
                ]
            }),
        ),

        StageId::Post => wrap(
            stage,
            json!({
                "video": "media/final.mp4",
                "cover": "media/cover.jpg",
                "subtitles": "media/subtitles.srt",
                "duration_seconds": 10.0,
                "aspect_ratio": "9:16"
            }),
        ),

        StageId::Review => wrap(
            stage,
            json!({
                "passed": true,
                "checks": [
                    { "name": "总时长", "kind": "technical", "passed": true, "detail": "ffprobe 实测 10.00 秒，与剧本一致" },
                    { "name": "画幅", "kind": "technical", "passed": true, "detail": "ffprobe 实测 1080x1920，9:16" },
                    { "name": "镜头数与转场", "kind": "technical", "passed": true, "detail": "五段拼接，四处硬切可审计" }
                ],
                "content_review": {
                    "items": [
                        { "criterion": "hook", "verdict": "met", "at_seconds": 0.6,
                          "evidence": "0.6 秒她已经转过头笑出来，同一帧里船头切开的水面和远处群岛都在，地点和人一起给到" },
                        { "criterion": "information_density", "verdict": "partially_met", "at_seconds": 5.8,
                          "evidence": "第四镜的碰杯除了「她很开心」没有给出新信息，删掉它观众不会察觉少了什么" },
                        { "criterion": "pacing", "verdict": "met", "at_seconds": 3.4,
                          "evidence": "第三镜给到 2.4 秒，是全片最长的一镜，正好够举手机、摇过去、按快门三个动作走完" },
                        { "criterion": "consistency", "verdict": "met", "at_seconds": 7.8,
                          "evidence": "第五镜的白裙、板鞋、奶油色挎包与第一镜逐一对得上，侧脸角度也仍在30度附近" },
                        { "criterion": "brief_metrics", "verdict": "met", "at_seconds": 9.9,
                          "evidence": "brief 要求30度侧脸至少出现在4/5镜，实际五镜都有；全片无可读文字与水印" }
                    ],
                    "summary": "最强的是前三秒的地点加人物一次给足；最弱的是第四镜没有承担新信息"
                }
            }),
        ),
    }
}

/// 该阶段的确认门样例。无门阶段返回 None。
pub fn confirmation(stage: StageId) -> Option<Confirmation> {
    let (prompt, approve_label) = match stage {
        // 选题这道门是唯一一个「多选一」的门：三个通过选项各对应一个方案，
        // 用户挑哪个由控制面记进产物的 _gate_choice，下游据此推进。
        StageId::Selection => {
            return Some(Confirmation {
                prompt: "三个方案，选一个推进：\n\
                         A（推荐）跟着她走——第一秒就给地点和笑脸，最好控，但比较平；\n\
                         B 先藏后露——从一只湿板鞋开始攒好奇，第四秒才给脸，\
                         张力最强，但露脸时机不可控，前四秒没有地点；\n\
                         C 纯风景——最容易出片，但看完不知道跟着谁玩了一趟。"
                    .to_string(),
                selection_type: SelectionType::Single,
                options: vec![
                    AnswerOption::new("c1", "A：跟着她走（推荐）"),
                    AnswerOption::new("c2", "B：先藏后露"),
                    AnswerOption::new("c3", "C：纯风景"),
                    AnswerOption::revise("revise", "三个都不满意，重想"),
                ],
            });
        }
        StageId::Script => (
            "是否确认按镜头内容智能分配时长的10秒剧本：1.4 / 2.0 / 2.4 / 2.0 / 2.2 秒？",
            "确认剧本，进入分镜",
        ),
        StageId::Storyboard => (
            "是否确认这版五镜头分镜？五镜头均保持约30度侧脸与角色连续性。",
            "确认分镜，进入视觉资产",
        ),
        StageId::VisualAssets => (
            "是否确认这组视觉资产计划？确认后按核心系列生成参考帧。",
            "确认资产计划，进入提示词",
        ),
        StageId::PromptPack => (
            "是否确认这份逐镜头提示词？确认后开始占用 GPU 渲染。",
            "确认提示词，开始渲染",
        ),
        StageId::Preview => (
            "480p 预览已生成，构图与内容是否符合预期？确认后开始正式 1080p 渲染。",
            "预览符合预期，开始正式渲染",
        ),
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

/// 身份锁：跨镜头逐字复用的那段外观描述。
///
/// 一致性不靠「每次都描述得很详细」，靠同一个字符串被逐字复制。
/// 提示词里出现的是它本身，不是它的近义改写。
pub const IDENTITY_LOCK: &str =
    "20岁东亚女性，长黑发及胸，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包";

/// 阶段提交时的一句话摘要。
pub fn summary(stage: StageId) -> &'static str {
    match stage {
        StageId::Idea => "已把千岛湖十秒五镜头游玩Vlog整理为可执行brief",
        StageId::Selection => "推荐轻快纪实旅行Vlog方案，优先地点辨识与角色连续性",
        StageId::Script => "已按镜头内容智能分配时长，合计10秒",
        StageId::Storyboard => "五镜头分镜完成，锁定30度侧脸与安全动作",
        StageId::VisualAssets => "规划统一角色卡、三张场景卡与一张道具卡",
        StageId::PromptPack => "逐镜头提示词与workflow参数编译完成",
        StageId::Preview => "五个镜头 480p 预览生成完成",
        StageId::Render => "五个镜头渲染完成",
        StageId::Post => "拼接、字幕与封面完成",
        StageId::Review => "验收通过",
    }
}

/// 这些样例同时是随包分发的黄金样例——Agent 会照着它们写。
///
/// 所以它们得自己先达标：禁用词一个不许有，身份锁逐字一致，
/// 每镜三个物理事实。样例松一寸，产出松一尺。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon;

    /// 收集样例里所有会被 Agent 当范文抄的自由文本。
    fn prose(stage: StageId) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let walk = |v: &serde_json::Value, path: String, out: &mut Vec<(String, String)>| {
            let mut stack = vec![(v.clone(), path)];
            while let Some((node, p)) = stack.pop() {
                match node {
                    serde_json::Value::String(s) => out.push((p, s)),
                    serde_json::Value::Array(a) => {
                        for (i, item) in a.into_iter().enumerate() {
                            stack.push((item, format!("{p}[{i}]")));
                        }
                    }
                    serde_json::Value::Object(m) => {
                        for (k, item) in m {
                            stack.push((item, format!("{p}.{k}")));
                        }
                    }
                    _ => {}
                }
            }
        };
        for (k, v) in outputs(stage) {
            walk(&v, k, &mut out);
        }
        out
    }

    #[test]
    fn no_stage_fixture_contains_a_banned_word() {
        for stage in StageId::all() {
            for (path, s) in prose(stage) {
                let hits = lexicon::banned_tier1_hits(&s);
                assert!(
                    hits.is_empty(),
                    "{stage} 的样例 {path} 里有禁用词 {hits:?}：{s}"
                );
            }
        }
    }

    /// 每一镜的正向提示词都必须原样带上身份锁——差一个字都算漂移。
    #[test]
    fn every_prompt_repeats_the_identity_lock_verbatim() {
        let pack = outputs(StageId::PromptPack);
        let shots = pack["prompt_pack"]["shots"].as_array().unwrap();
        assert_eq!(shots.len(), 5);
        for shot in shots {
            let positive = shot["positive"].as_str().unwrap();
            assert!(
                positive.contains(IDENTITY_LOCK),
                "{} 的提示词没有逐字带上身份锁",
                shot["shot_id"]
            );
        }
    }

    /// minimax_h3 的基线没有 negative 绑定，写了会被静默丢弃——
    /// 样例必须示范正确写法：把约束写进正向提示词。
    #[test]
    fn prompts_for_minimax_carry_constraints_positively() {
        let pack = outputs(StageId::PromptPack);
        for shot in pack["prompt_pack"]["shots"].as_array().unwrap() {
            // 片段化的系列用 head 而不是 workflow，两者互斥。
            let Some(head) = shot["head"].as_str() else {
                continue;
            };
            assert!(
                shot.get("negative").is_none(),
                "{} 用的是 head {head}，minimax_h3 不吃 negative",
                shot["shot_id"]
            );
            let positive = shot["positive"].as_str().unwrap();
            assert!(
                positive.contains("一镜到底") && positive.contains("不出现任何文字"),
                "{} 缺少正向写法的连续性与排版约束",
                shot["shot_id"]
            );
            assert!(
                shot["audio"].as_str().is_some_and(|a| !a.is_empty()),
                "{} 没写声音——核心系列是音视频联合生成，留空等于放弃原生音频",
                shot["shot_id"]
            );
        }
    }

    /// 三个物理事实是分镜不干巴的最低线。
    #[test]
    fn every_shot_states_three_physical_facts() {
        let sb = outputs(StageId::Storyboard);
        for shot in sb["storyboard"]["shots"].as_array().unwrap() {
            let facts = shot["three_facts"].as_array().unwrap();
            assert!(
                facts.len() >= 3,
                "{} 只有 {} 条物理事实",
                shot["shot_id"],
                facts.len()
            );
            for f in facts {
                let s = f.as_str().unwrap();
                assert!(
                    s.chars().count() >= 6,
                    "{} 里的物理事实太短：{s}",
                    shot["shot_id"]
                );
            }
            assert!(
                lexicon::CAMERA_MOTIONS.contains(&shot["camera_motion"].as_str().unwrap()),
                "{} 的运镜不在受控词表里",
                shot["shot_id"]
            );
        }
    }
}
