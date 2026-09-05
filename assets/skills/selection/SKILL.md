---
name: selection
description: 从可行性、受众匹配和发布风险筛选 brief，给出推荐方案与取舍。
---

<!-- 本文件由代码生成，请勿手改。 -->

# selection Skill

触发：brief 已通过，需要决定往哪个方向做。

不触发：创意本身还没成形；那是 idea 的事。

## 职责

- **逐个**评估上一阶段给出的每个方案，不要只写推荐的那个——用户要看的是比较，不是结论。
- 每个方案都评三样：可行性（模型可控性、制作成本）、受众匹配（钩子强度、观看收益、留存）、风险。
- 把发布风险分成可规避、不可接受、需用户决定三类。
- 给出一个明确推荐（`recommendation.concept_id` 指向候选之一），并说清楚推荐它牺牲了什么。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/story/concepts.md`
- `.agents/doctrine/story/hook.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `selection` 下。**提交前先调 `studio.schema("selection")`** 取回完整契约，不要凭印象填字段。必填项是：

- `selection.candidates`
- `selection.recommendation`
- `selection.tradeoffs`
- `selection.acceptance_metrics`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段有确认门 `selection.approval`。提交时必须同时给出 `confirmation`：一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。

用户选了 revise 类选项，控制面会自动把阶段打回草稿；用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] 每个方案都单独评过，没有只写推荐那个
- [ ] recommendation 指向的 concept_id 确实在候选里
- [ ] 推荐说清了牺牲什么，不是只讲优点
- [ ] 风险分成可规避 / 需用户决定 / 不可接受三类

## 注意

- 确认门在这里把你的候选列表原样摆给用户选。用户可能不选你推荐的那个——这是设计如此，不是异常。
- 被选中的方案会记进产物的 `_gate_choice`，后面的阶段照它写，不要回头改主意。
- 确认门问的是方向，不是细节。细节留到剧本阶段再改。

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
