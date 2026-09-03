//! 从已确认剧本生成字幕。
//!
//! 字幕**只能**来自确认过的剧本文本——后期阶段不新编内容。
//! 没有口播的作品不会产出字幕文件。

use serde_json::Value;

/// 把剧本的 segments 转成 SRT。没有任何字幕文本时返回 None。
pub fn from_script(script: &Value) -> Option<String> {
    let segments = script.get("segments")?.as_array()?;
    let mut out = String::new();
    let mut n = 0;
    for seg in segments {
        let text = seg
            .get("subtitle_text")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                seg.get("text")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
            })?;
        let start = seg.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let end = seg.get("end").and_then(|v| v.as_f64()).unwrap_or(start);
        n += 1;
        out.push_str(&format!(
            "{n}\n{} --> {}\n{}\n\n",
            stamp(start),
            stamp(end),
            text.trim()
        ));
    }
    if n == 0 {
        None
    } else {
        Some(out)
    }
}

fn stamp(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ambient_only_scripts_produce_no_subtitles() {
        let script = &studio_core::fixtures::outputs(studio_core::StageId::Script)["script"];
        assert!(
            from_script(script).is_none(),
            "无口播无字幕的作品不该凭空生成字幕"
        );
    }

    #[test]
    fn spoken_segments_become_srt() {
        let script = json!({ "segments": [
            { "segment_id": "s01", "start": 0,   "end": 1.4, "subtitle_text": "千岛湖，今天好开心！" },
            { "segment_id": "s02", "start": 1.4, "end": 3.4, "text": "风好舒服" }
        ]});
        let srt = from_script(&script).unwrap();
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:01,400\n千岛湖，今天好开心！"));
        assert!(srt.contains("2\n00:00:01,400 --> 00:00:03,400\n风好舒服"));
    }

    #[test]
    fn timestamps_cross_minutes_and_hours() {
        assert_eq!(stamp(0.0), "00:00:00,000");
        assert_eq!(stamp(61.5), "00:01:01,500");
        assert_eq!(stamp(3661.25), "01:01:01,250");
        assert_eq!(stamp(-1.0), "00:00:00,000");
    }
}
