# SeedVR2 成片超分

`upscale.json` 是 `post` 用来把每一镜超到交付规格的整图基线。

**它跟 `core_model_family` 无关。** 目录布局上跟 `minimax_h3/`、`ltx2_5/`
并列，语义上不是「一个出片的模型系列」——不管哪个系列渲的片子，超分都走
这一份。

## 权重

- diffusion: `seedvr2_7b_int8_convrot.safetensors`
- vae: `seedvr2_ema_vae_fp16.safetensors`

机器上同时还有 `seedvr2_3b_int8_convrot.safetensors`。**选 7B 是因为它几乎
不额外要钱**：一步采样，DiT 只前向一次，耗时被 tiled VAE 编解码主导，
实测 3B 39.6 s / 7B 42.1 s（同一镜 768×1344 39 帧超到 1080×1920），差 6%。

## 来源

`Comfy-Org/workflow_templates` 的
`templates/utility_seedvr2_3b_int8_upscale_video.json`，取
`definitions.subgraphs[0]`，逐条按 `links` 展平成 API 格式。

模板里有三个 `ComfySwitchNode` 和一个 `PrimitiveBoolean`，都是 UI 开关，
不是算法的一部分。按模板自带的默认值解成直连：

| 开关 | 模板默认 | 展平结果 |
|---|---|---|
| `trim_video` | `false` | 去掉 `Video Slice`，`GetVideoComponents` 直接接输入视频 |
| `split_latent` | `false` | 去掉 `SeedVR2TemporalChunk` / `TemporalMerge`，整段进一个 latent |

> **`SeedVR2TemporalChunk` / `TemporalMerge` 是显存不够时的分块逃生路径**，
> 不是默认路径。逐镜超分（39–73 帧）一个 latent 装得下，所以我们走的就是
> 官方默认的那条。真要处理长片再把它接回来——`frames_per_chunk` 落在 `4n+1`
> 网格上（1, 5, 9, 13…，默认 21）。

另一份社区模板 `utility_seedvr2_video_upscale.json` 走的是自定义节点包
（`SeedVR2LoadDiTModel` / `SeedVR2VideoUpscaler`），**目标机器上没有**，
没有参考价值。

## 不许动的参数

| 节点 | 参数 |
|---|---|
| `sampler`（KSampler） | `steps=1, cfg=1, sampler_name=euler, scheduler=simple, denoise=1` |
| `encode` / `decode`（VAE*Tiled） | `tile_size=512, overlap=128, temporal_size=64, temporal_overlap=8` |
| `resize` | `scale_method=lanczos`, `resize_type.crop=center` |
| `postp` | `color_correction_method=none` |
| `unet` | `weight_dtype=default` |

可覆盖的只有 `_studio.bindings` 里那五个：输入文件名、目标宽高、seed、
输出前缀。

## `resize_type` 是动态组合框，写法有坑

`ResizeImageMaskNode.resize_type` 的类型是 `COMFY_DYNAMICCOMBO_V3`：选一个
键，那个键自带一组子输入。API 格式里它是**平铺的点号兄弟键**：

```jsonc
"resize_type": "scale dimensions",
"resize_type.width": 1080,
"resize_type.height": 1920,
"resize_type.crop": "center"
```

**另外三种想当然的写法——`{"key": …, "multiplier": …}`、嵌套对象
`{"scale by multiplier": {…}}`、二元组 `["scale by multiplier", {…}]`——
图校验全部通过，执行时抛
`TypeError: ResizeImageMaskNode.execute() missing 1 required positional
argument: 'resize_type'`。** `/prompt` 对认不出来的键是当没看见的。

唯一把真相说出来的是纯字符串写法的验证错误，它点名了
`"input_name": "resize_type.multiplier"`。

这也是为什么 `studio-core::assembly::split_target` 允许输入名本身带点。

## 为什么一步定到交付尺寸，而不是 2× 再裁

扩散跑在 `resize` **之后**的分辨率上。2× 到 1536×2688 是 1080×1920 的 2.4
倍像素，而最终还要缩回去，那 2.4 倍全是白花的：

| 目标 | 耗时（39 帧） |
|---|---|
| `scale dimensions` 1080×1920 crop=center | 39.6 s（3B）/ 42.1 s（7B） |
| `scale by multiplier` 2.0 → 1536×2688 | 112.0 s（3B） |

`crop=center` 顺手把画幅修正了：MiniMax 的「9:16」画布实际是 768×1344，
化简是 4:7；居中裁到 9:16 掉宽度的 1.6%。

## `bindings_verified: true` 的依据

2026-09-05，A800 80GB / ComfyUI 0.34.0（`https://comfy.i-yongqi.xyz`）。

**这份文件原样跑过**（填上五个绑定值后直接提交）：768×1344 / 39 帧 / 24fps
的真实 MiniMax H3 成片 → 1080×1920 / 39 帧 / 24fps / AAC 音轨，39.9 秒。

**人眼看过。** 逐帧检查没有色带、光晕或幻觉字形；跟 ffmpeg lanczos 放到
100% 并排比过（裙子系带的编织纹路、逆光发丝），SeedVR2 明显更实。
3B 与 7B 之间人眼几乎分不出。

跑通不等于画面对，所以这一条写的是「跑完出片并且人眼确认过」，不是
「图校验通过」。
