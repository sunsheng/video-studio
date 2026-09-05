# 片段库的血缘

`fragments/` 里每一份文件都能追溯到一次真机跑通的记录——这是
[ADR-0005](../../../docs/decisions/ADR-0005-workflow-fragments.md) 把
「每份文件都要单独跑通」改成「血缘可追溯」之后的要求。

**接线逐字从基线抄，不按接口类型推断。** 理由见下面「为什么不许推断」。

## 从已验证基线切出来的

| 片段 | 切自 | run_id | 取了哪些节点 |
|---|---|---|---|
| `backbone.json` | `r2v.json` | `20260831T035154Z-29b70a` | `load_unet` `sigmashift` `load_clip` `vae_video` `vae_audio` `noise` `sampler_sel` `scheduler` `guider` `sampler` `decode_video` `decode_audio` `create_video` `save_video` |
| `head.reference.json` | `r2v.json` | 同上 | `h3_ref` |
| `head.image.json` | `i2v.json` | 同上 | `h3_i2v` |
| `input.image.json` | `r2v.json` | 同上 | `load_ref`（改名 `load`） |

### 骨架里被清空的三处

`guider.conditioning`、`sampler.latent_image`、`save_video.filename_prefix`
在片段里是**空的**，必须由组装器填。

原因：前两个在两份基线里分别指向各自的 head（`["h3_ref", 1]` /
`["h3_i2v", 1]`）。如果照抄 r2v 的值留在骨架里，换成 image head 时就会指向
一个不存在的 `h3_ref` 节点。第三个是验证时的输出路径，要按 `shot_id` 生成。

### 跟着 head 走的配套约束

`load_unet.unet_name` 与 `scheduler.scheduler` 在两份基线里不同，由 head 的
`backbone_overrides` 覆盖：

| | reference head | image head |
|---|---|---|
| 权重 | `minimax_h3_ref2va_int8_convrot` | `minimax_h3_fl2va_int8_convrot` |
| 调度器 | **`beta`** | **`simple`** |

**调度器这条差异不写在任何官方文档里**，是从两份已验证基线的 diff 里看出来的。
写进片段元数据就是为了不再依赖谁记得。

## 不是从基线切出来的（签名取自真机 `/object_info`）

| 片段 | 来源 | 验证状态 |
|---|---|---|
| `guide.image.json` `guide.clip.json` `guide.audio.json` | `/object_info/MiniMaxH3AddGuide` | ✅ 两个 AddGuide 串联 + 2 张 AUTOGROW 参考，真机图校验通过（2026-09-05） |
| `input.video.json` | `/object_info` 的 `LoadVideo` + `GetVideoComponents` | ✅ 真机出片，clip 锚点与 video 参考各一镜，画面人眼看过（2026-09-05，见下） |
| `input.audio.json` | `/object_info/LoadAudio` | ✅ 通道已验（audio 锚点真机跑通，2026-09-05，见下）；但 `ref_audios` 槽位单独标着未核验 |

`MiniMaxH3AddGuide` 是新节点（[ComfyUI#15439](https://github.com/Comfy-Org/ComfyUI/pull/15439)），
现有基线里没有它，所以只能从节点签名构造。但**接线方式是真机验过的**：

```
h3_ref ─[0] CONDITIONING─▶ guide1.positive
       └[1] LATENT ───────▶ guide1.latent
                            guide2.latent     ← 同样接 head，不是接 guide1
guide1 ─[0]──────────────▶ guide2.positive
guide2 ─[0]──────────────▶ guider.conditioning
h3_ref ─[1]──────────────▶ sampler.latent_image
```

AddGuide **只输出 CONDITIONING**，所以链式时只有 `positive` 串成链，
`latent` 一律接 head。

`bindings_verified: false` 的片段带着 `unavailable_reason`，控制面会拒绝用它们
渲染——跟未核验的整图基线是同一套规矩。真机跑通一整镜后改成 `true`。

`input.video`、`input.audio` 与四个 AUTOGROW 参考槽位都已于 2026-09-05 核验
（见下）。

## AUTOGROW 槽位一度整条是空转的（2026-09-05，已修）

**这是这个片段库出过的最严重的一个错，而且它躲过了全部既有测试。**

API 格式里 AUTOGROW 槽位是**平铺的点号兄弟键**：

```jsonc
"ref_images.ref_image_1": ["ref1_load", 0],
"ref_images.ref_image_2": ["ref2_load", 0]
```

组装器原来写的是嵌套对象 `{"ref_images": {"ref_image_1": [...]}}`。那个形状：

- 图校验通过 ✅
- 图跑得完 ✅
- 有产出文件 ✅
- **参考一个都没进模型** ❌

ComfyUI 的执行器不把嵌套对象里的 `["node", 0]` 当连线，加载节点成了死节点。
于是从 PR #19 合并到这次修复之间，`references` 声明了等于没声明——而
`#14` 整条 issue 的目的就是让参考能进渲染。

**怎么发现的**：查 `ref_audios` 为什么不生效时，先做阳性对照——换一张完全
不同的参考图，输出应该变。结果没变。再退一步：不挂参考图 / 挂纯绿 / 挂纯红，
三份输出**逐字节相同**。

**怎么定位形状**：把加载节点指向一个不存在的文件。连线被认出来，节点就是
活的，图校验会拒；认不出来就是死节点，校验照过。六种候选形态里只有
`ref_images.ref_image_1` 这种被拒。换上之后绿红两份画面立刻不同。

**留下的锁**：`real_comfy.rs` 里两条测试守着——换参考图画面必须不同、换音频
参考音轨必须不同。这是整套验收里一直缺的那条断言：之前所有测试只验到
「图合法、跑完了、有产出」，而这三样在错误形态下**全部成立**。

## ref_audios 是音色参考，不是「把这段声音放进去」

早先记过一条「`ref_audios` 不生效」，依据是 1kHz 纯音在输出里跟邻频持平
（0.5–1.9 倍）。**那条结论是错的**，两个原因叠在一起：

1. 当时 AUTOGROW 接线是上面那个死形态，音频参考根本没进模型；
2. 就算进去了，纯音也不是合适的判据——音色参考不该把纯音复现出来。

接线修好后重做，改用**音色 A/B**（同一提示词、同一 seed，只换参考音频）：

| | 参考基频 | 输出基频 | 输出谱质心 |
|---|---|---|---|
| 不挂参考 | — | 242 Hz | 1675 Hz |
| 挂低音参考 | 99 Hz | **121 Hz** | 2032 Hz |
| 挂高音参考 | 258 Hz | **262 Hz** | 2824 Hz |

输出音高跟着参考走，谱质心也跟着走。槽位已放开。

`guide.audio` 锚点那条老结论**仍然成立**（1kHz 纯音在输出里 4000 倍于邻频）——
锚点走的是 AddGuide 的具名输入，不经 AUTOGROW，没被这个 bug 影响。锚点是
「把这段声音放进输出」，参考是「照这个音色说话」，两件事。

## video 通道：核验经过（2026-09-05）

`clip` 锚点与 `kind: video` 参考共用 `LoadVideo` + `GetVideoComponents`。
两条各真机跑一镜，画面人眼看过：

- **video 参考**：640×384×22 帧，出来是正常的黏土网球场，提示词生效。
- **clip 锚点**：5 帧锚点接在 39 帧镜头开头。第 0 / 4 帧是锚点内容，
  第 6 帧起接管成网球场——正是接续要的语义。

素材经 `/upload/image` 上传，**那个端点收 mp4**，不需要另开一条路径。

**第一次跑是错的，而且「通过」了。** 最初用 22 帧锚点挂 22 帧镜头，两条都
跑完出片、测试全绿——但出来的整段就是锚点本身，提示词一个字都没生效。
等长的锚点把整镜钉死了。这个配置证明不了通道能用，按它翻 `bindings_verified`
就是拿一个无意义的绿色当证据。改成短锚点重跑才看到真行为。

教训跟 turbo 那次同源：**机器说"跑完了、有产出"，跟"验的是不是你以为的那件事"
是两回事。** 这一条写成了 V5 的后半句（锚点必须短于镜头）。

## audio 通道：进得去，但参考那一路没生效（2026-09-05）

音频听不了，所以拿**可判的信号**验：1kHz 纯音，用 Goertzel 量输出里 1kHz
与邻频（700 / 1400Hz）的能量比。

| 走法 | 1kHz / 邻频 | 结论 |
|---|---|---|
| `guide.audio` 锚点 | **~4000 倍** | 素材进去了，而且主导了输出 |
| `references: kind=audio` | 0.5–1.9 倍 | 一点痕迹都没有 |

参考那条**接线是对的**——从 ComfyUI 的 history 里把提交的图拉出来看过：
`ref_audios: {"ref_audio_1": ["ref1_load", 0]}`，`ref1_load` 是
`LoadAudio{audio: "anchor_tone.wav"}`，`audio_vae` 也接了。图能跑、有 1.625 秒
的 aac 音轨出来，就是模型不理这个参考。

**所以标志位分成了两层。** `input.audio` 的 `bindings_verified` 是 true
（通道验通了），而 `head.reference` 的 `ref_audios` 槽位单独标 `verified: false`。
不分开的话只能二选一：要么挡掉已经验通的锚点，要么把没生效的参考说成可用——
两个都不能接受。

`AutogrowSlot` 因此多了 `verified` / `unverified_reason` 两个字段，
组装器与提交时的校验都认它，错误消息里明说是「进去了但模型不理」而不是
「进不去」——两种错的下一步完全不同。

顺带实测到一条给 Agent 的信息：**audio 锚点影响的是整镜的声音**，不只锚定的
那 0.5 秒——整镜 1.625 秒里 1kHz 都在。所以它是给这一镜的声音定调，不要指望
它精确对齐到某一帧。这条写进了能力卡。

`ref_audios` 为什么不生效没查清：可能它要真实声音而非合成纯音，可能必须配
`ref_videos` 一起给，也可能它对输出的影响本来就很弱。查清楚并真机看到效果
之前，不放开。

## turbo 叠加层：图校验通过 ≠ 画面是对的

`overlay.turbo.reference` / `overlay.turbo.image` 挂官方的 turbo LoRA，
把 `scheduler.steps` 降到 LoRA 的步数（4 / 8），preview 用。

这两份的验证过程值得记一笔，因为它正好推翻了「图校验通过就算数」：

1. 第一版按 `load_unet → lora → sigmashift` 接，图校验**四种组合全过**。
2. 真机出片一看，reference + 4 步的画面是**坏的**——色带、光晕、底部有幻觉
   出来的字形，跟 20 步基准完全不能比。
3. 排查时试了五种变体。把 LoRA 换到 `sigmashift` 之后（B 变体），画面
   **一模一样地坏**——所以顺序不是原因，这两个模型补丁在 ComfyUI 里可交换。
4. 真正的原因是**调度器**：reference head 的配套档位是 `beta`，那是 20 步
   下的搭配，步数降到 4 就不成立了。换成 `simple` 立刻正常（C 变体）；
   保持 `beta` 但把步数提到 8 也正常（E 变体）。

所以 overlay 的 `backbone_overrides` 里显式写死 `scheduler = simple`，
盖掉 head 给的 `beta`。image 那份虽然 head 本来就是 `simple`，也照样显式写
——低步数下这个档位是成败关键，不该靠继承碰巧对上。

**真机耗时**（640×384×22 帧，同种子）：

| 组合 | 步数 | 调度器 | 耗时 |
|---|---|---|---|
| reference 普通 | 20 | beta | 8.6 / 10.2s |
| reference turbo | 4 | simple | 5.7 / 5.7s |
| image 普通 | 20 | simple | 10.9 / 10.4s |
| image turbo | 8 | simple | 8.5 / 7.8s |

这个尺寸下固定开销（VAE 解码、编码封装）占比大，所以只快 1.3–1.8 倍；
采样占比更高的真实预览尺寸上差距会拉开。

## 为什么不许按接口类型推断

写真机探针时，我按「AV latent 有视频和音频两路」推断
`decode_audio.samples` 应该接 `["sampler", 1]`。

**错了。** 真实基线接的是 `["sampler", 0]`——MiniMax H3 的 AV latent 是嵌套
结构，`VAEDecodeAudio` 自己从里面取音频部分。

两边的接口类型都是 `LATENT`，**类型系统挡不住这个错**。图能跑、不报错、出片，
只是音频是错的——正是本项目最怕的静默错接。

所以定成硬规则：片段的接线从已验证基线逐字抄，不许按签名推断。实在没有基线
可抄的（如 AddGuide），必须在真机上用 ComfyUI 自己的图校验验过接线才能标
`bindings_verified: true`。

## 怎么再生成一次

片段是用脚本从基线机械切出来的，不是手写的。基线更新后重新切，别手改
`fragments/` 里的文件。切分逻辑见本次改动的提交记录。
