---
name: visual
description: 规划并生成一致的角色卡、场景卡与参考资产。
---

<!-- 本文件由代码生成，请勿手改。 -->

# visual Skill

触发：分镜已确认，需要先把跨镜头复用的视觉资产定下来。

不触发：逐镜头的提示词；那是 prompt 的事。

## 职责

- 为跨镜头复用的角色、场景、道具各建一张卡，给稳定的 asset_id。
- 写明一致性锁定：角色外观、机位签名、环境、排版禁止项。
- 核心系列没有独立静态图 workflow 时，先生成开发片段再抽帧，并保留抽帧参数。
- 降级策略写死：核心系列不可用就结构化阻塞，不自动换系列。

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `asset_plan` 下。**提交前先调 `studio.schema("visual_assets")`** 取回完整契约，不要凭印象填字段。必填项是：

- `asset_plan.backend`
- `asset_plan.core_model_family`
- `asset_plan.consistency_lock`
- `asset_plan.requests`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `visual_assets.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- 这是 hybrid 阶段：你定资产计划，确认之后由控制面执行生成。

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
