---
name: run-management
description: 解释当前作品的状态，走修订与恢复路径。
---

<!-- 本文件由代码生成，请勿手改。 -->

# run-management Skill

触发：用户问「现在到哪了」「怎么改」「重做某一步」，或者遇到阻塞需要判断下一步。

不触发：各阶段的创作执行本身。

## 职责

- 先调 studio.status。信封里的 next_action 和 blocked_by 已经说清了该做什么。
- 阻塞时照 blocked_by.remedy 做。它一定会指向一个能调的工具。
- 用户提出修改意见就调 studio.revise——它不会失败，也不需要先解除什么占用。
- 作品的历史看 studio.timeline，某一步的产物看 studio.stage_output。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/failure/modes.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读。（`.studio/` 是控制面私有的，那个不要碰。）

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 注意

- **没有 run_id**。你打开的这个目录就是当前作品，工具也都不收项目参数。
- 新建、继续、修订之外没有别的动作。列表用 `ls`，另存为用 `cp -r`，删除用 `rm -rf`。

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
