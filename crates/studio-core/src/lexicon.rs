//! 受控词表。**这是词表层的唯一事实源。**
//!
//! 分镜里的镜头语言必须能落到视频模型真正听得懂的指令上。模型认的是
//! `[Push in]` 这类受控指令，不是「镜头缓缓推近，充满诗意」——所以分镜阶段
//! 就用枚举把话说死，提示词阶段只做机械翻译，中间不留自由发挥的余地。
//!
//! 这里的常量同时喂给三处，保证它们不会各说各话：
//!
//! - [`crate::schema`] 的 `enum`：Agent 提交时就被挡下
//! - 随包分发的方法层文档：词表表格由这里生成，不手写
//! - 模型能力卡：`camera_motion` 到各家指令的映射也在这里
//!
//! 禁用词表是反过来的约束：这些词对模型是噪声、对人是废话，出现即改。

/// 景别。从最远到最近。
pub const SHOT_SIZES: [&str; 7] = [
    "extreme_wide",
    "wide",
    "medium_wide",
    "medium",
    "medium_close",
    "close",
    "extreme_close",
];

/// 机位角度。
pub const ANGLES: [&str; 7] = [
    "eye_level",
    "low",
    "high",
    "overhead",
    "dutch",
    "over_shoulder",
    "pov",
];

/// 镜头运动。**每镜只能有一个**——两个以上的运动会让生成结果失控。
///
/// 这 15 项与 MiniMax 系列的运镜指令一一对应，见 [`MINIMAX_CAMERA_COMMANDS`]。
pub const CAMERA_MOTIONS: [&str; 15] = [
    "static",
    "push_in",
    "pull_out",
    "pan_left",
    "pan_right",
    "tilt_up",
    "tilt_down",
    "truck_left",
    "truck_right",
    "pedestal_up",
    "pedestal_down",
    "zoom_in",
    "zoom_out",
    "tracking",
    "handheld_shake",
];

/// 光源：光从哪来。
pub const LIGHTING_SOURCES: [&str; 8] = [
    "daylight",
    "moonlight",
    "practical",
    "firelight",
    "fluorescent",
    "overcast",
    "mixed",
    "artificial",
];

/// 光型：光怎么打在主体上。
pub const LIGHTING_KEYS: [&str; 8] = [
    "soft",
    "hard",
    "top",
    "side",
    "back",
    "rim",
    "silhouette",
    "bottom",
];

/// 镜头职能。三者都不占的镜头不该存在。
pub const SHOT_FUNCTIONS: [&str; 3] = ["change_emotion", "advance_action", "raise_pressure"];

/// 拍点在结构里的位置。
pub const BEAT_TYPES: [&str; 6] = ["hook", "setup", "develop", "turn", "payoff", "resolve"];

/// `camera_motion` 到 MiniMax 系列运镜指令的映射。
///
/// 提示词阶段照这张表翻译，不要自己造词——写在表里的指令才有效果。
/// 官方建议一次组合不超过三个指令，而分镜每镜只允许一个主运动，
/// 所以正常路径上永远只会用到一个。
pub const MINIMAX_CAMERA_COMMANDS: [(&str, &str); 15] = [
    ("static", "[Static shot]"),
    ("push_in", "[Push in]"),
    ("pull_out", "[Pull out]"),
    ("pan_left", "[Pan left]"),
    ("pan_right", "[Pan right]"),
    ("tilt_up", "[Tilt up]"),
    ("tilt_down", "[Tilt down]"),
    ("truck_left", "[Truck left]"),
    ("truck_right", "[Truck right]"),
    ("pedestal_up", "[Pedestal up]"),
    ("pedestal_down", "[Pedestal down]"),
    ("zoom_in", "[Zoom in]"),
    ("zoom_out", "[Zoom out]"),
    ("tracking", "[Tracking shot]"),
    ("handheld_shake", "[Shake]"),
];

/// 景别的中文说法。
pub const SHOT_SIZE_LABELS: [(&str, &str); 7] = [
    ("extreme_wide", "大远景：人在环境里很小，交代地理关系"),
    ("wide", "远景：人全身加大量环境"),
    ("medium_wide", "中远景：膝盖以上，看得清动作和环境"),
    ("medium", "中景：腰以上，动作与表情兼顾"),
    ("medium_close", "中近景：胸以上，表情为主"),
    ("close", "近景：肩以上，脸占主要面积"),
    ("extreme_close", "特写：眼睛、手、物件的局部"),
];

/// 机位角度的中文说法。
pub const ANGLE_LABELS: [(&str, &str); 7] = [
    ("eye_level", "平视：中性，没有理由就用它"),
    ("low", "仰拍：主体显得有力、有压迫感"),
    ("high", "俯拍：主体显得弱小、被审视"),
    ("overhead", "顶视：图形化，强调布局而非人"),
    ("dutch", "斜角：画面失衡，表示不安"),
    ("over_shoulder", "过肩：交代两人关系与视线"),
    ("pov", "主观视角：观众成为角色的眼睛"),
];

/// 光源的中文说法。
pub const LIGHTING_SOURCE_LABELS: [(&str, &str); 8] = [
    ("daylight", "自然日光"),
    ("moonlight", "月光：冷、暗、方向单一"),
    ("practical", "实用光：画面里能看见的灯，最容易出效果"),
    ("firelight", "火光：暖、跳动、方向低"),
    ("fluorescent", "荧光灯：偏绿、平、无情绪"),
    ("overcast", "阴天：大面积柔光，无明显方向"),
    ("mixed", "混合光：冷暖并存，需说明各自方向"),
    ("artificial", "人造光：影棚或不明来源的布光"),
];

/// 光型的中文说法。
pub const LIGHTING_KEY_LABELS: [(&str, &str); 8] = [
    ("soft", "柔光：阴影边缘柔和，皮肤平整"),
    ("hard", "硬光：阴影边缘锐利，反差大"),
    ("top", "顶光：眼窝落影，压迫感"),
    ("side", "侧光：一半亮一半暗，立体"),
    ("back", "逆光：主体压暗，边缘发光"),
    ("rim", "轮廓光：只有一道边光，把人从暗背景里分离"),
    ("silhouette", "剪影：主体全黑，只剩形状"),
    ("bottom", "底光：反常规照明，用于失常与恐怖"),
];

/// 镜头职能的中文说法。
pub const SHOT_FUNCTION_LABELS: [(&str, &str); 3] = [
    ("change_emotion", "改变情绪：看完这一镜，观众的感受变了"),
    ("advance_action", "推进动作：看完这一镜，故事往前走了一步"),
    (
        "raise_pressure",
        "增加压力：看完这一镜，观众更担心或更期待了",
    ),
];

/// 拍点类型的中文说法。
pub const BEAT_TYPE_LABELS: [(&str, &str); 6] = [
    ("hook", "勾住：给一个不看下去会难受的理由"),
    ("setup", "交代：谁、在哪、什么状态"),
    ("develop", "推进：信息量最大的地方"),
    ("turn", "转折：观众的预期被改写"),
    ("payoff", "兑现：前面埋的东西在这里结算"),
    ("resolve", "收束：给一个能停住的画面"),
];

/// 角色卡的视图。前五个是**必需**的，见 [`required_views`]。
///
/// 「多角度」不是形容词：一张大头照锁不住服装，一张全身照锁不住脸。
/// 转身图（turnaround）是这一行的标准做法，缺一个角度就少一处可比对的
/// 参照，下游只能靠猜。
pub const CHARACTER_VIEWS: [&str; 8] = [
    "front_full",
    "three_quarter",
    "profile",
    "back",
    "face_close",
    "expressions",
    "hands_props",
    "wardrobe_detail",
];

pub const CHARACTER_VIEW_LABELS: [(&str, &str); 8] = [
    ("front_full", "正面全身，自然站姿，中性表情（主视图）"),
    ("three_quarter", "四分之三侧身全身"),
    ("profile", "正侧面全身"),
    ("back", "背面全身：发型与服装背面"),
    ("face_close", "面部特写，中性表情"),
    ("expressions", "表情组：本片主导情绪 2–3 种"),
    ("hands_props", "手部与关键道具的持握关系"),
    ("wardrobe_detail", "服装材质与关键配饰特写"),
];

/// 场景卡的视图。前四个必需。
pub const SCENE_VIEWS: [&str; 6] = [
    "establishing",
    "key_angle",
    "reverse_angle",
    "detail",
    "lighting_variants",
    "empty_plate",
];

pub const SCENE_VIEW_LABELS: [(&str, &str); 6] = [
    ("establishing", "建立镜头广角，交代空间全貌（主视图）"),
    ("key_angle", "主机位角度：分镜里用得最多的那个"),
    ("reverse_angle", "反打角度，保证轴线两侧都成立"),
    ("detail", "材质、纹理或标志性局部"),
    (
        "lighting_variants",
        "剧本要求的时间光线变体：日 / 黄昏 / 夜",
    ),
    ("empty_plate", "空景，无人物，便于人物合成与对位"),
];

/// 道具卡的视图。四个都必需——道具少一个面，持握关系就对不上。
pub const PROP_VIEWS: [&str; 4] = ["front", "side", "in_use", "scale_reference"];

pub const PROP_VIEW_LABELS: [(&str, &str); 4] = [
    ("front", "正面（主视图）"),
    ("side", "侧面"),
    ("in_use", "使用状态：谁怎么拿着它"),
    ("scale_reference", "比例参照：与手或人体的相对大小"),
];

/// 资产类型。
pub const ASSET_KINDS: [&str; 5] = [
    "character_card",
    "scene_card",
    "prop_card",
    "safety_reference",
    "style_reference",
];

/// 某类资产的全部合法视图。参照类资产不强制多视图，返回空。
pub fn views_for(asset_kind: &str) -> &'static [&'static str] {
    match asset_kind {
        "character_card" => &CHARACTER_VIEWS,
        "scene_card" => &SCENE_VIEWS,
        "prop_card" => &PROP_VIEWS,
        _ => &[],
    }
}

/// 某类资产**必须**齐全的视图。缺一个就不放行。
pub fn required_views(asset_kind: &str) -> &'static [&'static str] {
    match asset_kind {
        "character_card" => &CHARACTER_VIEWS[..5],
        "scene_card" => &SCENE_VIEWS[..4],
        "prop_card" => &PROP_VIEWS,
        _ => &[],
    }
}

/// 主视图：先出、单独出的那一个。
///
/// 并行生成八个视图，出来的是八个长得像但不是同一个人的角色——
/// 这与后端无关，是生成模型的固有性质。其余视图一律以它为参考图。
pub fn anchor_view(asset_kind: &str) -> Option<&'static str> {
    match asset_kind {
        "character_card" => Some("front_full"),
        "scene_card" => Some("establishing"),
        "prop_card" => Some("front"),
        _ => None,
    }
}

/// 全部视图取值的并集，schema 的 enum 用它。
pub const ALL_VIEWS: [&str; 18] = [
    "front_full",
    "three_quarter",
    "profile",
    "back",
    "face_close",
    "expressions",
    "hands_props",
    "wardrobe_detail",
    "establishing",
    "key_angle",
    "reverse_angle",
    "detail",
    "lighting_variants",
    "empty_plate",
    "front",
    "side",
    "in_use",
    "scale_reference",
];

/// `camera_motion` 的中文说法，写给人看的那一列。
pub const CAMERA_MOTION_LABELS: [(&str, &str); 15] = [
    ("static", "固定机位"),
    ("push_in", "推近（机身前移）"),
    ("pull_out", "拉远（机身后移）"),
    ("pan_left", "左摇"),
    ("pan_right", "右摇"),
    ("tilt_up", "上仰"),
    ("tilt_down", "下俯"),
    ("truck_left", "左移（机身平移）"),
    ("truck_right", "右移（机身平移）"),
    ("pedestal_up", "升镜（机身抬高）"),
    ("pedestal_down", "降镜（机身降低）"),
    ("zoom_in", "变焦推近（镜头组变焦）"),
    ("zoom_out", "变焦拉远"),
    ("tracking", "跟移"),
    ("handheld_shake", "手持晃动"),
];

/// 出现即改。对模型是噪声，对人是废话——它们不描述任何可拍的东西。
///
/// 判定是子串匹配，所以这里只放足够长、不会误伤的词。
pub const BANNED_TIER1: [&str; 18] = [
    "cinematic",
    "epic",
    "stunning",
    "masterpiece",
    "best quality",
    "high quality",
    "ultra detailed",
    "8k",
    "4k",
    "电影感",
    "电影质感",
    "史诗",
    "大片质感",
    "唯美",
    "高清",
    "超高清",
    "精致细腻",
    "画质极佳",
];

/// 同一段里出现两个以上才算问题。单独用未必错，堆在一起就是形容词汤。
pub const BANNED_TIER2: [&str; 12] = [
    "beautiful",
    "dynamic",
    "dramatic",
    "atmospheric",
    "gorgeous",
    "breathtaking",
    "氛围感",
    "质感",
    "高级感",
    "震撼",
    "梦幻",
    "绝美",
];

/// 在一段文本里找出所有 Tier 1 禁用词，大小写不敏感。
pub fn banned_tier1_hits(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    BANNED_TIER1
        .iter()
        .copied()
        .filter(|w| lower.contains(&w.to_lowercase()))
        .collect()
}

/// 在一段文本里找出所有 Tier 2 禁用词，大小写不敏感。
pub fn banned_tier2_hits(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    BANNED_TIER2
        .iter()
        .copied()
        .filter(|w| lower.contains(&w.to_lowercase()))
        .collect()
}

/// 把分镜的 `camera_motion` 翻译成 MiniMax 系列的运镜指令。
pub fn minimax_camera_command(motion: &str) -> Option<&'static str> {
    MINIMAX_CAMERA_COMMANDS
        .iter()
        .find(|(k, _)| *k == motion)
        .map(|(_, v)| *v)
}

/// 全部词表，按名字取「取值 + 中文说法」。
///
/// 随包分发的方法层文档里的词表表格由这里生成，不手写——
/// 手写的表格迟早会和 schema 的 `enum` 对不上。
pub fn vocabulary(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    Some(match name {
        "shot_size" => &SHOT_SIZE_LABELS,
        "angle" => &ANGLE_LABELS,
        "camera_motion" => &CAMERA_MOTION_LABELS,
        "lighting_source" => &LIGHTING_SOURCE_LABELS,
        "lighting_key" => &LIGHTING_KEY_LABELS,
        "shot_function" => &SHOT_FUNCTION_LABELS,
        "beat_type" => &BEAT_TYPE_LABELS,
        "character_view" => &CHARACTER_VIEW_LABELS,
        "scene_view" => &SCENE_VIEW_LABELS,
        "prop_view" => &PROP_VIEW_LABELS,
        _ => return None,
    })
}

/// 每个词表的取值集合，用来和 [`vocabulary`] 对账。
pub fn values(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "shot_size" => &SHOT_SIZES,
        "angle" => &ANGLES,
        "camera_motion" => &CAMERA_MOTIONS,
        "lighting_source" => &LIGHTING_SOURCES,
        "lighting_key" => &LIGHTING_KEYS,
        "shot_function" => &SHOT_FUNCTIONS,
        "beat_type" => &BEAT_TYPES,
        "character_view" => &CHARACTER_VIEWS,
        "scene_view" => &SCENE_VIEWS,
        "prop_view" => &PROP_VIEWS,
        _ => return None,
    })
}

/// 有中文说法的词表名。
pub const VOCABULARIES: [&str; 10] = [
    "shot_size",
    "angle",
    "camera_motion",
    "lighting_source",
    "lighting_key",
    "shot_function",
    "beat_type",
    "character_view",
    "scene_view",
    "prop_view",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 映射表漏一项，提示词阶段就会在那一种运镜上无声地退回自由发挥。
    #[test]
    fn every_camera_motion_has_a_minimax_command_and_a_label() {
        for m in CAMERA_MOTIONS {
            assert!(
                minimax_camera_command(m).is_some(),
                "{m} 没有对应的 MiniMax 运镜指令"
            );
            assert!(
                CAMERA_MOTION_LABELS.iter().any(|(k, _)| *k == m),
                "{m} 没有中文说法"
            );
        }
        assert_eq!(MINIMAX_CAMERA_COMMANDS.len(), CAMERA_MOTIONS.len());
        assert_eq!(CAMERA_MOTION_LABELS.len(), CAMERA_MOTIONS.len());
    }

    /// 词表少一条中文说法，生成出来的文档就会缺一行——
    /// Agent 于是不知道那个取值是什么意思，也就不会用它。
    #[test]
    fn every_vocabulary_value_has_a_label() {
        for name in VOCABULARIES {
            let values = values(name).unwrap_or_else(|| panic!("{name} 没有取值集合"));
            let labels = vocabulary(name).unwrap_or_else(|| panic!("{name} 没有中文说法"));
            assert_eq!(
                values.len(),
                labels.len(),
                "{name} 的取值与中文说法数量对不上"
            );
            for v in values {
                assert!(
                    labels.iter().any(|(k, _)| k == v),
                    "{name} 的取值 {v} 没有中文说法"
                );
            }
            for (k, text) in labels {
                assert!(values.contains(k), "{name} 的中文说法里有多余的取值 {k}");
                assert!(!text.is_empty(), "{name}.{k} 的中文说法是空的");
            }
        }
    }

    #[test]
    fn no_duplicates_in_any_vocabulary() {
        for (name, words) in [
            ("shot_size", SHOT_SIZES.to_vec()),
            ("angle", ANGLES.to_vec()),
            ("camera_motion", CAMERA_MOTIONS.to_vec()),
            ("lighting_source", LIGHTING_SOURCES.to_vec()),
            ("lighting_key", LIGHTING_KEYS.to_vec()),
            ("shot_function", SHOT_FUNCTIONS.to_vec()),
            ("beat_type", BEAT_TYPES.to_vec()),
            ("banned_tier1", BANNED_TIER1.to_vec()),
            ("banned_tier2", BANNED_TIER2.to_vec()),
        ] {
            let mut sorted = words.clone();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(before, sorted.len(), "{name} 词表里有重复项");
        }
    }

    #[test]
    fn banned_words_are_found_case_insensitively() {
        assert_eq!(banned_tier1_hits("A CINEMATIC shot"), vec!["cinematic"]);
        assert!(banned_tier1_hits("很有电影感的画面").contains(&"电影感"));
        assert!(banned_tier1_hits("船头切开湖面，她抬手别过碎发").is_empty());
        assert!(banned_tier2_hits("氛围感拉满，质感很好").len() >= 2);
    }
}
