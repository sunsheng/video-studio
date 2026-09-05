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
| `input.video.json` | `/object_info` 的 `LoadVideo` + `GetVideoComponents` | ⚠️ `bindings_verified: false`——节点存在、输出类型对得上，但没跑通过一整镜 |
| `input.audio.json` | `/object_info/LoadAudio` | ⚠️ `bindings_verified: false`——同上 |

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

两个 `bindings_verified: false` 的片段带着 `unavailable_reason`，控制面会拒绝
用它们渲染——跟未核验的整图基线是同一套规矩。真机跑通一整镜后改成 `true`。

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
