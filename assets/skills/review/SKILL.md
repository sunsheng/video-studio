---
name: review
description: 检查成片的媒体完整性、时长、字幕、编码与发布风险。
---

<!-- 本文件由代码生成，请勿手改。 -->

# review Skill

触发：后期完成，需要验收。

不触发：片子还没拼完。

## 职责

- 技术验收由控制面做：每一条检查都基于 ffprobe 的**实测**元数据，不靠推断。
- 技术验收出来之后，**内容验收是你的**：调 studio.self_review，按 rubric 的五个维度各给一条结论。
- 每条结论都要带一个可指认的时间点，和在那一刻看见/听见了什么。写「还不错」会被退回。
- 逐条核对 idea 阶段定下的 success_metrics——技术型的在 checks 里，内容型的在 brief_metrics 那一条里。
- 任一必需项缺失就判不通过，不要为了让流程走完而放水。

## 方法

职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。

- `.agents/doctrine/quality/rubric.md`
- `.agents/doctrine/quality/checklist.md`

这些文件就在这部作品的目录里，用你的文件读取工具直接读——路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）

## 输入输出

本阶段的产物放在 `outputs` 的顶层键 `review` 下。**提交前先调 `studio.schema("review")`** 取回完整契约，不要凭印象填字段。必填项是：

- `review.passed`
- `review.checks`

上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。

## 确认点

本阶段没有确认门，提交即通过。

## 失败与恢复

任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。schema 不合规时 `message` 会精确指到出错的字段路径，例如 `script.story_arc[1].duration_seconds`。

## 提交前自检

逐条过。过不了就别提交——退回来重做比往下走便宜得多。

- [ ] 五个维度各一条，一条不少也不重复
- [ ] 每条都带了落在成片时长之内的时间点
- [ ] 证据写的是那一刻看见/听见了什么，不是「还不错」
- [ ] summary 说清了最强的一点和最弱的一点

## 注意

- 技术验收全过只说明片子是完整的，不说明它好看。这两半判的是两件事，见 quality/rubric.md。
- 内容自评不改变 passed，但不交这一份作品就不算收尾——status 会一直停在「等你交自评」。
- 说实话比说好话有用：全写 met 的自评既骗不到人，也帮不到下一次。
- 验收通过不等于可以发布。对外发布需要另行获得授权。

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
