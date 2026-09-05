# 待办

只记真正没做完的事，做完就删。不记「以后也许可以」的想法。

## 不需要 GPU，现在就能做

这一节排在最前面是有原因的：下面那节「需要 ComfyUI」里的活，多数曾被
**#14 那条地基**堵着——参考图进不了渲染、卡片挂不上多个锚点、AddGuide
串不起来，根子都是基线的绑定格式表达不了「数量由内容决定」的槽位。
那条地基已经实现（PR #19），被它堵着的几项现在可以往下走了。

### 渲染工作流改为按镜头动态组装（issue #14）—— 已实现，待合并

方案见 [ADR-0005](decisions/ADR-0005-workflow-fragments.md) /
[SPEC-0014](specs/SPEC-0014-dynamic-workflow-assembly.md) /
[PLAN-0014](plans/PLAN-0014-dynamic-workflow-assembly.md)，实现在 PR #19。
S0–S8 全部落地，CI 绿，真机验收 5 项全过。

落成的样子：基线降级为片段库（backbone / head / guide / input / overlay），
Agent 每镜声明 `head` + `references` + `guides` 而不是选一张整图，
`studio-core` 的确定性组装器把声明翻译成节点图，组合合法性校验 V1–V9
在提交那一刻就报，每条带 remedy。整图基线那条路保留给 `ltx2_5` / `wan2_2`，
schema 按 `core_model_family` 分派两种形状，不让 Agent 混着写。

**做的时候翻出来的三件事**，都记在 SOURCE-fragments.md 与 SPEC 里：

1. **图校验通过不等于画面是对的。** turbo 叠加层四种组合的 ComfyUI 图校验
   全过，真机出片才发现 reference + 4 步是坏的。原因不是接线顺序（换到
   sigmashift 之后画面一模一样地坏），是调度器——`beta` 是 20 步下的配套档位，
   步数降到 4 就不成立。所以真机验收必须真等出片，不能只看 `node_errors`。
2. **`clip` 锚点原本接的是一张静帧。** 它映射到 `LoadImage`，而帧序列得走
   `LoadVideo` + `GetVideoComponents`。两条路输出都是 `IMAGE`，图能过校验。
3. **接续镜要排队。** 引用 `sh01.tail` 的镜头得等 sh01 出片才有东西可裁，
   调度改成按依赖分波；上一镜的尾段也不在 `visual_assets` 里，用 ffmpeg 现裁。

`input.video` 已于 2026-09-05 真机核验并放开：`clip` 锚点（5 帧接 39 帧镜头）
与 `kind: video` 参考各跑一镜，画面人眼看过。连带补了 V5 的后半句——**锚点
必须短于这一镜**，等长的会把整镜钉死，那是第一次跑用等长锚点、测试全绿但
画面全错才发现的。

还剩 `input.audio`：独立音频参考与 audio 锚点没在真机上跑通过，仍是结构化
阻塞状态，能力卡上划掉了。

### 成片超分（issue #13）

MiniMax H3 的原生画布是短边 768（16:9 即 1344×768），而交付要 1080×1920。
`post` 现在只做拼接、字幕、封面抽帧，**没有超分**，成片就是 768 短边直接交付。

候选是 SeedVR2 7B（2×–4×，带 tiling 和色彩校正）。要定的：逐帧还是视频超分
（逐帧会闪）、放在拼接前还是拼接后、耗时占比、要不要做成可选步骤。

注意**不要**把超分放到卡片链路上：卡片是喂给 R2V 的参考，
`ref_image_size` 取值是 `"match"`，超分过的参考图进去也会被缩放对齐。

## 这台机器探到了什么（2026-09-05 实测）

**别再假设「开发环境没有 GPU」** —— 那个前提已经不成立。这次会话探到的：

| 探针 | 结果 |
|---|---|
| ComfyUI | 0.34.0，**A800 80GB PCIe**（79.2 GiB，实时空闲 59.6 GiB），经带 Bearer token 的负载均衡代理 |
| Z-Image Turbo 文生图 | **5/5 成功，21.9 秒**（768×1344 / 1344×768 / 1024² 三种画幅），中文「青山茶馆」四字渲染正确，灰底匀光全身卡片一次到位 |
| `MiniMaxH3AddGuide` | **节点在** |
| AddGuide 能接 R2V | **接口层确认**：R2V 输出 `[CONDITIONING, LATENT]`，AddGuide 要 `positive: CONDITIONING` + `latent: LATENT` |
| R2V 多参考 | 四个槽位 `ref_images` / `ref_videos` / `ref_video_audios` / `ref_audios`，类型 **`COMFY_AUTOGROW_V3`**（ComfyUI 原生的「按需增长」类型） |
| MiniMax 权重 | fl2va / ref2va / 剪枝版齐全，**turbo LoRA 两份都在**（`fl2v_turbo_8step`、`ref2v_turbo_4step`） |
| Codex | 0.153.4 + `gpt-5.6-sol`，无 metadata 警告 |
| FLUX.2 | **节点在，权重不在** |
| SeedVR2 | **节点在，权重不在**（upscale 只有 `RealESRGAN_x4plus`） |
| ffmpeg / ffprobe | **未安装** |

两条要记住的：

1. **「节点在」不等于「能用」。** FLUX.2 和 SeedVR2 的节点都在 `object_info`
   里，但权重没下载。探针要探到权重那一层。
2. Z-Image 的 `TextEncodeZImageOmni` 有 `image1` / `image2` / `image3`——
   **它吃 3 张参考图，不是单参考**。issue #12 里「Z-Image 是单参考」的说法
   是错的，那条论据不成立（累积锁定的对比结论仍待实测）。

### 权重已全部到位（2026-09-05 核对）

| 用途 | 文件 | 状态 |
|---|---|---|
| 成片超分（#13） | `seedvr2_7b_int8_convrot` / `seedvr2_3b_int8_convrot` / `seedvr2_ema_vae_fp16` | ✅ |
| 卡片累积锁定（#12） | `flux2_dev_fp8mixed` / `mistral_3_small_flux2_bf16` / `flux2-vae` | ✅ |
| 卡片草稿 / 已验证 | `z_image_turbo_bf16` + `qwen_3_4b` + `ae` | ✅ |

SeedVR2 走 **ComfyUI 原生节点**（`SeedVR2Preprocess` / `SeedVR2Conditioning` /
`SeedVR2TemporalChunk` 都在），不要装第三方 custom node。

**这意味着 #12 与 #13 不再被权重卡住**：#12 的累积锁定实测（卡片路线唯一
剩下的支点）现在可以直接跑 FLUX.2 的 10 参考，不必退而求其次用 Z-Image 的
3 参考；#13 的超分链路可以真机验证。两者仍排在 #14 之后——#12 依赖 #14 的
可变槽位，#13 独立但优先级低于地基。

### 装 ffmpeg

云端会话的 setup script 加一行 `apt-get update && apt-get install -y ffmpeg`
（带 ffprobe）。装上之后十个阶段可以在这台机器上走完，不必再等生产机。

## 需要一台有 ComfyUI 的机器

### ~~确认 ComfyUI 版本带 `MiniMaxH3AddGuide`~~（已探到，见上）

这个节点把关键帧从「只能锚首尾」放开成「锚任意帧」，且能接 clip 和音频作为
guide。它是镜头续接的核心手段——把上一镜的最后 22 帧连同音频喂进下一镜的
frame 0，模型生成的是两条流的续接，比抽一张静止尾帧强得多。

它是 [Comfy-Org/ComfyUI#15439](https://github.com/Comfy-Org/ComfyUI/pull/15439)
加进来的，**生产机的 ComfyUI 版本得够新**。顺带在 UI 上确认一件事：
把 `MiniMaxH3ReferenceToVideo` 的 positive/latent 接到 `AddGuide` 上能不能连。
读代码看是能的（socket 是通用的 `Conditioning` + `Latent`，而且那个 PR 为了
让 AddGuide 复用，专门把 R2V 的 `_encode_ref_audio` 提到了模块级），但没有
真机连过线。

**能连的话，成片阶段可以统一走 R2V**——身份靠参考、构图靠 guide，
`fl2va` 权重不必常驻，省下的 19.5G 正好够 FLUX.2 一起装进 80G。

顺带把图片输入的接法确认清楚，#14 的片段库要用：`minimax_h3/i2v` 的
`load_first` / `load_last`、`r2v` 的 `load_ref`（都是 `LoadImage`）究竟吃
什么——直接给文件名，还是要先经 `/upload/image` 传上去？多节点集群要不要
按节点分别传？R2V 挂多张参考时是并列多个 `LoadImage`，还是单节点接列表？

（这几个节点**本来就在图里**，只是 `_studio.bindings` 没暴露。原先的做法是
「补一行绑定」，那只够挂 1 张图——正是 `r2v.json` 现在 image-only 的状态。
官方支持 9 图 + 3 视频 + 3 音频，补绑定够不着，所以这一项并进了 #14。）

### FLUX.2 累积锁定的一致性实测（**卡片路线唯一剩下的支点**）

在导出基线、写执行器之前先做这个，成本很低、结论决定性。

用上次那个把脸锁崩了的身份锁（纯文字锚定，四个视图出来是三张不同的脸），
出同样四个视图：`front_full` → `three_quarter` → `profile` → `face_close`，
每一步把**已定稿的全部视图**挂上去当参考，看四张是不是同一个人。

跟上次的单参考结果直接对比。**锁不住就别急着导基线**——那说明卡片路线要
重新选型。

### 导出 `flux2_dev` 的两条基线

视觉资产阶段（角色卡 / 场景卡 / 道具卡）要两条基线，目前一条都没有：

| 用途 | 文件 | 做什么 |
|---|---|---|
| 文生图 | `assets/workflows/flux2_dev/t2i.json` | 出主视图 |
| 多参考编辑 | `assets/workflows/flux2_dev/multiref_edit.json` | 挂已定稿的全部视图累积锁定，出其余视图 |

**导出要求已经写好**，在 `assets/workflows/flux2_dev/README.md`：要绑哪些参数、
`_studio` 长什么样、`bindings_verified` 什么时候才能置 true，逐条写死了。

同时把 `config/models.toml` 的 `[flux2_dev]` 段填上真实文件名（现在是注释掉的
占位）。

**参考图那一项现在绑不了**，跟渲染那边的 R2V 撞的是同一堵墙（可变数量槽位），
解法在 issue #14。所以这一轮导出能做的是「把图导出来 + 绑固定参数」，
外加**记录参考图的接法**（`/upload/image` 之后承接的节点叫什么、多张参考是
并列多节点还是单节点接列表）——那些信息 #14 的片段库要用。

做完之前 `studio-cli doctor` 会一直报「卡片生成基线未就绪」——那是提醒，
不是故障：资产计划照样能提交，只是 `status` 一直停在 `planned`、生不出图。

### 视觉资产执行器与首帧图控制点（**先别在没有 GPU 的机器上写**）

**注意 `docs/prompt-architecture.md` §6 已经过时**：那一章写的是 Z-Image 单后端、
「主视图先出、其余视图一律以主视图为参考」。现在的结论是 FLUX.2 累积锁定
（出视图 N 时挂上已定稿的 1..N-1）加 MiniMax 锚视频，见 issue #12。
§6 的重写等上面那项实测出结果再做——现在改是在猜。

要做的执行器：主视图先行、逐视图**累积**锚定、落盘 `media/assets/`、
产物登记、门改为看图确认，以及每镜首帧图的控制点。

**代码没写，是有意的。** 这一块的价值全在真机行为上——尺寸对齐、参考图上传、
逐视图重试、失败阻塞的判据，在开发环境只能拿假节点测状态流转。
这份清单最后那条「渲染与后期的真实链路」就是这么来的：代码写完了、
假执行器测过了、第一次真跑仍然大概率暴露参数细节问题。
再提前写一套，只是把同一笔债翻倍。

`asset_plan` 的视图词表与结构校验（视图齐全、主视图唯一、同卡画幅一致、
身份锁逐字包含）已经在 `studio-core` 里跑起来了。但 **`derived_from` 还是
单值**（指向主视图），累积锁定要求它变成**数组**（指向所有已定稿视图）——
这处 schema 改动跟 #14 的可变槽位一起做，不在生产机上做。

### 锚视频与镜头续接的真机验证

issue #12 定的链路里，MiniMax 锚视频是卡片之外的第二路参考——它给 R2V 的
3 个 video reference 槽位提供内容，带的是图片参考给不了的运动先验。要验的：

- 一段 2 秒（49 帧）锚片段在 A800 80G 上的实际耗时，决定一个角色出几个视图
- FLUX.2 出的卡片 vs MiniMax 锚视频抽的帧，回喂 R2V 的参考效力差多少
  （跨模型域差有多大，决定锚视频这一路值不值得做）
- 上一镜末 22 帧 + 音频作为下一镜 frame 0 的 guide，接缝质量如何
- FLUX.2 dev fp8 与 MiniMax `ref2va` 能否真的同时常驻 80G

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

（#14 之后 `preview` 还能更便宜：片段化以后可以换 turbo LoRA 组合——官方有
`ref2v_turbo_4step`——并去掉音频解码分支，而不是只把分辨率缩到 480p。
门要看的本来就只是构图和内容。）

### 核验四份 workflow 基线（**最低优先级**）

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

**优先级压到最低，别让它分心。** 现在全部精力在 `minimax_h3` 的深度利用上
（多参考、AddGuide、卡片链路），备选系列的核验不产生画面质量收益。
这些系列**保持整图基线**，#14 的片段化只对 `minimax_h3` 做——只有它需要
可变槽位。两种形式共存是有意的。

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
