<!-- 本文件由代码生成，请勿手改。 -->

# 这部作品

你现在打开的这个文件夹**就是一部作品**。像一份 .docx：新建、继续、修订，
只有这三个动作。没有项目列表，没有 run id，工具也都不收项目参数。

- 想看另一部作品：退出，`cd` 到那个文件夹再打开。
- 想另存一版：`cp -r 这个目录 另一个名字.studio`。
- 想归档、打包或发给别人：这些超出你的能力范围，提醒用户自己在终端处理，
  不要代劳。

## 你只能通过 Studio MCP 改变状态

创作判断由你来做，状态由控制面持有。**不要**用 shell 去读写这个目录里的
状态。你能看到的只有这份 MCP 工具面——没有能推进阶段的命令行，因为
状态变更只有 MCP 一个入口。

`.studio/` 是控制面私有的，里面是状态库、日志和锁。不要读，不要改。
它有完整性校验，外部改动会在下一次调用时以 `state_drift` 暴露出来。

## 四条工作习惯

1. **不确定就先调 `studio.status`。** 信封里的 `next_action` 说了下一步交什么，
   `pending_question` 说了在等用户答什么，`blocked_by` 说了被什么挡住。
2. **提交前先调 `studio.schema`，不要猜字段。** 也不要参考别处的产物——
   这个目录里没有别的作品，schema 才是唯一事实源。
3. **被挡住时照 `blocked_by.remedy` 做。** 每一条阻塞都带着可执行的下一步。
   如果 remedy 说不通，那是控制面的缺陷，报告出来，不要绕过去。
4. **写之前先看 `next_action.decisions`。** 那是用户此前**否决过什么**
   （`rejected`，他的原话逐字）和**在门上选过什么**（`chose`）。
   他在剧本阶段说过的话，到分镜阶段依然算数——不要让他再说第二遍。

### 怎么读 decisions

按时间**倒序**给出，最近的在前。同一件事上，后面的条目压过前面的：
用户先说「要快节奏」、后说「第三镜可以慢一点」，两条都在，
以后一条为准。

它是**历史**，不是待办：记下就不改、不删，`studio.undo` 也不回退它。
撤销一次修订不等于用户改了口味——真改了他会再说一句新的。

## 方法手册

上面三条讲的是**怎么交**。**怎么写得好**是另一回事，写在
`.agents/doctrine/` 里：镜头语法、光与色、调度、结构与钩子、声音设计、
一致性、失败模式、禁用词，还有一部完整作品的黄金样例。
每个 Skill 会指出自己该读哪几份，索引在 `.agents/doctrine/README.md`。

各个模型系列吃的参数**不一样**——写了某条系列没有绑定的参数会被静默丢弃，
不报错也不生效。写提示词之前先看 `.agents/models/` 里对应系列那一份。

这些文件用你的文件读取工具直接读，**按需读，不要一次全读**。
唯一的禁区还是 `.studio/`。

## 阶段与确认门

| # | 阶段 | 能力 | 类型 | 确认门 |
|---|---|---|---|---|
| 1 | `idea` | `idea` | creative（你产出全部内容） | — |
| 2 | `selection` | `selection` | creative（你产出全部内容） | `selection.approval` |
| 3 | `script` | `script` | creative（你产出全部内容） | `script.approval` |
| 4 | `storyboard` | `director` | creative（你产出全部内容） | `storyboard.approval` |
| 5 | `visual_assets` | `visual` | hybrid（你定内容，控制面执行） | `visual_assets.approval` |
| 6 | `prompt_pack` | `prompt` | creative（你产出全部内容） | `prompt_pack.approval` |
| 7 | `preview` | `comfyui` | deterministic（控制面执行，你只观察） | `preview.approval` |
| 8 | `render` | `comfyui` | deterministic（控制面执行，你只观察） | — |
| 9 | `post` | `post` | deterministic（控制面执行，你只观察） | — |
| 10 | `review` | `review` | deterministic（控制面执行，你只观察） | — |


门在阶段**产出之后**暂停。`prompt_pack` 那道门是花 GPU 时间之前的最后一关。

确认门的选项要自己声明 `outcome`：`approve` 通过并进入下一阶段，
`revise` 把本阶段打回草稿。不要靠选项 id 的字面意思去暗示，控制面只认 `outcome`。

## 修订

用户提出修改意见时调 `studio.revise(stage, message)`。它**不会失败**，
也不需要先解除任何占用——提交、修订、再提交是一条顺畅的路径。

修订会让作品的进度整体退回到那个阶段：**它之后的阶段一律变回未执行**。
分镜是照旧剧本做的，剧本一改它就不再成立。旧产物文件留着，你可以用
`studio.stage_output` 读出来参考，重新提交时直接覆盖。

程序不做版本管理。要留版本请让用户 `cp -r`，或提醒他们自己在终端打包。

## 工具

| 工具 | 作用 |
|---|---|
| `studio.status` | 读取决策信封：现在在哪个阶段、该谁行动、下一步要交什么。任何时候不确定就先调它。 |
| `studio.schema` | 取回某个阶段产物的 JSON Schema。提交前先看它，不要去猜字段，也不要参考别处的产物。 |
| `studio.submit_stage` | 提交当前阶段的产物。有确认门的阶段必须同时给出 confirmation；选项要用 outcome 声明是通过还是打回，不要靠 id 的字面意思暗示。 |
| `studio.answer` | 把用户对确认门的选择交回来。选中 outcome=revise 的选项会自动把阶段打回草稿。 |
| `studio.revise` | 用户提出修改意见时调它。阶段回到草稿，可以立刻重新提交。它不会失败，也不需要先解除什么占用。作品的进度会整体退回到该阶段，它之后的阶段一律变回未执行——旧产物留着可以读出来参考。 |
| `studio.undo` | 撤销上一次修订，把作品整个恢复到那次 studio.revise 之前——旧产物回来，被退回的下游阶段也恢复已通过。只有一层，且恢复后即失效。 |
| `studio.stage_output` | 读取某个阶段的完整产物。上游被改后，下游的旧产物仍可在这里读到，供参考着改。 |
| `studio.timeline` | 读取用户可见的操作历史：每个阶段何时提交、何时挂门、何时被修订。 |
| `studio.export` | 把交付物投递到作品的 output/ 目录。后期阶段通过之后才可用。 |
| `studio.comfy.exclude_node` | 把一个 ComfyUI 节点加入本次会话的临时排除名单，选节点时会跳过它。怀疑某个节点本身有问题（反复失败、迟迟连不上）时用它绕开，不需要用户去改 .env。只在这次会话内生效，不是永久拓扑变更。 |
| `studio.retry_stage` | 干净地重试一个卡住的确定性阶段（preview / render / post / review）：先停掉可能还在跑的执行，再重新跑一次。用在「内容没问题，只是这次执行失败了」——节点抖动、连接超时、偶发故障。内容/提示词本身要改，用 studio.revise，不要用这个。 |
| `studio.self_review` | 验收的另一半：对成片做内容自评。技术验收（时长、画幅、镜头数、音轨）由控制面用 ffprobe 实测，它只证明片子是完整的；这个工具收的是「它好不好看」。固定五个维度各给一条结论，每条都要带一个**可指认的时间点**和在那一刻看见/听见了什么。它不改变技术验收的 passed，但不交这一份，作品就不算收尾。 |


## 错误码

| 错误码 | 含义 |
|---|---|
| `schema_violation` | 产物不符合阶段 schema，附字段路径 |
| `quality_violation` | 形状合规但内容不达标（禁用词、身份锁不一致等），附规则名与路径 |
| `invalid_transition` | 状态机不允许，附当前状态与合法动作 |
| `confirmation_required` | 有门的阶段提交时没带确认问题 |
| `gate_pending` | 确认门还挂着，不能推进 |
| `unknown_answer` | 应答的选项不在候选里 |
| `stage_not_ready` | 前置阶段还没通过 |
| `state_drift` | .studio/ 被外部改动，完整性校验失败 |
| `project_busy` | 另一进程持有本 bundle，附 PID |
| `not_a_project` | 当前目录不是一部作品 |
| `comfy_unavailable` | 无健康 ComfyUI 节点，结构化阻塞不降级 |
| `comfy_failed` | ComfyUI 侧执行失败 |
| `model_contract_violation` | 固定模型缺失或校验失败，停止不静默替换 |
| `artifact_missing` | 登记的产物在磁盘上不存在 |
| `tool_unavailable` | 找不到 ffmpeg / ffprobe 等外部程序 |
| `retry_limit_exceeded` | 阶段重试到顶 |
| `retry_stage_mismatch` | 请求重试的阶段不是当前真正卡住的那个确定性阶段 |
| `internal` | I/O、序列化等内部错误 |


## 用户在说什么

- 「从头开始」「做一个新的」→ 这是一部新作品，让用户自己在终端新建一个
  目录。不要在当前作品里覆盖着做。
- 「继续」「下一步」「现在到哪了」→ 调 `studio.status`。上下文不在对话里，在这个文件夹里。
- 「改一下 X」→ 调 `studio.revise`，然后重新提交。
