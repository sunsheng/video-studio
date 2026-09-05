# z_image：卡片生成的基线（**尚未落地**）

这个目录目前是**空的**，只有这份说明。视觉资产阶段（角色卡 / 场景卡 /
道具卡）需要两条基线，都还没有：

| 用途 | 文件 | 做什么 |
|---|---|---|
| 文生图 | `t2i.json` | 出主视图：角色的 `front_full`、场景的 `establishing`、道具的 `front` |
| 参考图生图 | `edit.json` | 出其余视图：以主视图为参考图，换机位/表情 |

## 为什么这里没有 JSON

上一级 README 第一句写着「这里放**真机跑通过**的 ComfyUI API 格式节点图」。
本仓库的开发环境**没有 GPU、没有 ComfyUI**，出不了真机导出，也就没有资格
往这里放一份看起来像模像样的节点图——那种文件的危险之处在于它能通过所有
静态检查，然后在生产机上安静地画错东西。

所以这里放的是**导出要求**。在装了 ComfyUI 的机器上按下面的清单导出，
放进来，视觉资产阶段就能跑。

## 导出清单

1. 在目标机器的 ComfyUI 里把两条流程各跑通一次，确认出图正常。
2. 用 **API 格式**导出（不是前端的 UI workflow——带 `nodes` / `links` /
   `definitions.subgraphs` 的那种不能用）。
3. 各自补一段 `_studio`，写清参数绑到哪个节点的哪个输入上。

### `t2i.json` 至少要绑

| 参数 | 说明 |
|---|---|
| `positive` | 视图提示词。`identity_prompt` 逐字 + 本视图机位描述 + 画幅 |
| `width` / `height` | 卡片尺寸。同一张卡的所有视图同一套规格 |
| `seed` | 固定并记录。卡片也要可复现——重出一个视图时要能对齐 |

`negative` 有就绑，没有就把约束写成正向句子（和视频那边同一个道理，
见 `.agents/doctrine/consistency/bible.md`）。

### `edit.json` 至少要绑

`t2i.json` 那几项，外加：

| 参数 | 说明 |
|---|---|
| `references` | **参考图**。这是这条基线存在的全部理由——非主视图靠它锚定「是同一个人」 |

参考图走 ComfyUI 的 `/upload/image` 先传上去，绑定指向承接它的
`LoadImage` 节点。多节点集群要按节点分别上传。

### `_studio` 的样子

```jsonc
{
  "_studio": {
    "bindings": {
      "positive":   ["<节点id>.inputs.<字段>"],
      "width":      ["<节点id>.inputs.width"],
      "height":     ["<节点id>.inputs.height"],
      "seed":       ["<节点id>.inputs.<字段>"],
      "references": ["<节点id>.inputs.image"]
    },
    "source": "<从哪台机器、哪份流程导出的>",
    "bindings_verified": true
  }
}
```

`bindings_verified` 只有在**真的按这份绑定跑出过正确的图**之后才写
`true`。没验证过就写 `false` 并补 `unavailable_reason`——控制面会跳过
未核验的基线，不会拿它去画图。

## 落地之前，这个阶段能做到哪一步

Agent 可以完整提交一份资产计划：卡片、视图清单、身份锁、参考关系
都会被校验（视图缺失、多个主视图、`derived_from` 没指向锚点、
同卡混画幅、身份锁近义改写，都会被挡下）。

**但图生不出来。** 计划里 `status` 一律是 `planned`，`path` 与
`provenance` 空着。这不是缺陷，是这一步的真实状态——
把计划写对，是这台没有 GPU 的机器上能做完的全部。
