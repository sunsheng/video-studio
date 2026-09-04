---
name: review
description: 检查成片的媒体完整性、时长、字幕、编码与发布风险。
---

<!-- 本文件由代码生成，请勿手改。 -->

# review Skill

触发：后期完成，需要验收。

不触发：创作质量的主观评价。

## 职责

- 每一条检查都必须基于 ffprobe 的**实测**元数据，不能靠推断。
- 逐条核对 idea 阶段定下的 success_metrics。
- 任一必需项缺失就判不通过，不要为了让流程走完而放水。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `review` 下。**提交前先调 `studio.schema("review")`** 取回完整契约，不要凭印象填字段。必填项是：

- `review.passed`
- `review.checks`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 验收通过不等于可以发布。对外发布需要另行获得授权。

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
