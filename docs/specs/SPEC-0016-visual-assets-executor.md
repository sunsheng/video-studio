# SPEC-0016 视觉资产执行器：卡片真的生成出来

| | |
|---|---|
| 对应 issue | [#12](https://github.com/sunsheng/video-studio/issues/12) |
| 状态 | 设计中 |
| 前置 | [PR #22](https://github.com/sunsheng/video-studio/pull/22)（AUTOGROW 参考槽位修复）——参考不进模型的话，卡片生成出来也没用 |

---

## 1. 问题

`Pipeline::execute` 里**没有 `VisualAssets` 分支**。阶段图把它标成
`StageKind::Hybrid`（「Agent 定内容，控制面执行生成」），Agent 也确实在提交
资产计划——但控制面从来没有执行过，**角色卡一张都没生成过**。

后果沿着链路一路空掉：

- `AssetResolver::registered_asset()` 查到视图的 `status` 永远是 `planned`，
  直接报 `artifact_missing`
- 于是 `prompt_pack` 里写 `references: ["C01"]` 的镜头根本渲不了
- 而参考锁身份是 #12 全部讨论的落点

**这是「功能不完整」最大的那个洞**，补上它 = 让现实追上早就声明好的契约。

---

## 2. 选型：FLUX.2 dev 累积锁定

### 2.1 为什么不是视频抽帧

MiniMax H3 的三份基线全是 `CreateVideo → SaveVideo`，**没有静态图出口**——
用它出卡片在物理上只能「渲一小段再抽帧」。那是被模型逼的，不是选出来的。

2026-09-05 重探这台机器，**FLUX.2 dev 的权重已经齐了**
（`flux2_dev_fp8mixed` + `flux2-vae` + `mistral_3_small_flux2`），
所以这个约束不成立了。

### 2.2 三条路的实测对比

同一个身份提示词、同一套卡片规格（768×1344，中性灰底，匀光，全身入画）：

| | 主视图耗时 | 多视图一致性 | 构图一致性 | 覆盖范围 |
|---|---|---|---|---|
| **FLUX.2 dev** | 57 s | ✅ 累积锁定，最多 10 张参考 | ✅ **四视图全身入画，一张没漂** | 任意视图 |
| Z-Image Turbo | 15 s | 未验（多参考走 `TextEncodeZImageOmni`，配的是 Z-Image **Base**，权重不在） | — | — |
| MiniMax 转身抽帧 | 168 s / 4 视图 | ✅ 同一次采样，物理上同一个人 | ✅ 机位固定 | ❌ 只覆盖连续运动扫得到的 |
| MiniMax 逐视图独立采样 | 22 s（5 帧）| ✅ 参考锁得住 | ❌ **会自己裁成中景**，提示词写了 `full body head to toe` 也没守住 | 任意视图 |

**卡片是测量用的参考素材，构图不一致比画面不好看更伤**——一张全身一张中景，
喂进 R2V 时比例就不对了。这一项 FLUX.2 赢得干净。

### 2.3 累积锁定实测

出第 N 个视图时，把主视图 + 已定稿的视图 1..N-1 全挂上去：

| 视图 | 参考数 | 耗时 |
|---|---|---|
| `front_full`（主视图，t2i） | 0 | 57 s |
| `three_quarter` | 1 | 64 s |
| `profile` | 2 | 102 s |
| `back` | 3 | 138 s |

四个视图同一个人、同一条裙子（方领、抽褶）、同一双鞋，全身入画一致。

**阴性对照**：同 seed 同提示词、一张参考都不挂，出来是个「相像但不同」的人
——脸型更圆、领口从方领变圆领、鞋子多了红条。参考确实在起作用，但在提示词
写得很死、seed 又相同的情况下，**边际影响是中等而不是压倒性的**。更硬的
机械证据是耗时随参考数单调上升（32 → 64 → 102 → 138 s），参考 latent
确实进了 conditioning。

> 这一条要如实记着。上一次「参考生效」的判断栽过（AUTOGROW 那次接线是死的），
> 所以这里同时给了视觉证据和机械证据，并且写明了效应量级，不夸大。

### 2.4 各模型的位置

| 模型 | 位置 |
|---|---|
| **FLUX.2 dev** | **卡片定稿的唯一后端。** 主视图 t2i，其余视图累积锁定编辑 |
| Z-Image Turbo | **草稿档**。15 秒一张，分镜阶段批量试构图用；定稿不走它 |
| MiniMax H3 | 卡片链路上**退场**。它的锚视频回到本来的用途——填 R2V 的 `ref_videos` 给运动先验，那是 `render` 的事，不是卡片的事 |

---

## 3. 接线

来源：`Comfy-Org/workflow_templates` 的 `templates/image_flux2.json`，
取 `definitions.subgraphs[0]`（"Image Edit (Flux.2 Dev)"）。

```
CLIPLoader(mistral_3_small_flux2_bf16, type=flux2) ─▶ CLIPTextEncode ─▶ FluxGuidance(4)
                                                                            │
参考图 ─▶ LoadImage ─▶ VAEEncode ─▶ ReferenceLatent ◀───────────────────────┘
                                        │ （多张参考就把 ReferenceLatent 串成链）
                                        ▼
UNETLoader(flux2_dev_fp8mixed) ─▶ BasicGuider ◀─ conditioning
Flux2Scheduler(steps,w,h) ─┐
EmptyFlux2LatentImage(w,h) ─┼─▶ SamplerCustomAdvanced ─▶ VAEDecode ─▶ SaveImage
RandomNoise(seed) ─────────┤
KSamplerSelect(euler) ─────┘
```

两处**必须偏离模板**，各有理由：

| 模板写的 | 这里用 | 为什么 |
|---|---|---|
| `VAELoader: full_encoder_small_decoder.safetensors` | `flux2-vae.safetensors` | 模板那份权重这台机器上没有 |
| `LoraLoaderModelOnly: Flux_2-Turbo-LoRA` + 两个 `ComfySwitchNode` | 全部摘掉，走 20 步 | 模板的 `enable_turbo_mode` 默认 `false`，turbo LoRA 也不在机器上 |

纯文生图（主视图）= 把参考分支（`VAEEncode` → `ReferenceLatent`）整条摘掉，
宽高直接给 `Flux2Scheduler` 与 `EmptyFlux2LatentImage`。

### 3.1 两份新基线

```
assets/workflows/flux2_dev/t2i.json            主视图，无参考
assets/workflows/flux2_dev/multiref_edit.json  其余视图，1..10 张参考
assets/workflows/flux2_dev/SOURCE-README.md
```

两份都标 `"role": "card"`——**不是给镜头选的基线**，不进 Agent 的能力面
（`role` 机制见 SPEC-0015 §3.1）。

`multiref_edit` 的参考数是可变的，`_studio.bindings` 的固定路径数组表达不了。
按 SPEC-0014 的做法，它用 **AUTOGROW 风格的槽位声明**：

```jsonc
"_studio": {
  "role": "card",
  "bindings": {
    "positive": ["pos.inputs.text"],
    "width":    ["sigmas.inputs.width", "latent.inputs.width"],
    "height":   ["sigmas.inputs.height", "latent.inputs.height"],
    "seed":     ["noise.inputs.noise_seed"],
    "output_prefix": ["save.inputs.filename_prefix"]
  },
  "reference_chain": {
    "load": { "class_type": "LoadImage", "inputs": { "image": "" } },
    "encode": { "class_type": "VAEEncode", "inputs": { "pixels": ["load", 0], "vae": ["vae", 0] } },
    "link": { "class_type": "ReferenceLatent", "inputs": { "conditioning": null, "latent": ["encode", 0] } },
    "head": "guidance",
    "tail": "guider.inputs.conditioning",
    "max": 10
  }
}
```

`reference_chain` 是本规格新增的一种可变槽位形态：**链式**（每张参考插一段
三节点、`conditioning` 串起来），区别于 AUTOGROW 的**平铺编号**。两者都由
`studio-core` 里的确定性代码展开，Agent 看不见。

---

## 4. 执行时机：**先生成，再上确认门**

`StageKind::Hybrid` 现在的文档串写的是「Agent 定内容，**确认后**由控制面执行
生成」。**这条要改成「控制面执行生成，**然后**在门上给人看产物」。**

理由：门叫 `visual_assets.approval`，人要确认的是**卡片长得对不对**。
按原顺序，人是在批准一份自己没见过的 JSON。

先例就在旁边：`preview` 是先执行、再在门上让人看 480p 预览。视觉资产是同一个
形状——花时间产出便宜的东西，让人看过再往下花贵的。

所以 Hybrid 的语义定为：

```
Agent submit_stage(asset_plan)
  → 控制面执行（生成卡片，回填 path / status / provenance）
  → 确认门（人看图）
  → 下一阶段
```

对代码的影响：

- `StageKind` 的文档串改掉
- `Project::next_action` 里 Hybrid 不再一律 `WaitingOn::Agent`：**已提交但还没
  执行**的 Hybrid 阶段是 `WaitingOn::System`，跟 Deterministic 一样起 worker
- `retry_stage` 放开给 Hybrid——生成卡片会失败（ComfyUI 挂了、显存不够），
  失败之后必须能原样重跑，这跟 render 是同一类需求

---

## 5. schema 的唯一实质改动：`derived_from` 单值 → 列表

累积锁定要挂「主视图 + 已定稿的 1..N-1」，单值表达不了。

```jsonc
"derived_from": ["C01.front_full", "C01.three_quarter"]
```

规矩：

- 主视图（`is_anchor: true`）不填，且**必须是这张卡里第一个**
- 非主视图必填，**第一项必须是主视图**
- 只能指向**同一张卡里已经排在前面**的视图（顺序即生成顺序）
- 最多 10 项（FLUX.2 的上限）

新增校验 V10–V12（在 `check_prompt_pack` 的兄弟位置，`asset_plan` 提交时就报）：

| # | 规则 |
|---|---|
| V10 | 每张卡有且仅有一个 `is_anchor`，且它排在第一位 |
| V11 | 非主视图的 `derived_from` 非空，首项是主视图，其余项都在本卡且排在自己前面 |
| V12 | `derived_from` 不超过 10 项 |

**Agent 不必自己算累积链**——doctrine 会说「填到目前为止已定稿的全部视图」，
但 V11 只要求「指向前面的视图」，Agent 少填几个不算错，只是锁得弱一点。

---

## 6. 执行器

```rust
fn visual_assets(&self, ctx: &ExecContext<'_>) -> Result<Outputs> {
    // 卡与卡之间互不依赖 → 可以并发；卡内视图必须串行（后面的要拿前面的当参考）
    for card in plan["assets"] {           // 按 comfy_concurrency 并发
        for view in card["views"] {        // 串行，顺序即 derived_from 的依赖顺序
            let graph = if view.is_anchor { t2i } else { multiref_edit(refs) };
            提交 → 等 → 下载到 media/assets/<asset_id>/<view>.png
            回填 status: ready / path / provenance
        }
    }
}
```

- **落盘位置**：`media/assets/<asset_id>/<view>.png`，正是 schema 里 `path`
  字段的注释写的形状
- **provenance**：`{ backend, workflow, width, height, seed, refs }`，可审计
- **失败**：那一个视图 `status: failed` 并附原因，整卡继续；全卡失败才让阶段红
  ——一个视图没出来不该让前面几十秒白跑
- **画幅**：`aspect` 字段解析成宽高，短边按原生画布走（`768`），
  复用 SPEC-0015 的 `delivery_dims` 思路但常量不同

---

## 7. 验收标准

### 7.1 不需要 GPU

- V10–V12 的单元测试，每条都要有 remedy
- `derived_from` 列表形态的 schema 往返
- 两份 flux2 基线 `parse` 得出、`check()` 过、`role: card` 因此不进能力面
- 参考链展开是确定性的：同一份声明展开两次逐字节相同
- Hybrid 阶段在「已提交未执行」时 `waiting_on: system`

### 7.2 需要真机

- 一张角色卡四个视图真的生成出来，`status` 全 `ready`，文件真的在
- **换参考出的图必须不同**（AUTOGROW 那次的教训，参考链同样要守这条）
- 生成完的卡片被 `render` 引用，`AssetResolver` 解析得到、传得上去

### 7.3 报结论时

附探针结果与验到哪一层。§2.3 那组结论是人眼看过的，效应量级也如实写了。

---

## 8. 不做的事

- **不做** Z-Image 的多参考路线：`TextEncodeZImageOmni` 配的是 Z-Image Base，
  权重不在这台机器上。要比这一条得先下 `z_image_bf16`，本规格不等它。
- **不做**卡片超分：卡片喂给 R2V 时 `ref_image_size: "match"` 会缩放对齐，
  超分是白花（#13 已经把 SeedVR2 放到 `post` 了）。
- **不改** `render` 侧：`AssetResolver` 早就写好了，它一直在等 `status: ready`。
