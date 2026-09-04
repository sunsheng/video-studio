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

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/models/minimax_h3.md`
- `.agents/models/wan2_2.md`
- `.agents/models/ltx2_5.md`
- `.agents/doctrine/exemplars/prompt_pack.md`
- `.agents/doctrine/consistency/bible.md`
- `.agents/doctrine/quality/banned.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

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

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] 逐项对照能力卡：写的每个参数这条基线都吃
- [ ] 不支持负向提示词的系列，约束改写成了正向的完整句子
- [ ] 身份锁在每一镜里逐字出现，没有写成「同一位…」
- [ ] 没有禁用词（cinematic / 电影感 / 唯美这类）
- [ ] 种子固定并记录，尺寸与帧数按各镜时长算准
- [ ] audio 写了三层，没有放弃原生音频

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
- `studio.comfy.exclude_node`
- `studio.retry_stage`
