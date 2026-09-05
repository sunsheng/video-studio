# 待办

只记真正没做完的事，做完就删。不记「以后也许可以」的想法。

## 不需要 GPU，现在就能做

### 一条会一直显示绿色的空断言

`real_comfy` 的 `an_unverified_input_channel_is_refused_rather_than_downgraded`
现在打印「跳过：所有输入通道都核验过了，这条测试的前提不再成立」然后**通过**。
它是绿的，但什么都没验——`image` / `video` / `audio` 三条通道全核验过了，
找不到未核验的通道来触发「宁可挡下，不要静默降级」那条规则。

**这个项目对这种东西的容忍度应该是零**：一条永远绿、永远什么都不验的测试，
比没有这条测试更糟，因为它让人以为那条规则有人守着。

两条出路，选一条：造一份只在测试里存在的未核验片段来触发它；或者把这条规则
下沉到单元测试层（那里可以随便捏一个 `verified: false` 的片段），真机这条删掉。
倾向后者——「未核验就阻塞」是纯逻辑，不需要 GPU 来证明。

这一节排在最前面是有原因的：下面那节「需要 ComfyUI」里的活，多数曾被
**#14 那条地基**堵着——参考图进不了渲染、卡片挂不上多个锚点、AddGuide
串不起来，根子都是基线的绑定格式表达不了「数量由内容决定」的槽位。
那条地基已经实现（PR #19），被它堵着的几项现在可以往下走了。

### 渲染工作流改为按镜头动态组装（issue #14）—— 已实现并合并（PR #19）

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

`input.audio` 也已核验：`audio` 锚点真机跑通（1kHz 纯音在输出里 4000 倍于邻频）。

`ref_audios` 一度被记成「模型不理这个参考」（同一段纯音 0.5–1.9 倍）。
**那条结论是错的，而且错在更上游**：AUTOGROW 槽位当时写成嵌套对象
`{"ref_images": {"ref_image_1": [...]}}`，ComfyUI 的执行器不认，加载节点
是死的——**整条参考通道都在空转**，跟音频没关系。图校验过、图能跑、有产出，
三样全成立，所以全部既有测试都没感觉。

改成平铺的点号键 `"ref_images.ref_image_1": [...]` 之后，四个槽位都真的生效了。
`ref_audios` 重做音色 A/B（同提示词同 seed，只换参考音频）：不挂参考输出基频
242Hz，挂 99Hz 低音参考出 121Hz、挂 258Hz 高音参考出 262Hz。它是**音色参考**，
不是「把这段声音放进输出」——拿纯音当判据本来就不合适。四个槽位现已全部放开。

真机验收补了两条一直缺的断言：换参考图画面必须不同、换音频参考音轨必须不同。
详见 `assets/workflows/minimax_h3/SOURCE-fragments.md`。

### ~~成片超分（issue #13）~~ —— 已实现并合并（PR #21）

`post` 逐镜用 SeedVR2 7B 超到交付规格（短边 1080），然后再拼接。
规格见 [SPEC-0015](specs/SPEC-0015-final-cut-upscale.md)。默认开，
`COMFY_UPSCALE=0` 关掉。实测同一镜渲染 63.9 秒、超分 42.1 秒（约 66%）。

**不走 issue 原文说的「2× 到 1536×2688 再裁」**：扩散跑在 resize 之后的
分辨率上，2× 是 2.4 倍像素而最终还要缩回去，白花 112 秒 vs 40 秒。
直接 `scale dimensions` + `crop=center` 定到 1080×1920。

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

### ~~视觉资产执行器~~ —— 已实现（SPEC-0016）

卡片终于真的会被生成出来。`Pipeline::visual_assets` 主视图走
`flux2_dev/t2i`，其余视图走 `flux2_dev/multiref_edit` 并按 `derived_from`
累积挂上已定稿的视图；`derived_from` 已从单值改成列表，V10–V12 三条校验
在提交那一刻就报。`StageKind::Hybrid` 也真的会执行了——**先生成，再上确认门**
（门要人看的是卡片，不是一份没见过的 JSON）。

选型换了：**卡片走 FLUX.2 dev，不走视频抽帧**。当初想抽帧是因为 MiniMax
没有静态图出口，那是被模型逼的；这台机器上 FLUX.2 dev 的权重后来齐了，
约束就不成立了。三条路真机比过，FLUX.2 在**构图一致性**上赢得干净——
MiniMax 逐视图独立采样会自己裁成中景，写了 `full body head to toe` 也没守住。
完整对比见 `assets/workflows/flux2_dev/README.md`。

`docs/prompt-architecture.md` §6 已按 FLUX.2 累积锁定重写，
`config/models.toml` 的 `[flux2_dev]` / `[seedvr2]` 也填上了真实文件名。

**还剩：每镜首帧图的控制点**没做（i2v 那条）。

十阶段端到端跑出来两个缺陷，都已修，形状记在 §6.8：

1. **上传参考图跟控制面共用 30 秒读超时**，而实测传一张 1.09 MB 卡片图要
   25–38 秒（那条代理当时有已知故障，数字不是干净基线；故障期外量到过 14.5
   秒。结论不依赖具体数字）——超时值正压在观测耗时的中位数上。表现不是
   「上传失败」这么直白：
   锚点图出来了、落盘了、status 是 ready，只是传不回去，于是后面五个派生视图
   全报「参考图还没生成出来」，一句与事实相反的话。大块传输现在走独立的
   5 分钟读超时，失败原因也分「没生成」和「传不回去」两种记。
2. **部分失败被放行。** §6.8 早就写着「部分 ready、部分 failed 不放行」，
   实现里却只在 `ready == 0` 时才拦——六个视图成一个照样上确认门。
   规格写了不等于实现了，尤其这种只在坏路径上才走到的分支。
3. **AGENTS.md 没说 `waiting_on: system` 该怎么办。** 控制面发的是
   `next_action.kind: "await"`，但工作习惯那节从没解释这个值——于是 Codex
   在第一个要跑几分钟的阶段（visual_assets 正在出卡片图）上写了段总结就
   收尾了，十个阶段走到第五个就停。补了第五条工作习惯和一节专门说明：
   等，隔几十秒再 `studio.status`，别结束这一轮；耗时长不是卡住了。

顺带在对抗性重读时补掉一条：`status` / `path` / `provenance` 都是 Agent
能写的字段，而 `retry_stage` 会把上一轮输出原样喂回来。`provenance` 提交成
字符串时下标赋值会 panic 掉 worker；视图标着 ready 却没有 path 会被静默
跳过。两条都挡住了。

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

**render 往后没有。** 那一半要真实 ComfyUI + GPU + ffmpeg——**这台机器现在
三样都有了**（见上面的探针结果），不必再等生产机。步骤见 `docs/e2e.md`。


### 渲染与后期的真实链路

`render` 那一段已经对着真实 ComfyUI 跑过了（#14 的真机验收，7 项全过），
`preview` 的 turbo 组合也是。**没跑过的是 `post` / `review`**——ffmpeg 拼接、
字幕挂载、封面抽帧、ffprobe 核对这一串，测试只用过假执行器。

`preview` 按短边 480 缩放后的尺寸对齐问题已经暴露并修掉：768×1344 缩到 480
得 840，不是 32 的倍数，片段化的系列现在会取整到 32（见 SPEC-0014 V8）。

### ~~核验四份 workflow 基线~~（**不做了，2026-09-05 定**）

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

**2026-09-05 定：这条不做了。** 先只用一个模型，把整条链路做完整、做正确——
备选系列的核验不产生画面质量收益，反而分散精力。全部精力放在 `minimax_h3`
的深度利用上（多参考、AddGuide、卡片链路）。

这四份**保留在仓库里、保持未核验状态**：控制面本来就拒绝用未核验的基线渲染，
留着不构成风险，将来真要换系列时图还在。整图基线这条路也一并保留——#14 的
片段化只对 `minimax_h3` 做，两种形式共存是有意的。

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
