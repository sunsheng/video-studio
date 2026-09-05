# 待办

只记真正没做完的事，做完就删。不记「以后也许可以」的想法。

## 需要一台有 ComfyUI 的机器

### 核验四份 workflow 基线

这四份从前身仓库带过来了，图是完整的，但**参数绑定没核验，当前标记为不可用**，
控制面拒绝用它们渲染——绑错节点会静默产出错的画面，比直接报错难查得多。

| 基线 | 卡在哪 |
|---|---|
| `wan2_2/i2v` | 正负提示词与尺寸走的连线尚未确认：`WanImageToVideo` 的 width/height 来源、两个 `CLIPTextEncode` 哪个是负向 |
| `wan2_2/flf2v` | 同上，且首尾帧输入需要额外的图片上传流程 |
| `ltx2_5/flf2v` | 首尾帧变体的尺寸经由 `ResizeImageMaskNode` 推导，与 t2v/i2v 的 Primitive 链不同 |
| `wan_animate2/i2v` | 需要一段驱动视频作为输入，当前流水线不提供；且不属于默认三系列 |

做法：在目标 ComfyUI 上跑通一次，确认每个参数落到哪个节点的哪个输入，
补进 `assets/workflows/<系列>/<用途>.json` 的 `_studio.bindings`，
删掉 `unavailable_reason`，把 `bindings_verified` 改成 `true`。
**不需要改代码。** 改完跑 `studio-cli workflows check` 验证。

已核验可用的六份（`minimax_h3` 三份、`wan2_2/t2v`、`ltx2_5` 两份）可以作参考。

### 给视频基线补图片输入绑定

`minimax_h3/i2v` 和 `r2v` 的**节点图里已经有图片输入节点**，但 `_studio.bindings`
没有把它们暴露出来：

| 基线 | 图里已有的节点 | 要绑成什么 |
|---|---|---|
| `minimax_h3/i2v` | `load_first` / `load_last`（都是 `LoadImage`） | 首帧、尾帧两个入口 |
| `minimax_h3/r2v` | `load_ref`（`LoadImage`） | 参考图入口 |

现在的后果：提示词包里的 `references` 写了也进不了渲染请求——它只被登记下来，
不会变成图片喂给模型。**角色卡做出来也没有通道进渲染**，这是画面一致性链条上
断掉的那一环，见 `docs/prompt-architecture.md` §2.4。

做法：在目标 ComfyUI 上确认这几个 `LoadImage` 节点吃的是什么（文件名？
先经 `/upload/image` 上传？多节点集群要不要按节点分别传？），把
`references` 加进 `_studio.bindings`，真机跑通一次再提交。
**这一项是视觉资产生成（下一条）的硬前置**：卡片做出来进不了渲染就还是纸面计划。

### 导出 `z_image` 的两条基线

视觉资产阶段（角色卡 / 场景卡 / 道具卡）要两条基线，目前一条都没有：

| 用途 | 文件 | 做什么 |
|---|---|---|
| 文生图 | `assets/workflows/z_image/t2i.json` | 出主视图 |
| 参考图生图 | `assets/workflows/z_image/edit.json` | 以主视图为参考图出其余视图 |

**导出要求已经写好**，在 `assets/workflows/z_image/README.md`：要绑哪些参数、
`_studio` 长什么样、`bindings_verified` 什么时候才能置 true，逐条写死了。
照着做即可，不需要在这里重复。

同时把 `config/models.toml` 的 `[z_image]` 段填上真实文件名（现在是注释掉的
占位）。开发环境没有 GPU 也没有 ComfyUI，出不了真机导出，所以这两件事都只能
在生产机上做。

做完之前 `studio-cli doctor` 会一直报「卡片生成基线未就绪」——那是提醒，
不是故障：资产计划照样能提交，只是 `status` 一直停在 `planned`、生不出图。

### 视觉资产执行器与首帧图控制点（**先别在没有 GPU 的机器上写**）

设计已经定完，见 `docs/prompt-architecture.md` §6.4 与批次 3、4：
`trait ImageBackend`（文生图 + 参考图生图）与 `ZImageBackend` 实现、
主视图先行、逐视图参考图锚定、落盘 `media/assets/`、门改为看图确认，
以及每镜首帧图的控制点。

**代码没写，是有意的。** 这一块的价值全在真机行为上——尺寸对齐、参考图上传、
逐视图重试、失败阻塞的判据，在开发环境只能拿假节点测状态流转。
这份清单最后那条「渲染与后期的真实链路」就是这么来的：代码写完了、
假执行器测过了、第一次真跑仍然大概率暴露参数细节问题。
再提前写一套，只是把同一笔债翻倍。

等能连上 ComfyUI 时，把这一条和前面两条**一起做**：导出基线 → 补绑定 →
写执行器 → 真跑一轮。一轮就能收敛，比分三次各猜一半省事。

`asset_plan` 的 schema、视图词表、结构校验（视图齐全、主视图唯一、
`derived_from` 指向锚点、同卡画幅一致、身份锁逐字包含）已经在 `studio-core`
里跑起来了，不需要重做——差的只有执行那一半。

### 端到端跑一次真实 Codex 会话（**render 之后那一半**）

render 之前的六个阶段已经用真实 Codex 会话跑过了（开发环境，`gpt-5.6-sol`）：
22 次调用 0 失败、修订往返 2 次调用、全程没有绕过 MCP，停在 `preview`
的 `waiting_on: system`。「Codex 读完 AGENTS.md 和 SKILL.md 之后会不会正确
使用工具面」这个问题，前六个阶段有答案了。

**render 往后没有。** 那一半要真实 ComfyUI + GPU + ffmpeg，开发环境跑不出真
信号，顶多验证到「提交后结构化阻塞在 `comfy_unavailable`」。步骤见
`docs/e2e.md`，跑完把 `report.json` 带回开发环境分析。

### 渲染与后期的真实链路

`preview` / `render` / `post` / `review` 的代码写完了，测试用假执行器和
本机 TCP 假节点覆盖了状态流转、并发分片、轮询容错、重试路径，
但**没有对着真实 ComfyUI 和真实 ffmpeg 跑过**。
第一次真跑大概率会暴露参数细节问题，属于预期内——尤其是 `preview`
按短边 480 缩放后的尺寸是否符合各模型系列自身的对齐要求，需要真机验证。

## 已知限制

### Linux 包依赖 glibc

当前用 `x86_64-unknown-linux-gnu` 目标，需要目标机器的 glibc 不低于构建机。
换成 musl 静态链接会更「绿色」，但 `rusqlite` 的 bundled SQLite 要 `musl-gcc`，
我没有能验证的环境，不想推一个没跑过的构建。要做的话在 release 工作流里
装 `musl-tools` 并加 `x86_64-unknown-linux-musl` 目标。

### 没有 redo

`studio.undo` 是撤销栈，可以连着往回走，但没有反向的 redo。
撤销之后想再回去只能重新提交。加 redo 需要另一个栈，目前没有实际需求。

### Codex 沙箱的读写边界没实测

`.studio/` 的保护现在是三层：dotdir 约定 + AGENTS.md 明确禁止 + 完整性摘要兜底。
第三层能发现篡改但不能阻止。真正的阻止要靠 Codex 的沙箱配置，
而受限 profile 到底是只限写还是读写都限，我没有实测过。

结果影响设计：如果只限写，`.studio/studio.db` 仍然能被读出来，
虽然改不了，但状态是可见的。实测半小时就能有结论。

## 完成后从这里删掉

这份清单不追加历史，做完的条目直接删。要看做过什么，看 git log。
