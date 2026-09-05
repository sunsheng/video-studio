# flux2_dev：卡片生成的基线

视觉资产阶段（角色卡 / 场景卡 / 道具卡）的两条基线，**都已真机核验**：

| 用途 | 文件 | 做什么 |
|---|---|---|
| 文生图 | `t2i.json` | 出主视图：角色的 `front_full`、场景的 `establishing`、道具的 `front` |
| 多参考编辑 | `multiref_edit.json` | 出其余视图：把**已定稿的全部视图**一起挂上去当锚，换机位/表情 |

两份都标 `"role": "card"`——**不是给镜头选的基线**，不进 Agent 的能力面
（`role` 机制见 SPEC-0015 §3.1）。写进某一镜没有意义。

## 权重

- diffusion：`flux2_dev_fp8mixed.safetensors`
- text encoder：`mistral_3_small_flux2_bf16.safetensors`（`CLIPLoader` type=`flux2`）
- vae：`flux2-vae.safetensors`

## 为什么是 FLUX.2 [dev]

选它的第一理由是**参考图容量**：纯文字身份锁锁不住脸（同一个身份锁逐字复用，
正面 / 四分之三 / 特写出来是三张不同的脸），而 dev 版一次吃**最多 10 张参考**，
够把「已定稿的全部视图」一起挂上去——出第 5 个视图时前 4 个都在场，
新视图必须同时与它们自洽。这叫**累积锁定**。

2026-09-05 三条路真机比过（同一身份提示词、同一套卡片规格 768×1344）：

| | 主视图耗时 | 构图一致性 | 覆盖范围 |
|---|---|---|---|
| **FLUX.2 dev** | 57 s | ✅ 四视图全身入画，一张没漂 | 任意视图 |
| Z-Image Turbo | 15 s | 主视图很好，多参考那条没验（配的是 Z-Image **Base**，权重不在） | — |
| MiniMax 转身抽帧 | 168 s / 4 视图 | ✅ 机位固定 | ❌ 只覆盖连续运动扫得到的 |
| MiniMax 逐视图独立采样 | 22 s | ❌ **会自己裁成中景**，写了 `full body head to toe` 也没守住 | 任意视图 |

**卡片是测量用的参考素材，构图不一致比画面不好看更伤**——一张全身一张中景，
喂进 R2V 时比例就不对。这一项 FLUX.2 赢得干净。

累积锁定的耗时随参考数涨：0 张 32 s、1 张 64 s、2 张 102 s、3 张 138 s。
**这条单调上升本身就是「参考真的进了 conditioning」的机械证据**——
上一次判断「参考生效」栽过（AUTOGROW 接线是死的），所以这里不只看画面。

阴性对照：同 seed 同提示词、一张参考都不挂，出来是个「相像但不同」的人
（脸型更圆、领口从方领变圆领、鞋子多了红条）。**参考确实起作用，但在提示词
已经写得很死、seed 又相同的情况下，边际影响是中等而不是压倒性的。** 如实记着。

许可是 Non-Commercial。本项目自用 / 研究，不构成约束。完整选型讨论见 issue #12。

## 接线来源

`Comfy-Org/workflow_templates` 的 `templates/image_flux2.json`，取
`definitions.subgraphs[0]`（"Image Edit (Flux.2 Dev)"），逐条按 `links` 展平。

两处**必须偏离模板**，各有理由：

| 模板写的 | 这里用 | 为什么 |
|---|---|---|
| `VAELoader: full_encoder_small_decoder.safetensors` | `flux2-vae.safetensors` | 模板那份权重这台机器上没有 |
| `LoraLoaderModelOnly: Flux_2-Turbo-LoRA` + 两个 `ComfySwitchNode` | 全部摘掉，走 20 步 | 模板的 `enable_turbo_mode` 默认 `false`，turbo LoRA 也不在机器上 |

`t2i.json` 是在此基础上再把参考分支（`VAEEncode` → `ReferenceLatent`）整条
摘掉，宽高直接给 `Flux2Scheduler` 与 `EmptyFlux2LatentImage`。

## 参考链是怎么表达的

参考数由内容决定（1..10），`_studio.bindings` 的固定路径数组喂不下，
所以 `multiref_edit.json` 多一段 `_studio.reference_chain`：

```jsonc
"reference_chain": {
  "nodes": {                       // 每张参考复制一份这三个节点
    "load":   { "class_type": "LoadImage",       "inputs": { "image": "" } },
    "encode": { "class_type": "VAEEncode",       "inputs": { "pixels": ["load", 0], "vae": ["vae", 0] } },
    "link":   { "class_type": "ReferenceLatent", "inputs": { "conditioning": null, "latent": ["encode", 0] } }
  },
  "asset":     "load.inputs.image",         // 素材文件名写这儿
  "chain_in":  "link.inputs.conditioning",  // 上一环的输出接这儿
  "chain_out": ["link", 0],                 // 本环的输出
  "head":      ["guidance", 0],             // 链条第一环接谁
  "tail":      "guider.inputs.conditioning",// 链条最后一环接到哪
  "max": 10
}
```

这是**链式**可变槽位，区别于 `minimax_h3` 那边 AUTOGROW 的**平铺编号**
（`"ref_images.ref_image_1": [...]`，见 SPEC-0014 §2.1）。两种都由
`studio-core` 里的确定性代码展开，Agent 看不见。

## `bindings_verified: true` 的依据

2026-09-05，A800 80GB / ComfyUI 0.34.0。**两份文件都填上绑定值原样提交跑通**：
`t2i` 768×1344 主视图 33 秒，`multiref_edit` 挂两张参考出正侧面 101 秒。
**人眼看过**——灰底匀光、全身入画、中性表情，符合卡片规格；两份出的是同一个人、
同一条裙子、同一双鞋。

跑通不等于画面对，所以这一条写的是「跑完出片并且人眼确认过」。
