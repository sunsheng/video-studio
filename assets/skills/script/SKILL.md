---
name: script
description: 创建短视频的故事结构、节奏与声音时间线。
---

<!-- 本文件由 `studiod emit-assets` 生成，请勿手改。 -->

# script Skill

触发：方向已确认，需要把它变成逐拍的内容。

不触发：镜头语言、景别、机位——那是 director 的事。

## 职责

- 按**内容**分配时长，不要平均切分。动作复杂、信息量大的拍给更多时间。
- 各拍时长必须精确合计到 brief 规定的总时长。
- 同时给出声音时间线：有口播就写台词，没有就写环境声与拟音的来源。
- 字幕策略要明确。没有字幕也要写清楚是「本版无字幕」。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `script` 下。**提交前先调 `studio.schema("script")`** 取回完整契约，不要凭印象填字段。必填项是：

- `script.title`
- `script.total_duration_seconds`
- `script.shot_count`
- `script.timing_rule`
- `script.story_arc`
- `script.segments`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `script.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 「不要固定 2 秒」这类反馈直接调 studio.revise，然后重新提交。不需要先解除任何占用。
- 总时长对不上是最常见的退回原因，提交前自己加一遍。

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
