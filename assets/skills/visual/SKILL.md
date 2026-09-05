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
- **每张卡出多个视图**，不是一张大头照：角色走转身图五视图，场景走建立/主机位/反打/细节，道具走正/侧/使用/比例。必需视图缺一个提交就会被挡下，各类必需哪些见 consistency/character-sheet.md。
- 每张卡写一段 identity_prompt，**一次写定**，**这张卡的**所有视图逐字复用——每个视图的 prompt 都以它开头，一个字不改，后面再接该视图特有的机位／取景。**那半句要写成人话，不是把视图的枚举名抄上去**：`face_close` 写成「面部特写，肩部以上入画，中性表情」，`wardrobe_detail` 写成「服装材质与关键配饰的近景」。抄枚举名的结果是七个视图长得一模一样——真发生过，控制面会在提交时挡下（V14）。连带一条：**特写类视图不要写「不裁切」**，它们按定义就是裁切的。**每张卡的身份锁写的是这张卡自己那个东西**：角色卡写那个人，场景卡写那个空间，道具卡写那个道具。把角色的身份锁抄进场景卡或道具卡，出来的三张卡是同一个人——真发生过，控制面会在提交时就挡下（V13）。只有角色卡这一段要逐字包含分镜定下的身份锁。
- 标出主视图（每张卡有且仅有一个）并把它排在 views 第一位——**顺序就是生成顺序**。其余视图的 derived_from 是一个列表：第一项是主视图，后面补上这一张之前已经定稿的其余视图（累积锁定，最多 10 张）。并行出八张，出来的是八个长得像但不是同一个人的角色。
- 写明一致性锁定：角色外观、机位签名、环境、排版禁止项。
- 降级策略写死：核心系列不可用就结构化阻塞，不自动换系列。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/consistency/character-sheet.md`
- `.agents/doctrine/consistency/bible.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `asset_plan` 下。**提交前先调 `studio.schema("visual_assets")`** 取回完整契约，不要凭印象填字段。必填项是：

- `asset_plan.backend`
- `asset_plan.core_model_family`
- `asset_plan.consistency_lock`
- `asset_plan.assets`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `visual_assets.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] consistency_lock.character 是从分镜 character_lock.identity_lock 复制来的，逐字相同
- [ ] 每个跨镜头复用的角色、场景、道具都有卡
- [ ] 每张卡的必需视图齐全，一个不少
- [ ] 每张卡只有一个主视图且排在第一位，其余视图的 derived_from 累积挂上前面已定稿的
- [ ] 同一张卡的所有视图画幅一致
- [ ] 每个视图的提示词都以**本卡自己的** identity_prompt 逐字开头（不是别的卡的）
- [ ] 一致性锁定写明了外观、机位签名、环境与排版禁止项

## 注意

- 这是 hybrid 阶段：你定资产计划，**提交通过之后控制面立刻开始生成**，生成完才上确认门——门上要人看的是卡片本身，不是一份没见过的 JSON。所以提交完 waiting_on 会变成 system：那时候等着，隔一会儿再 studio.status，别在那里收尾。
- status 一律写 planned，path 与 provenance 留空——那两个字段由控制面回填，不是你写的。
- 卡片是**测量用的参考素材**，不是好看的剧照。**角色卡与道具卡**走中性灰底、均匀柔光、无阴影投射——主体要能从背景里孤立出来才好比对。**场景卡不套这条**：那个空间本身就是主体，孤立不了。它走自然光照与真实环境，同一空间同一陈设，变的只有机位与光线时段；建立镜头放在灰底上就没有全貌可看了。戏剧性打光留给成片。

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
- `studio.retry_stage`
- `studio.self_review`
