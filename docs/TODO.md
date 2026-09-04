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

### 端到端跑一次真实 Codex 会话

`docs/e2e.md` 写好了步骤，但还没在生产环境真跑过。
开发环境只能验证协议层（`scripts/replay-protocol.py`），
验证不了「Codex 读完 AGENTS.md 和 SKILL.md 之后会不会正确使用工具面」——
那才是端到端真正要看的东西。

跑完把 `report.json` 带回开发环境分析。

### 渲染与后期的真实链路

`render` / `post` / `review` 的代码写完了，测试用假执行器覆盖了状态流转，
但**没有对着真实 ComfyUI 和真实 ffmpeg 跑过**。
第一次真跑大概率会暴露参数细节问题，属于预期内。

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
