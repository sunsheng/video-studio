# SPEC-0015 成片超分：逐镜超到交付规格

| | |
|---|---|
| 对应 issue | [#13](https://github.com/sunsheng/video-studio/issues/13) |
| 状态 | 设计中 |
| 前置 | ComfyUI 上要有 SeedVR2 原生节点与权重（本机已探到，见 §2） |
| 顺带修 | 两个真机上才暴露出来的既有缺陷，见 §7 |

---

## 1. 目标与非目标

### 目标

`render` 出的每一镜是 MiniMax H3 的原生画布（短边 768），交付规格是
1080×1920。把中间这一段补上：**在 `post` 里逐镜做时序超分，然后再拼接**。

### 非目标

- **不做**卡片超分。#13 正文已经说清楚为什么：卡片是喂给 R2V 的参考图，
  `ref_image_size: "match"` 会把它缩回去，超分白花时间。
- **不改**阶段图，不加 MCP 工具，不动 Agent 可观察的任何东西。超分是
  `post` 内部多出来的一步，提示词包与确认门一个字都不变。
- **不改** `render` 的画布规则。Agent 仍然按原生画布写宽高，doctrine 里
  「要更高分辨率走后期的超分」那句话从今天起是真的了。

---

## 2. 已验证的事实

**下面每一条都是 2026-09-05 在真机（A800 80GB，ComfyUI 0.34.0，
`https://comfy.i-yongqi.xyz`）上跑出来的，不是推断。**

### 2.1 节点与权重

原生 SeedVR2 节点齐全：`SeedVR2Preprocess`、`SeedVR2Conditioning`、
`SeedVR2TemporalChunk`、`SeedVR2TemporalMerge`、`SeedVR2PostProcessing`。
权重：`seedvr2_3b_int8_convrot.safetensors`、
`seedvr2_7b_int8_convrot.safetensors`（`diffusion_models`），
`seedvr2_ema_vae_fp16.safetensors`（`vae`）。

> 另一份社区模板 `utility_seedvr2_video_upscale.json` 用的是自定义节点包
> （`SeedVR2LoadDiTModel` / `SeedVR2VideoUpscaler`），**这台机器上没有**。
> 基线抄的是原生那份。

### 2.2 它是时序超分，不是逐帧

整段视频编码成**一个带时间维的 latent**，一次采样。
`SeedVR2TemporalChunk` / `TemporalMerge` 是**显存不够时的分块逃生路径**，
官方模板里由 `split_latent` 开关控制，**默认关**。

这回答了 #13 的第一个问题：不会有帧间闪烁，因为根本不是逐帧跑的。

### 2.3 接线（逐字抄自官方模板）

来源：`Comfy-Org/workflow_templates` 的
`templates/utility_seedvr2_3b_int8_upscale_video.json`，
取 `definitions.subgraphs[0]`。模板里三个 `ComfySwitchNode` 按其默认值
（`trim_video=false`、`split_latent=false`）解成直连——这两个开关都是 UI
便利，不是算法的一部分。

```
LoadVideo ──→ GetVideoComponents ─┬─ images ─→ ResizeImageMaskNode(lanczos)
                                  ├─ audio ──────────────────────┐
                                  ├─ fps ────────────────────────┤
                                  └─ bit_depth ──────────────────┤
                                                                 │
ResizeImageMaskNode ─┬─→ SeedVR2Preprocess → VAEEncodeTiled ─┬─→ SeedVR2Conditioning
                     │                                       └─→ KSampler.latent_image
                     └────────────────→ SeedVR2PostProcessing.original_resized_images
UNETLoader ─┬─→ SeedVR2Conditioning.model
            └─→ KSampler.model
SeedVR2Conditioning ─┬─ positive ─→ KSampler.positive
                     └─ negative ─→ KSampler.negative
KSampler → VAEDecodeTiled → SeedVR2PostProcessing → CreateVideo → SaveVideo
```

固定参数（模板原值，不许自己改）：

| 节点 | 参数 |
|---|---|
| `KSampler` | `steps=1, cfg=1, sampler_name=euler, scheduler=simple, denoise=1` |
| `VAEEncodeTiled` / `VAEDecodeTiled` | `tile_size=512, overlap=128, temporal_size=64, temporal_overlap=8` |
| `ResizeImageMaskNode` | `scale_method=lanczos` |
| `SeedVR2PostProcessing` | `color_correction_method=none` |
| `UNETLoader` | `weight_dtype=default` |

### 2.4 `COMFY_DYNAMICCOMBO_V3` 在 API 图里怎么写

`ResizeImageMaskNode.resize_type` 是动态组合框：选一个键，那个键自带一组
子输入。API 格式是**平铺的点号兄弟键**：

```jsonc
"inputs": {
  "resize_type": "scale dimensions",
  "resize_type.width": 1080,
  "resize_type.height": 1920,
  "resize_type.crop": "center"
}
```

**另外三种写法（`{"key":…}`、嵌套对象、二元组）图校验全过、执行时抛
`TypeError: execute() missing 1 required positional argument`。**
又一例「校验通过 ≠ 能跑」——`/prompt` 只认识不了的键当没看见。
唯一一条把真相说出来的是第一种写法的验证错误：
`"input_name": "resize_type.multiplier"`。

这条对 `Workflow::write_at` 有影响，见 §5.2。

### 2.5 一步到位到 1080×1920，不走「2× 再裁」

#13 正文的方案是 2× 到 1536×2688 再裁到 1080×1920。实测这是亏的：

| 目标 | 耗时（39 帧） | 输出 |
|---|---|---|
| `scale dimensions` 1080×1920 crop=center | **39.6 s** | 1080×1920 |
| `scale by multiplier` 2.0 | 112.0 s | 1536×2688 |

扩散是在 resize **之后**的分辨率上跑的，2× 意味着 2.4 倍像素。既然最终
要缩回 1080×1920，那 2.4 倍像素全是白花的。直接把 `ResizeImageMaskNode`
定到交付尺寸，一步到位，还省掉一次 ffmpeg 缩放。

`crop=center` 顺手解决画幅：MiniMax 的 9:16 画布实际是 768×1344，化简是
**4:7**（0.5714），不是 9:16（0.5625）。从 4:7 居中裁到 9:16 掉宽度的
1.6%，可以忽略。

### 2.6 耗时与选型

同一镜 768×1344 / 39 帧 / 24fps：

| 步骤 | 耗时 | 占渲染 |
|---|---|---|
| MiniMax H3 渲染 | 63.9 s | 100% |
| SeedVR2 **3B** → 1080×1920（暖机） | 39.6 s | 62% |
| SeedVR2 **7B** → 1080×1920（暖机） | 42.1 s | 66% |

**7B 只贵 6%**：一步采样，DiT 只前向一次，耗时被 tiled VAE 编解码主导。
所以选 7B。

画质：3B / 7B 都明显强过 ffmpeg lanczos（裙子系带的编织纹路、逆光发丝），
两者之间人眼几乎分不出。**这条是人眼看过的**，不是靠指标。

---

## 3. 形状：一份整图基线，不是片段

超分链路没有可变槽位——不随镜头内容改变结构，只改四个数（文件名、宽、高、
seed）。所以它是 **`Workflow` 整图基线**，走 `_studio.bindings` 那条老路，
不进 ADR-0005 的片段库。

```
assets/workflows/seedvr2/upscale.json
assets/workflows/seedvr2/SOURCE-README.md
```

`_studio` 元数据：

```jsonc
{
  "bindings": {
    "filename":      ["load.inputs.file"],
    "width":         ["resize.inputs.resize_type.width"],
    "height":        ["resize.inputs.resize_type.height"],
    "seed":          ["sampler.inputs.seed"],
    "output_prefix": ["save.inputs.filename_prefix"]
  },
  "source": "Comfy-Org/workflow_templates utility_seedvr2_3b_int8_upscale_video.json（子图展平，7B 权重）",
  "bindings_verified": true
}
```

`bindings_verified: true` 的依据：2026-09-05 真机跑通并**人眼看过对比图**
（见 §2.6），不只是图校验过。

> `seedvr2` 是个独立的「系列」目录，跟 `core_model_family` 无关——任何系列
> 渲出来的片子都用它超分。目录布局上跟 `ltx2_5/` 这些并列，语义上不同。
> 这一点写进 `SOURCE-README.md`。

---

## 4. 放在哪：`post` 的第一步，逐镜

```
render 的 N 个镜头 ──逐镜超分──→ media/upscaled/<shot_id>.mp4 ──拼接──→ media/final.mp4
```

**为什么逐镜而不是整片**（这条不需要问人，四个理由都是硬的）：

1. 显存跟成片长度解耦。整片 10 秒 × 30fps = 300 帧进一个 latent，多半要开
   `TemporalChunk`；逐镜是 39–73 帧，一个 latent 装得下，走的是官方默认路径。
2. 复用现有的按并发度分发与逐镜重试。
3. 时序模型跨硬切没有意义——两镜之间是切换，不是运动。
4. 超分后各镜参数仍然一致，`post` 的 `can_stream_copy` 照样成立，拼接仍是
   直接复制流。

**为什么不放进 `render`**：`render` 的产物是「模型出的片」，超分是交付规格。
混在一起的话，重跑 `post` 换个交付尺寸就得重渲一遍。

---

## 5. 实现

### 5.1 交付尺寸怎么算

```rust
/// 交付短边。短视频平台的通行规格。
pub const DELIVERY_SHORT_EDGE: i64 = 1080;

/// 按 brief 声明的画幅算交付尺寸。
/// 解析不出 `W:H` 就退回素材自己的宽高比——那样至少分辨率是对的。
fn delivery_dims(aspect: &str, src_w: i64, src_h: i64) -> (i64, i64)
```

规则：

- 短边取 `max(DELIVERY_SHORT_EDGE, 源短边)`——**只放大不缩小**。
- 长边按解析出的比例算，两边都向上取偶数（H.264 要偶数）。
- 算出来跟源尺寸完全相同就跳过这一镜，在 notes 里记一笔。

### 5.2 `Workflow::write_at` 要认点号输入名

现在的实现把 `<节点>.inputs.<输入>` 之后还有内容判为「层级过深」。
而 §2.4 说了，ComfyUI 的动态组合框的输入名**本身带点**
（`resize_type.width`）。改成把剩下的部分 join 回去当输入名，只在为空时报错。

`studio-core::assembly` 里的同名逻辑一并对齐——一条规则两处实现是这个项目
反复栽过的坑。

### 5.3 `post` 的新步骤

```rust
fn post(&self, ctx) -> Result<Outputs> {
    let shots = render["shots"];
    let parts = if ctx.settings.comfy_upscale() {
        self.upscale_shots(ctx, shots)?     // 逐镜，按 comfy_concurrency 并发
    } else {
        shots.map(|s| s["path"])            // 今天的行为
    };
    // 以下不变：can_stream_copy → concat → cover → subtitles → probe
}
```

留痕：每镜一条 `ctx.step("upscale").shot(id)`，带上 `from`/`to` 尺寸与
`prompt_id`；跳过的镜头记 `skipped` 与原因。`exec report` 因此能直接看出
超分占了整条流水线多少时间。

`post` 的产物里增加：

```jsonc
{ "upscaled": true, "delivery": "1080x1920" }
```

### 5.4 开关

`ComfyConfig.upscale: bool`，默认 `true`；环境变量 `COMFY_UPSCALE=0` 关掉。
与 `preview_turbo` 完全对称。

**关掉时不是静默降级**：`post` 的产物里 `upscaled: false`，进度里说一句
「按配置跳过超分，成片是原生画布」。

**开着但 ComfyUI 不可达时报结构化阻塞，不降级。** remedy 里给两条路：
修好 ComfyUI，或者 `COMFY_UPSCALE=0` 明确接受原生画布。这跟本项目对
「探不到就阻塞」的一贯处理一致。

---

## 6. 对 `review` 的影响

`review` 的画幅检查拿 ffprobe 实测值跟 `brief.aspect_ratio` 做**字符串
相等**比较。今天渲染出 768×1344，化简是 `4:7`，跟 `9:16` 不等——
**这条检查现在就是红的**。超分到 1080×1920 之后它变绿，是顺带修好的，
不是本规格新引入的行为。

关掉超分的作品这条仍然是红的。那是事实，不该掩盖：交付规格没达到，验收就
不该说过。

---

## 7. 顺带修的两个既有缺陷

### 7.1 `collect_files` 不过滤 `type`（会拿错文件）

`LoadVideo` 会把**输入文件**回显进 history 的 `outputs`：

```jsonc
"load": { "images": [{ "filename": "anchor.mp4", "subfolder": "", "type": "input" }] },
"save_video": { "images": [{ "filename": "sh01_00001_.mp4", "subfolder": "", "type": "output" }] }
```

`collect_files`（`studio-comfy/src/lib.rs`）按 `images/gifs/videos/audio/files`
收，**不看 `type`**；节点 id 排序下 `guide1_src_load` / `ref1_load` 都排在
`save_video` 前面，而 `generate_shot_once` 取的是 `files.first()`。
**结果是：带 `clip` 锚点或 `kind: video` 参考的镜头，渲染结果会指向锚点素材
而不是成片。**

`real_comfy.rs` 里那条 video 通道的测试只断言了 `!files.is_empty()`，没下载，
所以没抓到。本规格的超分图同样带 `LoadVideo`，不修就会原样中招。

修法：`collect_files` 只收 `type == "output"` 的项。缺 `type` 的按 `output`
算（`default_type()` 已经是这个语义），老测试不受影响。

同时把 `real_comfy.rs` 那条测试补成「下载下来核对帧数」——只断言「有产物」
的测试挡不住这一类错误。

### 7.2 `Workflow::write_at` 拒绝点号输入名

见 §5.2。不修的话 SeedVR2 基线的 `width` / `height` 绑定根本写不进去。

---

## 8. 验收标准

### 8.1 不需要 GPU

- `delivery_dims` 的单元测试：9:16 / 16:9 / 1:1 / 解析不出 / 源已超过 1080
- `Workflow` 能把值写进点号输入名，层级仍然只允许一层节点
- `collect_files` 丢掉 `type: "input"` 的项，保留缺 `type` 的项
- SeedVR2 基线 `check()` 通过（每条绑定指向的节点都存在）且 `is_verified()`
- `COMFY_UPSCALE=0` 时 `post` 的行为与今天逐字节一致

### 8.2 需要真机（`real_comfy.rs`，`COMFY_NODE` 没配就跳过并打印理由）

- 一镜 768×1344 超到 1080×1920：出片、尺寸对、帧数不变、音轨还在
- 超分后的多镜仍然 `can_stream_copy`
- 带 `clip` 锚点的镜头下载下来的是**成片**，不是锚点（7.1 的回归）

### 8.3 报结论时必须一起说的

探针结果（型号、显存、权重清单）、验到哪一层（图校验 / 跑完出片 /
人眼看过画面）。§2.6 那组结论是人眼看过的。

---

## 9. 不需要 ADR

没有新的架构决策：整图基线这条路 ADR 早就定了，分层没变（`studio-comfy`
出 HTTP，`studio-pipeline` 编排），阶段图没动。
