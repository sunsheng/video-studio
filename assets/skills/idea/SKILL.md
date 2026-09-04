---
name: idea
description: 把用户创意整理成可执行 brief，明确受众、平台、时长与发布风险。
---

<!-- 本文件由代码生成，请勿手改。 -->

# idea Skill

触发：用户描述了一个想拍的东西，或者说「从头开始」「做一个新的」。

不触发：已经有 brief 之后的任何阶段。

## 职责

- 把口语化的创意转成结构化 brief：标题、logline、平台、受众、时长、镜头数、画幅。
- 对模糊输入做出判断并**写进 assumptions**，不要私下假设也不要反复追问。
- 识别发布风险并分级：可规避 / 需用户决定 / 不可接受。
- 定义可验收的成功标准——后面的 review 阶段会照着它逐条核对。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `brief` 下。**提交前先调 `studio.schema("idea")`** 取回完整契约，不要凭印象填字段。必填项是：

- `brief.title`
- `brief.logline`
- `brief.platform`
- `brief.audience`
- `brief.duration_seconds`
- `brief.shot_count`
- `brief.aspect_ratio`
- `brief.story_beats`
- `brief.success_metrics`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 这一阶段没有确认门，提交即通过。真正的第一道门在选题阶段。
- 用户说「20色女性」这类明显笔误，按最合理的理解处理并在 assumptions 里写明，不要卡住不动。

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
