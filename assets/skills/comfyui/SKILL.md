---
name: comfyui
description: 提交已确认且通过校验的 workflow，选择健康节点、跟踪执行并登记输出。
---

<!-- 本文件由代码生成，请勿手改。 -->

# comfyui Skill

触发：提示词包已确认，控制面开始渲染。

不触发：任何创作判断。

## 职责

- 这是确定性阶段，由控制面执行，你只需要用 studio.status 观察。
- 失败时读 studio.timeline 看清是哪一镜、哪个节点、什么原因。
- 节点不可用或模型契约不满足时会结构化阻塞——不要建议换模型来绕过。
- 怀疑是某个节点本身有问题（反复失败、迟迟连不上），先调 studio.comfy.exclude_node 把它排除，再重试。
- 执行失败但内容没问题（节点抖动、连接超时）时调 studio.retry_stage，不要用 studio.revise——那是给内容要改的场景用的。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/failure/modes.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

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
- 孤立的一次轮询连接超时不代表渲染失败：控制面会自动容错重试，只有连续失败或总耗时超过 timeout 才会真正报错。

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
- `studio.self_review`
