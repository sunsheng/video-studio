# 已验证 workflow 基线

这里放的每一份文件都必须**能追溯到一次真机跑通的记录**，按
`<模型系列>/<用途>.json` 组织：

```
workflows/
├── minimax_h3/
│   ├── t2v.json  i2v.json  r2v.json     整图基线
│   ├── fragments/                        片段库（只有这个系列有）
│   └── SOURCE-fragments.md               片段的血缘
├── ltx2_5/…
└── wan2_2/…
```

## 两种形式，两种追溯方式

| | 整图基线 | 片段 |
|---|---|---|
| 是什么 | 一张完整的、能直接提交的 API 图 | 一张图的一部分，单独拿出来跑不通 |
| 怎么追溯 | 自带 `bindings_verified` 与真机 run_id | `_studio.from` + `source_run` 指回它被切出来的那份已验证基线 |
| 谁在用 | `ltx2_5` / `wan2_2` / `wan_animate2`，以及 `minimax_h3` 的对照 | 只有 `minimax_h3` |

**规约是「血缘可追溯」，不是「每份文件都能单独跑通」**——片段本来就跑不通。
这条在 [ADR-0005](../../docs/decisions/ADR-0005-workflow-fragments.md) 里改的，
起因是固定基线表达不了「挂几个参考由内容决定」。验证精神没丢，只是粒度从
「整张图」变成「片段 + 组合规则」。

片段化**只对 `minimax_h3` 做**，因为只有它需要可变槽位（9 图 + 3 视频 +
3 音频的参考、可链式的 AddGuide）。其余系列保持整图基线，**两种形式共存是
有意的**，不是过渡态。

提示词包里的 `workflow` 字段写的就是这里的相对路径（不含 `.json`），
例如 `minimax_h3/t2v`。找不到对应文件时控制面报 `model_contract_violation`
并停下——**不会自动换成别的系列或别的节点图**。

## 格式

必须是 **API 格式**：顶层每个键是节点 id，值带 `class_type` 与 `inputs`。
从 ComfyUI 前端导出的 UI workflow（带 `nodes` / `links` / `definitions.subgraphs`）
不能直接用，需要在当前 ComfyUI 版本里导出 API 图。

除节点之外，基线要带一段 `_studio.bindings`，说明逐镜头参数写到哪个节点的哪个输入上。
提交前这段会被剥掉，不会发给 ComfyUI。

```jsonc
{
  "_studio": {
    "bindings": {
      "positive":      ["6.inputs.text"],
      "negative":      ["7.inputs.text"],
      "width":         ["5.inputs.width"],
      "height":        ["5.inputs.height"],
      "length_frames": ["5.inputs.length"],
      "seed":          ["3.inputs.seed"]
    }
  },
  "3": { "class_type": "KSampler",        "inputs": { "seed": 0, "steps": 20, "cfg": 7.0 } },
  "5": { "class_type": "EmptyLatentVideo","inputs": { "width": 1080, "height": 1920, "length": 42 } },
  "6": { "class_type": "CLIPTextEncode",  "inputs": { "text": "" } },
  "7": { "class_type": "CLIPTextEncode",  "inputs": { "text": "" } }
}
```

一个参数可以绑到多个路径（例如尺寸同时要写进两个节点）。基线里没绑定的参数
会被忽略——不同系列支持的参数本来就不同。绑定指向不存在的节点则是硬错误，
说明这份基线自己坏了。

`FORMAT-EXAMPLE.json` 是上面这段的可运行副本，用来对照格式；
**它不是可用的基线**，不要放进模型系列目录。

## 当前状态

跑 `studio-cli workflows check` 看每份基线的状态。带 `unavailable_reason` 的
被标为**不可用**，控制面拒绝用它们渲染。待核验的清单和卡点见
[docs/TODO.md](../../docs/TODO.md)。

## 加一个新系列

1. 在目标 ComfyUI 上真机跑通，导出 API 图
2. 存成 `<系列>/<用途>.json`
3. 补上 `_studio.bindings`
4. 固定模型组合登记进 `config/models.toml`

不需要改任何代码。
