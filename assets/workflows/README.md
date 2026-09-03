# 已验证 workflow 基线

这里放**真机跑通过**的 ComfyUI API 格式节点图，按 `<模型系列>/<用途>.json` 组织：

```
workflows/
├── minimax_h3/
│   ├── t2v.json
│   ├── i2v.json
│   └── r2v.json
├── ltx2_5/…
└── wan2_2/…
```

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

## 加一个新系列

1. 在目标 ComfyUI 上真机跑通，导出 API 图
2. 存成 `<系列>/<用途>.json`
3. 补上 `_studio.bindings`
4. 固定模型组合登记进 `config/models.toml`

不需要改任何代码。
