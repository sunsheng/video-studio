---
name: prompt
description: 把已确认分镜与视觉资产编译成逐镜头 prompt 和 workflow 参数。
---

<!-- 本文件由代码生成，请勿手改。 -->

# prompt Skill

触发：视觉资产已确认，准备进入渲染。

不触发：画面内容本身还在改；那是 director 的事。

## 职责

- 逐镜头给出正向、负向提示词，以及尺寸、帧数、帧率、种子。
- 种子必须固定并记录，否则结果不可复现。
- workflow 名必须是已验证基线里的，不要临时编一个。
- 引用视觉资产用 asset_id，不要重复描述角色外观。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `prompt_pack` 下。**提交前先调 `studio.schema("prompt_pack")`** 取回完整契约，不要凭印象填字段。必填项是：

- `prompt_pack.core_model_family`
- `prompt_pack.shots`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `prompt_pack.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 这道门是花 GPU 时间之前的最后一关。确认之后就开始烧显卡了，提交前自己再读一遍。

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
