//! 媒体处理：拼接、转码、字幕、封面、元数据。
//!
//! 全部通过外部 ffmpeg / ffprobe 进程完成，本进程不链接任何编解码库。
//!
//! **两个可执行文件都不要求在 PATH 中。** 查找顺序见 [`studio_engine::Settings`]：
//! bundle 的 `.env` 优先，然后是程序目录的 `.env`、进程环境、`config.toml`，
//! 最后才轮到 PATH。找不到时报 `tool_unavailable`，并在 remedy 里说明去哪配。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use studio_core::{Result, StudioError};
use studio_engine::Settings;

/// 一个外部工具的解析结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// 找过哪些地方——报错和 doctor 都要用。
    pub looked_in: Vec<String>,
}

pub struct Media<'a> {
    settings: &'a Settings,
}

impl<'a> Media<'a> {
    pub fn new(settings: &'a Settings) -> Media<'a> {
        Media { settings }
    }

    fn resolve(&self, tool: &str) -> Result<PathBuf> {
        self.settings
            .tool_path(tool)
            .ok_or_else(|| StudioError::ToolUnavailable {
                tool: tool.to_string(),
                looked_in: self.settings.searched.clone(),
            })
    }

    /// 体检用：找得到就顺便取版本号。
    pub fn probe_tool(&self, tool: &str) -> ToolStatus {
        match self.settings.tool_path(tool) {
            None => ToolStatus {
                name: tool.into(),
                found: false,
                path: None,
                version: None,
                looked_in: self.settings.searched.clone(),
            },
            Some(p) => {
                let version = Command::new(&p)
                    .arg("-version")
                    .output()
                    .ok()
                    .and_then(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .next()
                            .map(|l| l.trim().to_string())
                    });
                ToolStatus {
                    name: tool.into(),
                    found: true,
                    path: Some(p.display().to_string()),
                    version,
                    looked_in: self.settings.searched.clone(),
                }
            }
        }
    }

    /// 读取媒体元数据。验收必须基于实测，不能靠推断。
    pub fn probe(&self, file: &Path) -> Result<MediaInfo> {
        let ffprobe = self.resolve("ffprobe")?;
        if !file.is_file() {
            return Err(StudioError::ArtifactMissing {
                path: file.display().to_string(),
            });
        }
        let out = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(file)
            .output()
            .map_err(|e| StudioError::internal(format!("执行 ffprobe 失败：{e}")))?;
        if !out.status.success() {
            return Err(StudioError::internal(format!(
                "ffprobe 退出码 {:?}：{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| StudioError::internal(format!("ffprobe 输出无法解析：{e}")))?;
        Ok(MediaInfo::from_ffprobe(&v))
    }

    /// 各片段能否直接 copy 拼接：编码、分辨率、帧率、音轨都一致才行。
    ///
    /// 这个判断**只用 ffprobe**。判得出来就不必让 ffmpeg 重编码一遍——
    /// 重编码既慢又掉画质，而且五个十秒片段本来就是同一套参数出的。
    pub fn can_stream_copy(&self, parts: &[PathBuf]) -> Result<bool> {
        let mut first: Option<MediaInfo> = None;
        for p in parts {
            let info = self.probe(p)?;
            match &first {
                None => first = Some(info),
                Some(a) => {
                    if !same_stream(a, &info) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(first.is_some())
    }

    /// 按分镜顺序拼接。
    ///
    /// `reencode` 为 None 时先用 ffprobe 看各片段是否一致：一致就 `-c copy`
    /// 直接拼，不一致才重编码。给 Some 则强制。
    pub fn concat_auto(
        &self,
        parts: &[PathBuf],
        out: &Path,
        reencode: Option<bool>,
    ) -> Result<bool> {
        let reencode = match reencode {
            Some(v) => v,
            None => !self.can_stream_copy(parts)?,
        };
        self.concat(parts, out, reencode)?;
        Ok(reencode)
    }

    /// 按分镜顺序拼接。用 concat demuxer，不重编码时最快。
    pub fn concat(&self, parts: &[PathBuf], out: &Path, reencode: bool) -> Result<()> {
        let ffmpeg = self.resolve("ffmpeg")?;
        for p in parts {
            if !p.is_file() {
                return Err(StudioError::ArtifactMissing {
                    path: p.display().to_string(),
                });
            }
        }
        let list = out.with_extension("concat.txt");
        let body: String = parts
            .iter()
            .map(|p| {
                format!(
                    "file '{}'\n",
                    p.display().to_string().replace('\'', "'\\''")
                )
            })
            .collect();
        std::fs::write(&list, body)
            .map_err(|e| StudioError::internal(format!("写 concat 清单失败：{e}")))?;

        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list);
        if reencode {
            cmd.args([
                "-c:v", "libx264", "-preset", "medium", "-crf", "20", "-c:a", "aac",
            ]);
        } else {
            cmd.args(["-c", "copy"]);
        }
        cmd.arg(out);
        run(cmd, "ffmpeg concat")?;
        let _ = std::fs::remove_file(&list);
        Ok(())
    }

    /// 抽一帧做封面或参考帧。核心系列没有独立静态图 workflow 时，
    /// 角色卡与场景卡就是这样从开发片段里抽出来的。
    pub fn extract_frame(&self, video: &Path, at_seconds: f64, out: &Path) -> Result<()> {
        let ffmpeg = self.resolve("ffmpeg")?;
        if !video.is_file() {
            return Err(StudioError::ArtifactMissing {
                path: video.display().to_string(),
            });
        }
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-y", "-ss", &format!("{at_seconds}")])
            .arg("-i")
            .arg(video)
            .args(["-frames:v", "1", "-q:v", "2"])
            .arg(out);
        run(cmd, "ffmpeg 抽帧")
    }

    /// 截出一段。镜头之间接续用的就是它：把上一镜的尾段裁出来，
    /// 作为本镜第 0 帧的锚点喂回模型。
    ///
    /// **重编码，不 stream copy**：copy 只能从关键帧切，尾段的起点几乎不会
    /// 正好落在关键帧上，切出来会缺头几帧或多出一截——而锚点的帧数是要
    /// 卡在 `17k+5` 网格上的，差一帧就不是声明的那个东西了。
    pub fn cut(&self, video: &Path, start_seconds: f64, seconds: f64, out: &Path) -> Result<()> {
        let ffmpeg = self.resolve("ffmpeg")?;
        if !video.is_file() {
            return Err(StudioError::ArtifactMissing {
                path: video.display().to_string(),
            });
        }
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-y"])
            .arg("-i")
            .arg(video)
            .args([
                "-ss",
                &format!("{:.6}", start_seconds.max(0.0)),
                "-t",
                &format!("{seconds:.6}"),
                "-an",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(out);
        run(cmd, "ffmpeg 截取片段")
    }

    /// 把字幕烧进画面或作为软字幕封装。
    pub fn mux_subtitles(&self, video: &Path, srt: &Path, out: &Path) -> Result<()> {
        let ffmpeg = self.resolve("ffmpeg")?;
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-y"])
            .arg("-i")
            .arg(video)
            .arg("-i")
            .arg(srt)
            .args(["-c", "copy", "-c:s", "mov_text"])
            .arg(out);
        run(cmd, "ffmpeg 封装字幕")
    }
}

fn run(mut cmd: Command, what: &str) -> Result<()> {
    let out = cmd
        .output()
        .map_err(|e| StudioError::internal(format!("执行 {what} 失败：{e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(StudioError::internal(format!(
            "{what} 退出码 {:?}：{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
                .trim()
        )))
    }
}

/// 从 ffprobe 读到的实测元数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub has_audio: bool,
}

impl MediaInfo {
    pub fn from_ffprobe(v: &serde_json::Value) -> MediaInfo {
        let mut info = MediaInfo {
            duration_seconds: v["format"]["duration"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            ..Default::default()
        };
        if let Some(streams) = v["streams"].as_array() {
            for s in streams {
                match s["codec_type"].as_str() {
                    Some("video") => {
                        info.width = s["width"].as_u64().unwrap_or(0) as u32;
                        info.height = s["height"].as_u64().unwrap_or(0) as u32;
                        info.video_codec = s["codec_name"].as_str().map(String::from);
                        info.fps = parse_rational(s["r_frame_rate"].as_str().unwrap_or("0/1"));
                    }
                    Some("audio") => {
                        info.has_audio = true;
                        info.audio_codec = s["codec_name"].as_str().map(String::from);
                    }
                    _ => {}
                }
            }
        }
        info
    }

    /// 化简后的画幅，例如 `9:16`。
    pub fn aspect_ratio(&self) -> String {
        if self.width == 0 || self.height == 0 {
            return "unknown".into();
        }
        let g = gcd(self.width, self.height);
        format!("{}:{}", self.width / g, self.height / g)
    }
}

/// 两段媒体的流参数是否一致到可以直接拼接。时长不参与——它本来就不同。
fn same_stream(a: &MediaInfo, b: &MediaInfo) -> bool {
    a.video_codec == b.video_codec
        && a.audio_codec == b.audio_codec
        && a.has_audio == b.has_audio
        && a.width == b.width
        && a.height == b.height
        && (a.fps - b.fps).abs() < 0.01
}

fn parse_rational(s: &str) -> f64 {
    match s.split_once('/') {
        Some((n, d)) => {
            let n: f64 = n.parse().unwrap_or(0.0);
            let d: f64 = d.parse().unwrap_or(1.0);
            if d == 0.0 {
                0.0
            } else {
                n / d
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings_without_tools() -> Settings {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        Settings::load(None, Some(dir.path()))
    }

    #[test]
    fn missing_tool_says_where_it_looked() {
        let s = settings_without_tools();
        let m = Media::new(&s);
        let st = m.probe_tool("definitely-not-a-real-tool");
        assert!(!st.found);
        assert!(
            st.looked_in.iter().any(|p| p.contains(".env")),
            "应当报告找过 .env：{:?}",
            st.looked_in
        );
        assert!(st.looked_in.contains(&"PATH".to_string()));
    }

    #[test]
    fn probing_a_missing_file_is_an_artifact_error_not_a_crash() {
        let s = settings_without_tools();
        let m = Media::new(&s);
        let e = m.probe(Path::new("/nope/missing.mp4")).unwrap_err();
        // 没装 ffprobe 时先报工具缺失，装了则报产物缺失——两者都必须带 remedy。
        assert!(matches!(e.code(), "tool_unavailable" | "artifact_missing"));
        assert!(!e.remedy().is_empty());
        if e.code() == "tool_unavailable" {
            assert!(e.remedy().contains("FFPROBE_PATH"));
            // 配置是人的动作，重试才是 Agent 的动作——remedy 给的是后者，
            // 而不是一个二进制的命令行。见 docs/decisions/ADR-0002。
            assert!(e.remedy().contains("studio.retry_stage"), "{}", e.remedy());
            assert!(!e.remedy().contains("studiod"), "{}", e.remedy());
        }
    }

    #[test]
    fn ffprobe_output_is_parsed_into_measured_facts() {
        let v = json!({
            "format": { "duration": "10.000000" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "width": 1080, "height": 1920, "r_frame_rate": "30/1" },
                { "codec_type": "audio", "codec_name": "aac" }
            ]
        });
        let info = MediaInfo::from_ffprobe(&v);
        assert_eq!(info.duration_seconds, 10.0);
        assert_eq!(info.width, 1080);
        assert_eq!(info.height, 1920);
        assert_eq!(info.fps, 30.0);
        assert_eq!(info.aspect_ratio(), "9:16");
        assert!(info.has_audio);
        assert_eq!(info.video_codec.as_deref(), Some("h264"));
    }

    /// 判断能否直接拼接只用 ffprobe。没装 ffprobe 时报工具缺失而不是崩。
    #[test]
    fn stream_copy_decision_needs_only_ffprobe() {
        let s = settings_without_tools();
        let m = Media::new(&s);
        let e = m
            .can_stream_copy(&[PathBuf::from("/nope/a.mp4")])
            .unwrap_err();
        assert!(matches!(e.code(), "tool_unavailable" | "artifact_missing"));
    }

    #[test]
    fn identical_streams_can_be_copied() {
        let a = MediaInfo {
            duration_seconds: 1.4,
            width: 1080,
            height: 1920,
            fps: 30.0,
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
            has_audio: true,
        };
        let mut b = a.clone();
        b.duration_seconds = 2.0; // 时长不同不影响能否 copy
        assert!(same_stream(&a, &b));

        let mut c = a.clone();
        c.width = 720;
        assert!(!same_stream(&a, &c), "分辨率不同必须重编码");

        let mut d = a.clone();
        d.fps = 24.0;
        assert!(!same_stream(&a, &d), "帧率不同必须重编码");

        let mut e = a.clone();
        e.has_audio = false;
        assert!(!same_stream(&a, &e), "有的有音轨有的没有，直接拼会出问题");
    }

    #[test]
    fn aspect_ratio_handles_landscape_and_unknown() {
        let mut i = MediaInfo {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        assert_eq!(i.aspect_ratio(), "16:9");
        i.width = 0;
        assert_eq!(i.aspect_ratio(), "unknown");
    }

    #[test]
    fn odd_frame_rates_do_not_divide_by_zero() {
        assert_eq!(parse_rational("30000/1001").round(), 30.0);
        assert_eq!(parse_rational("0/0"), 0.0);
        assert_eq!(parse_rational("25"), 25.0);
    }
}
