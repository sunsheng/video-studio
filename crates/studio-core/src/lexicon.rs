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
