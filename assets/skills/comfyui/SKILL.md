---
name: comfyui
description: 提交已确认且通过校验的 workflow，选择健康节点、跟踪执行并登记输出。
---

<!-- 本文件由 `studiod emit-assets` 生成，请勿手改。 -->

# comfyui Skill

触发：提示词包已确认，控制面开始渲染。

不触发：任何创作判断。

## 职责

- 这是确定性阶段，由控制面执行，你只需要用 studio.status 观察。
- 失败时读 studio.timeline 看清是哪一镜、哪个节点、什么原因。
- 节点不可用或模型契约不满足时会结构化阻塞——不要建议换模型来绕过。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `render` 下。**提交前先调 `studio.schema("render")`** 取回完整契约，不要凭印象填字段。必填项是：

- `render.shots`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 运行控制面的机器不需要 GPU，一切经 ComfyUI 的 HTTP API 完成。

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
