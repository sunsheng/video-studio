---
name: director
description: 把已确认剧本转成逐镜头分镜，定义景别、构图、机位、灯光与时长。
---

<!-- 本文件由代码生成，请勿手改。 -->

# director Skill

触发：剧本已确认，需要把每一拍变成可拍的镜头。

不触发：还在讨论故事讲什么；那是 script 的事。

## 职责

- 每个镜头一个主动作、一个主镜头运动。两个以上的运动会让生成结果失控。
- 写清动作链（起 → 承 → 收）、首帧与尾帧，转场要能被审计。
- 锁定角色连续性：外观、服装、机位签名，逐镜保持一致。
- 安全约束写进分镜本身，而不是留给后面的阶段补救。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/camera/grammar.md`
- `.agents/doctrine/camera/blocking.md`
- `.agents/doctrine/camera/lighting.md`
- `.agents/doctrine/consistency/bible.md`
- `.agents/doctrine/audio/design.md`
- `.agents/doctrine/exemplars/storyboard.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `storyboard` 下。**提交前先调 `studio.schema("storyboard")`** 取回完整契约，不要凭印象填字段。必填项是：

- `storyboard.title`
- `storyboard.aspect_ratio`
- `storyboard.total_duration_seconds`
- `storyboard.shot_count`
- `storyboard.shots`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `storyboard.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] 每镜说得出 shot_function，说不出就删掉这一镜
- [ ] 每镜三条物理事实齐全，且都是拍得出来的
- [ ] 每镜只有一个主运镜，且落在受控词表里
- [ ] 镜头时长与剧本各拍对齐，总和一致
- [ ] character_lock.identity_lock 一次写定：年龄、发型、上衣、下装、鞋、随身物都在里面
- [ ] 角色外观串逐镜逐字相同（复制粘贴，不要复述）
- [ ] 每镜的 audio 都写了，没有留空

## 注意

- 镜头时长必须与剧本各拍对齐；改了时长就要回到剧本阶段改。

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
