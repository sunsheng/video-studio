//! 把声明里的 `asset_id` 变成 ComfyUI 那侧的文件名。
//!
//! 组装器只认「素材叫什么名字」，不关心它从哪来——这一层负责补上中间那段：
//! 找到 bundle 里的实际文件、传到 ComfyUI、把返回的文件名交回去。
//!
//! 引用有两个来源，跟 [`studio_core::assembly`] 的 V7 / V9 一一对应：
//!
//! - **登记过的资产**（`C01` / `C01.front`）：来自 `visual_assets` 的视图，
//!   控制面生成完会回填 `path` 与 `status: ready`。
//! - **镜间片段**（`sh01.tail` / `sh01.tail22`）：来自本次已经渲完的上一镜，
//!   用 ffmpeg 现裁。不带帧数的取一帧静图，带帧数的裁成那么长的一段。
//!
//! 同一份素材会被多镜引用（角色卡尤其如此），所以上传结果按 `asset_id`
//! 缓存——传一次就够，重复传只是浪费带宽和 ComfyUI 的磁盘。

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use studio_comfy::Comfy;
use studio_core::assembly::{parse_shot_segment, ShotSegment};
use studio_core::{Result, StudioError};
use studio_engine::bundle::Bundle;
use studio_media::Media;

/// 一次渲染里所有素材的解析与上传。
pub struct AssetResolver<'a> {
    bundle: &'a Bundle,
    media: Media<'a>,
    /// `visual_assets` 的产物，用来查登记过的资产。
    plan: Value,
    /// 已经渲完的镜头：`shot_id` → bundle 内相对路径。镜间引用查这里。
    rendered: BTreeMap<String, RenderedShot>,
    /// `asset_id` → ComfyUI 那侧的文件名。
    uploaded: Mutex<BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct RenderedShot {
    pub path: String,
    pub duration_seconds: f64,
    pub fps: f64,
}

impl<'a> AssetResolver<'a> {
    pub fn new(
        bundle: &'a Bundle,
        settings: &'a studio_engine::Settings,
        plan: Value,
        rendered: BTreeMap<String, RenderedShot>,
    ) -> AssetResolver<'a> {
        AssetResolver {
            bundle,
            media: Media::new(settings),
            plan,
            rendered,
            uploaded: Mutex::new(BTreeMap::new()),
        }
    }

    /// `asset_id` → ComfyUI 那侧的文件名。同一个 id 只会真的传一次。
    pub fn upload(&self, comfy: &Comfy, asset_id: &str) -> Result<String> {
        if let Some(name) = self.uploaded.lock().unwrap().get(asset_id) {
            return Ok(name.clone());
        }
        let local = self.resolve(asset_id)?;
        let bytes = std::fs::read(&local).map_err(|e| StudioError::ArtifactMissing {
            path: format!("{}（{e}）", local.display()),
        })?;
        // 上传名带上 asset_id，出问题时在 ComfyUI 的 input 目录里认得出来。
        let ext = local
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "png".into());
        let safe: String = asset_id
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let remote = comfy.upload_image(&format!("{safe}.{ext}"), &bytes)?;
        self.uploaded
            .lock()
            .unwrap()
            .insert(asset_id.to_string(), remote.clone());
        Ok(remote)
    }

    /// `asset_id` → bundle 里的绝对路径。需要现裁的会先裁出来。
    fn resolve(&self, asset_id: &str) -> Result<std::path::PathBuf> {
        match parse_shot_segment(asset_id) {
            Some(seg) => self.cut_from_rendered(asset_id, seg),
            None => self.registered_asset(asset_id),
        }
    }

    /// 登记过的资产：`C01` 取主视图，`C01.front` 取指定视图。
    fn registered_asset(&self, asset_id: &str) -> Result<std::path::PathBuf> {
        let (card_id, view_name) = match asset_id.split_once('.') {
            Some((c, v)) => (c, Some(v)),
            None => (asset_id, None),
        };
        let cards = self.plan["assets"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        let card = cards
            .iter()
            .find(|c| c["asset_id"].as_str() == Some(card_id))
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: format!(
                    "「{asset_id}」在 visual_assets 里找不到对应的卡（{card_id}）。\
                     可用的：{}",
                    list_ids(cards)
                ),
            })?;
        let views = card["views"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        // 不指定视图就用主视图——它是这张卡的基准，其余视图都以它为参考图生成。
        let view = match view_name {
            Some(name) => views.iter().find(|v| v["view"].as_str() == Some(name)),
            None => views.iter().find(|v| v["is_anchor"] == true),
        }
        .ok_or_else(|| StudioError::ModelContractViolation {
            detail: match view_name {
                Some(name) => format!("卡 {card_id} 没有名为 {name} 的视图"),
                None => format!("卡 {card_id} 没有标记主视图（is_anchor），不知道该用哪一张"),
            },
        })?;

        let status = view["status"].as_str().unwrap_or("planned");
        let path = view["path"].as_str().unwrap_or_default();
        if status != "ready" || path.is_empty() {
            return Err(StudioError::ArtifactMissing {
                path: format!(
                    "{asset_id}（视图状态 {status}）——这张卡还没生成出来。\
                     视觉资产的生成还没接上控制面，参考图这条路要等它落地"
                ),
            });
        }
        self.bundle.resolve(path)
    }

    /// 镜间片段：从已经渲完的那一镜裁出来。
    fn cut_from_rendered(&self, asset_id: &str, seg: ShotSegment) -> Result<std::path::PathBuf> {
        let src =
            self.rendered
                .get(seg.shot_id)
                .ok_or_else(|| StudioError::ModelContractViolation {
                    detail: format!(
                        "「{asset_id}」要的是 {} 的片段，但这一轮还没有它的产物。\
                         镜间引用只能指向更靠前的镜头，调度会先跑完它——\
                         出现这条说明依赖顺序算错了",
                        seg.shot_id
                    ),
                })?;
        let video = self.bundle.resolve(&src.path)?;
        let safe: String = asset_id
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();

        match seg.frames {
            // 不带帧数：取一帧静图。尾帧往回退半帧，落在最后一帧上而不是越界。
            None => {
                let rel = format!("media/segments/{safe}.png");
                let out = self.bundle.resolve(&rel)?;
                let at = if seg.from_tail {
                    (src.duration_seconds - 0.5 / src.fps.max(1.0)).max(0.0)
                } else {
                    0.0
                };
                self.media.extract_frame(&video, at, &out)?;
                Ok(out)
            }
            // 带帧数：裁成那么长的一段。
            Some(n) => {
                let rel = format!("media/segments/{safe}.mp4");
                let out = self.bundle.resolve(&rel)?;
                let seconds = n as f64 / src.fps.max(1.0);
                let start = if seg.from_tail {
                    (src.duration_seconds - seconds).max(0.0)
                } else {
                    0.0
                };
                self.media.cut(&video, start, seconds, &out)?;
                Ok(out)
            }
        }
    }
}

fn list_ids(cards: &[Value]) -> String {
    let ids: Vec<&str> = cards
        .iter()
        .filter_map(|c| c["asset_id"].as_str())
        .collect();
    if ids.is_empty() {
        "（一张都没有）".to_string()
    } else {
        ids.join("、")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_tail_means_a_single_frame() {
        let s = parse_shot_segment("sh01.tail").unwrap();
        assert_eq!(s.shot_id, "sh01");
        assert!(s.from_tail);
        assert_eq!(s.frames, None);
    }

    #[test]
    fn a_tail_with_a_count_means_a_clip() {
        let s = parse_shot_segment("S02.tail22").unwrap();
        assert_eq!(s.shot_id, "S02");
        assert!(s.from_tail);
        assert_eq!(s.frames, Some(22));
    }

    #[test]
    fn head_reads_from_the_front() {
        let s = parse_shot_segment("S02.head5").unwrap();
        assert!(!s.from_tail);
        assert_eq!(s.frames, Some(5));
    }

    /// 视角 id 里也有点，但后缀不是 tail/head，不能被当成镜间引用——
    /// 认错了就会去找一个不存在的镜头，而不是那张卡。
    #[test]
    fn a_view_id_is_not_a_shot_segment() {
        assert!(parse_shot_segment("C01.front").is_none());
        assert!(parse_shot_segment("C01").is_none());
        assert!(parse_shot_segment("SC02.key_angle").is_none());
    }
}
