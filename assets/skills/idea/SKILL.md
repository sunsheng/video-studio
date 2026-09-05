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
- 给出 **2–3 个互斥方案**：平台受众时长这些由需求定死的共用，各方案不同的是切入角度、前三秒钩子和节拍走向。互斥的判据见 concepts.md。
- 每个方案写清选它要牺牲什么，且各方案牺牲的不是同一件事。
- 对模糊输入做出判断并**写进 assumptions**，不要私下假设也不要反复追问。
- 识别发布风险并分级：可规避 / 需用户决定 / 不可接受。
- 定义可验收的成功标准——后面的 review 阶段会照着它逐条核对。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/story/concepts.md`
- `.agents/doctrine/story/hook.md`
- `.agents/doctrine/story/structure.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `brief` 下。**提交前先调 `studio.schema("idea")`** 取回完整契约，不要凭印象填字段。必填项是：

- `brief.title`
- `brief.logline`
- `brief.platform`
- `brief.audience`
- `brief.duration_seconds`
- `brief.shot_count`
- `brief.aspect_ratio`
- `brief.concepts`
- `brief.success_metrics`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] 至少两个方案，且互斥：选了一个，另一个独有的东西就拍不进去了
- [ ] 各方案的 angle、hook_0_3s、story_beats 都不同，不是换了说法的同一拍法
- [ ] 各方案的 tradeoff 各不相同，没有三条都写成需求本身的约束
- [ ] 钩子在前 3 秒内成立，且说得出具体是什么画面
- [ ] 对模糊输入的判断写进了 assumptions，没有私下假设也没有反复追问
- [ ] success_metrics 每一条都能被验收，不是「效果好」这类说法

## 注意

- 这一阶段没有确认门，提交即通过。真正的第一道门在选题阶段，用户在那里从你给的方案里挑一个——只给一个方案，那道门就退化成「同意 / 重来」。
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
- `studio.self_review`
