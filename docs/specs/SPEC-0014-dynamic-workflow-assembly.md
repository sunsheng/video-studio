# SPEC-0014 渲染工作流按镜头动态组装

| | |
|---|---|
| 对应 issue | [#14](https://github.com/sunsheng/video-studio/issues/14) |
| 状态 | 规格已定，待实现 |
| 前置 | 无（不需要 GPU；真机验证部分已完成，见 §2） |
| 阻塞谁 | [#12](https://github.com/sunsheng/video-studio/issues/12) 的参考绑定、AddGuide、累积锁定全部依赖本规格落地 |

---

## 1. 目标与非目标

### 目标

把渲染的 workflow 从「三份固定的完整节点图」换成「**片段库 + 结构化声明 +
确定性组装器**」，使每一镜的节点图结构可以随内容变化，同时不引入静默错接
的可能。

具体要能表达：

| 镜头 | 需要的图 |
|---|---|
| 1 秒空镜 | backbone + image head + 1 张图 |
| 接上一镜的对话镜 | backbone + reference head + N 个参考 + AddGuide 挂上一镜末 22 帧和音频 |
| 群戏 | backbone + reference head + 5 张角色卡 + 2 张场景卡 + 1 段锚视频 |

### 非目标

- **不做**卡片生成、FLUX.2 接入、成片超分——那是 #12 / #13。
- **不改** `wan2_2` / `ltx2_5` / `wan_animate2`。片段化**只对 `minimax_h3` 做**，
  因为只有它需要可变槽位。两种形式（整图基线 / 片段库）共存是有意的。
- **不让 LLM 生成节点图**。理由见 #14 正文三条，核心是静默错接。

---

## 2. 已验证的事实

**下面每一条都是在真机（A800 80GB，ComfyUI 0.34.0）上验过的，不是推断。**
组装器的实现必须与之一致。

### 2.1 AUTOGROW 多参考的 API 形态

`MiniMaxH3ReferenceToVideo` 的四个参考槽位类型是 `COMFY_AUTOGROW_V3`：

| 输入名 | prefix | max |
|---|---|---|
| `ref_images` | `ref_image_` | 9 |
| `ref_videos` | `ref_video_` | 3 |
| `ref_video_audios` | `ref_video_audio_` | 3 |
| `ref_audios` | `ref_audio_` | 3 |

API 格式里它是**一个对象**，键名是 `prefix + 序号`（从 1 开始）：

```jsonc
"ref_images": {
  "ref_image_1": ["load_ref1", 0],
  "ref_image_2": ["load_ref2", 0]
}
```

实测：挂 2 张图提交，图校验通过。

> `ref_videos` 的元素类型是 `IMAGE`（"Reference video frames at 24 fps, 2-15s"），
> 也就是**帧序列**，不是 VIDEO 对象。加载路径是
> `LoadVideo → GetVideoComponents → IMAGE`（`GetVideoComponents` 同时吐
> `IMAGE` 和 `AUDIO`，正好喂 `ref_videos` 与 `ref_video_audios`）。

### 2.2 AddGuide 的链式接线

`MiniMaxH3AddGuide` 签名：

```
positive: CONDITIONING  (必填)
latent:   LATENT        (必填)
frame_idx: INT          (必填，负数从末尾倒数)
vae / audio_vae: VAE    (可选)
image: IMAGE            (可选)
audio: AUDIO            (可选)
→ 输出: positive (CONDITIONING)   ← 只吐 CONDITIONING，不吐 LATENT
```

因为它不吐 LATENT，链式时 **`latent` 一律接 head 的 LATENT 输出**，只有
`positive` 串成链：

```
h3_ref ──[0] CONDITIONING──▶ guide1.positive
       └─[1] LATENT ────────▶ guide1.latent
                              guide2.latent      ← 同样接 head，不是接 guide1
       guide1 ─[0]─────────▶ guide2.positive
       guide2 ─[0]─────────▶ guider.conditioning  ← 链尾
       h3_ref ─[1]─────────▶ sampler.latent_image ← 仍是 head
```

实测：两个 AddGuide 串联 + 2 张 AUTOGROW 参考，图校验通过。

### 2.3 跟着 head 走的配套约束

| | reference head | image head |
|---|---|---|
| 节点 | `MiniMaxH3ReferenceToVideo` | `MiniMaxH3ImageToVideo` |
| `load_unet.unet_name` | `minimax_h3_ref2va_int8_convrot` | `minimax_h3_fl2va_int8_convrot` |
| `scheduler.scheduler` | **`beta`** | **`simple`** |

`beta` / `simple` 这条差异**不写在任何官方文档里**，是从两份已验证基线的
diff 里看出来的。这类约束必须写进片段元数据，不能指望谁记得。

### 2.4 一个必须记住的教训：接线不能靠类型推断

写探针时我按「AV latent 有视频和音频两路」推断
`decode_audio.samples` 应该接 `["sampler", 1]`。**错了**，真实基线接的是
`["sampler", 0]`——AV latent 是嵌套结构，`VAEDecodeAudio` 自己从里面取音频部分。

接口类型对得上（都是 LATENT），所以**类型系统挡不住这个错**。如果不是对着
已验证基线抄，这就是一次典型的静默错接。

**因此定为硬规则：片段的接线必须从已验证基线切出来，不允许按接口类型推断。**

---

## 3. 片段库

### 3.1 目录与命名

```
assets/workflows/minimax_h3/
  fragments/
    backbone.json          骨架
    head.reference.json    R2V head + 它的配套约束
    head.image.json        I2V head + 它的配套约束
    guide.image.json       AddGuide（图）
    guide.clip.json        AddGuide（视频片段）
    guide.audio.json       AddGuide（音频）
    input.image.json       LoadImage
    input.video.json       LoadVideo + GetVideoComponents
    input.audio.json       LoadAudio
  SOURCE-fragments.md      每个片段从哪份已验证基线的哪几个节点切出来
```

整图基线（`t2v.json` / `i2v.json` / `r2v.json`）**保留不动**——它们是片段的
来源与对照，也是其它系列仍在用的形式。

#### 与 `assets/workflows/README.md` 那条规约的冲突（已裁决）

那份 README 第一句写着「这里放**真机跑通过**的 ComfyUI API 格式节点图」。
片段不是完整的图，单独拿出来跑不通，跟这条直接冲突。

**裁决：规约改成「血缘可追溯」**，不是「每份文件都能单独跑通」。

即每份文件都要能追溯到一次真机跑通的记录：整图基线靠自己的
`bindings_verified` + run_id；片段靠 `from` + `source_run` 指回它被切出来的
那份已验证基线。验证精神不丢，只是粒度从「整张图」变成「片段 + 组合规则」。

README 要跟着改这一句，并说明两种形式的边界。

### 3.2 片段格式

```jsonc
{
  "_studio": {
    "kind": "head",                     // backbone | head | guide | input
    "id": "reference",
    "from": "minimax_h3/r2v.json",      // 从哪份已验证基线切来的
    "source_run": "20260831T035154Z-29b70a",  // 血缘：那份基线的真机 run_id
    "bindings_verified": true,

    // 这个片段要求 backbone 怎么配——§2.3 那些「跟着 head 走」的约束
    "backbone_overrides": {
      "load_unet.inputs.unet_name": "minimax_h3_ref2va_int8_convrot.safetensors",
      "scheduler.inputs.scheduler": "beta"
    },

    // 对外暴露的端口，组装器按名字接线，不按位置猜
    "outputs": { "conditioning": ["h3_ref", 0], "latent": ["h3_ref", 1] },
    "inputs":  { "clip": "h3_ref.inputs.clip", "vae": "h3_ref.inputs.vae",
                 "audio_vae": "h3_ref.inputs.audio_vae" },

    // 逐镜头可注入的参数
    "bindings": {
      "positive": ["h3_ref.inputs.prompt"],
      "width":    ["h3_ref.inputs.width"],
      "height":   ["h3_ref.inputs.height"],
      "length_frames": ["h3_ref.inputs.length"]
    },

    // AUTOGROW 槽位声明（§2.1）
    "autogrow": {
      "references.image": { "target": "h3_ref.inputs.ref_images",
                            "prefix": "ref_image_", "max": 9 },
      "references.video": { "target": "h3_ref.inputs.ref_videos",
                            "prefix": "ref_video_", "max": 3 },
      "references.video_audio": { "target": "h3_ref.inputs.ref_video_audios",
                                  "prefix": "ref_video_audio_", "max": 3 },
      "references.audio": { "target": "h3_ref.inputs.ref_audios",
                            "prefix": "ref_audio_", "max": 3 }
    }
  },

  "h3_ref": { "class_type": "MiniMaxH3ReferenceToVideo", "inputs": { /* ... */ } }
}
```

**`bindings_verified` 与 `source_run` 保留**——「已验证」这条规矩不丢，只是
验证粒度从「整张图」变成「片段 + 组合规则」。

---

## 4. Agent 提交的声明

### 4.1 `prompt_pack` 的 shot 形状

```jsonc
{
  "shot_id": "S03",
  "head": "reference",              // 枚举：reference | image
  "positive": "...",
  "width": 1344, "height": 768,
  "length_frames": 73, "fps": 24, "seed": 101,

  "references": [
    { "kind": "image", "asset_id": "C01.front" },
    { "kind": "image", "asset_id": "SC02.key_angle" },
    { "kind": "video", "asset_id": "C01.anchor", "with_audio": true }
  ],

  "guides": [
    { "at_frame": 0, "kind": "clip", "asset_id": "S02.tail22", "with_audio": true },
    { "at_frame": -1, "kind": "image", "asset_id": "C01.profile" }
  ]
}
```

- `references[].kind`：`image` | `video` | `audio`
- `references[].with_audio`：仅 `video` 有意义，true 时同时占用
  `ref_videos` 与 `ref_video_audios` 的**同号**槽位
- `guides[].kind`：`image` | `clip` | `audio`
- `guides[].at_frame`：负数从末尾倒数

**Agent 声明「要什么」，不声明「怎么接」。** 连线规则是模型契约，属于代码。

### 4.2 与旧 shot 的关系

旧形状里的 `workflow: "minimax_h3/t2v"` 字段**去掉**，由 `head` 取代。
其它系列（`ltx2_5` 等）仍用 `workflow` 字段走整图基线——schema 按
`core_model_family` 分派两种形状。

---

## 5. 组装器

### 5.1 位置与分层

```
studio-core/src/assembly.rs     纯数据：声明 → 组装计划，零 I/O，可完整单测
studio-pipeline                 读片段文件、把计划渲染成 API 图、提交
```

沿用 `capability.rs` 的分层做法：判断逻辑在 core，文件读取在 pipeline，
用 trait 连起来。**核心层不必知道片段文件长什么样。**

### 5.2 组装步骤

1. 按 `head` 选 head 片段，取它的 `backbone_overrides` 覆盖 backbone
2. 每个 `references[]` 生成一个 input 片段实例，按 kind 填进对应 AUTOGROW
   槽位，序号从 1 递增
3. 每个 `guides[]` 生成一个 guide 片段实例，`positive` 串成链（第一个接 head
   的 conditioning），`latent` 全部接 head 的 latent
4. `guider.conditioning` 接链尾（没有 guide 时直接接 head）
5. `sampler.latent_image` 接 head 的 latent
6. 注入 `bindings` 声明的逐镜头参数
7. 生成稳定的 node id（见 5.3）

### 5.3 node id 规则

**必须确定性**——同一份声明两次组装得到逐字节相同的图，否则
`studio.retry_stage`（「内容没问题，原样重跑」）失去意义，debug 请求也对不上。

规则：片段内的原始 id 加前缀，前缀由片段角色和序号决定：

```
backbone 的节点        原样保留（load_unet、sigmashift、sampler…）
head                   原样保留（h3_ref / h3_i2v）
第 i 个 reference 的 input   ref{i}_<原始id>
第 j 个 guide                guide{j}_<原始id>
```

### 5.4 preview 的额外收益（本规格内实现）

片段化之后 `preview` 可以换一套更便宜的组合，而不只是把分辨率缩到 480p：

- 挂 turbo LoRA（`minimax_h3_ref2v_turbo_4step` / `fl2v_turbo_8step`，两份权重
  都已在机器上）
- 相应把 `scheduler.steps` 降到 LoRA 的步数

**这一项做成可配置，默认开。** 因为 preview 门要看的只是构图与内容。

---

## 6. 校验规则

全部在 `studio-core`，提交 `prompt_pack` 时执行，每条违反都是带 remedy 的
`schema_violation`。

| # | 规则 | 依据 |
|---|---|---|
| V1 | `head: reference` 时参考上限 9 图 / 3 视频 / 3 音频 | §2.1 实测 |
| V2 | `head: image` 不接 `references`，`guides` 最多 2 个且 `at_frame` 只能是 0 或 -1 | I2V 只有 first/last |
| V3 | `guides[].at_frame ∈ [-length_frames, length_frames)` | AddGuide 语义 |
| V4 | `length_frames` 吃 `17k+5` 网格 | MiniMax H3 帧网格，**现在完全没校验** |
| V5 | `kind: clip` 的 guide 长度吃 `5 / 22 / 39 / …`（同 `17k+5`） | AddGuide 自动 snap，显式挡下更好排查 |
| V6 | `with_audio: true` 只能出现在 `kind: video` 的 reference 上 | 槽位语义 |
| V7 | 引用的 `asset_id` 必须在 `visual_assets` 的产物里存在 | 跨阶段一致性 |
| V8 | `width` / `height` 是 32 的倍数，短边 ≤ 768 | MiniMax 原生画布 |

V4 值得单独说：**现在的代码完全不校验帧数**，写个 50 帧下去模型会自己 snap，
产出的时长和提示词包里写的对不上，而 `post` 拼接是按声明的时长算的。

---

## 7. 对既有代码的影响

| 位置 | 改动 |
|---|---|
| `assets/schema/prompt_pack.json` | shot 形状按 §4.1 重做，`workflow` → `head` + `references` + `guides` |
| `studio-core/src/capability.rs` | 对账数据源从「基线 bindings」换成「片段库 + 组合规则」。**逻辑不变**（多写/少写/未核验三种照报），只换数据源、加组合维度 |
| `studio-core/src/assembly.rs` | 新增，组装器核心 |
| `studio-pipeline/src/workflow.rs` | `Workflow::apply` 保留给整图基线（其它系列仍用），新增片段加载与组装 |
| `studio-pipeline/src/lib.rs` | `generate_shot` 从「加载基线 + apply」改成「组装 + 提交」 |
| `assets/skills/prompt/SKILL.md` | 从「填 workflow 字段」改成「按镜头意图选 head、组参考、设 guide」 |
| `.agents/doctrine/` | 新增一份：什么时候该接续、参考怎么组、guide 挂哪一帧 |
| `.agents/models/minimax_h3.md` | 能力卡改成片段清单 + 各自上限 |

`references` 现在的豁免规则（「允许提前写，但基线一旦支持就必须写」）
可以收紧成必写——片段库支持它了。

---

## 8. 验收标准

### 8.1 不需要 GPU 的部分（本规格的主体）

- [ ] 三种典型镜头的组装结果有单测钉住：1 秒空镜 / 接续镜 / 群戏
- [ ] V1–V8 每条校验各有一个失败用例，且断言 remedy 非空
- [ ] 同一份声明两次组装，输出逐字节相同（确定性）
- [ ] 组装器测试不依赖 GPU、不依赖 ComfyUI
- [ ] `cargo fmt` / `clippy -D warnings` / `cargo test` / `emit-assets --check` 全过

### 8.2 需要真机的部分

- [ ] 三种典型镜头组装出的图，提交到 ComfyUI **通过图校验**（`node_errors` 为空）
      —— 这一步不必等生成完，校验过就删队列，不烧 GPU
- [ ] 至少一镜真正跑完出片，确认画面不是坏的
- [ ] preview 的 turbo LoRA 组合真机跑通，并记录与非 turbo 的耗时对比

### 8.3 Agent 侧

- [ ] 真实 Codex 会话走一遍 idea → prompt_pack，Agent 能按新 schema 正确
      声明 head / references / guides
- [ ] Codex Review 无 P0/P1

---

## 9. 需要 ADR

本规格推翻的是「已验证基线」这条既有约定（ADR-0002 之后最大的一条）：
验证粒度从「整张图」变成「片段 + 组合规则」。

ADR 要写清楚：为什么不让 LLM 生成节点图（#14 正文三条理由 + §2.4 那个
我自己差点犯的静默错接）、片段血缘怎么保证、以及两种形式共存的边界。
