---
name: post
description: 把生成片段拼接为交付视频，处理字幕、音频、封面。
---

<!-- 本文件由 `studiod emit-assets` 生成，请勿手改。 -->

# post Skill

触发：渲染完成，需要出成片。

不触发：镜头本身要重做；那要回到更早的阶段。

## 职责

- 按分镜顺序拼接，转场必须与分镜里写的一致。
- 字幕只能来自已确认的剧本文本，不要在这一步新编。
- 封面从成片里抽帧，不要另外生成一张对不上的图。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `post` 下。**提交前先调 `studio.schema("post")`** 取回完整契约，不要凭印象填字段。必填项是：

- `post.video`
- `post.duration_seconds`
- `post.aspect_ratio`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 这是确定性阶段，由控制面执行。ffmpeg 不要求在 PATH 中，配置见 .env。

## Studio MCP

可用工具（全部不带 run_id，当前目录就是当前作品）：

- `studio.status`
- `studio.schema`
- `studio.submit_stage`
- `studio.answer`
- `studio.revise`
- `studio.undo`
- `studio.stage_output`
- `studio.timeline`
- `studio.export`
- `studio.comfy.exclude_node`
- `studio.retry_stage`
