# PLAN-0014 动态组装的执行计划

规格见 [SPEC-0014](../specs/SPEC-0014-dynamic-workflow-assembly.md)。
这份只回答**按什么顺序做、每步怎么验、在哪里切提交**。

## 原则

1. **每一步都能独立验证**，不留「写完一大坨再一起测」的段落。
2. **先立契约再改实现**：schema 和组装器（纯数据、可单测）先行，
   碰 ComfyUI 的部分排在后面。
3. **随时可停**：任何一步做完，仓库都是绿的、可提交的状态。

---

## 步骤

### S0 ADR：验证粒度从整张图变成片段 + 组合规则

**做什么**：写 `docs/decisions/ADR-0005-workflow-fragments.md`。

内容：为什么不让 LLM 生成节点图（#14 三条理由 + SPEC §2.4 那个
`decode_audio` 差点接错的实例）；片段血缘怎么保证；两种形式共存的边界；
`assets/workflows/README.md` 那条规约从「单独跑通」改成「血缘可追溯」。

**验证**：无代码。ADR 索引更新。
**提交**：`docs(ADR-0005): 工作流验证粒度改为片段 + 组合规则`

---

### S1 片段库落盘

**做什么**：按 SPEC §3 从现有三份已验证基线切出 9 份片段，写
`SOURCE-fragments.md` 记录每份的来源节点与 run_id 血缘。改
`assets/workflows/README.md` 那条规约。

**关键**：接线**逐字从基线抄**，不按接口类型推断（SPEC §2.4）。

**验证**：
- 新增测试：每份片段的 `_studio` 元数据完整（`kind`/`id`/`from`/`source_run`/
  `bindings_verified`）
- 新增测试：片段声明的 `outputs`/`inputs` 端口指向的节点在片段内真实存在
- `cargo test`

**提交**：`feat(workflows): 从已验证基线切出 minimax_h3 的片段库`

---

### S2 组装器（纯数据，零 I/O）

**做什么**：`studio-core/src/assembly.rs`。输入是声明 + 片段库描述，
输出是组装计划（node id → class_type + inputs）。实现 SPEC §5.2 的七步。

**这一步不碰文件系统、不碰网络**，片段库以 trait 注入，跟 `capability.rs`
一个套路。

**验证**：
- 三种典型镜头的组装结果快照测试：1 秒空镜 / 接续镜 / 群戏
- **确定性测试**：同一份声明组装两次，输出逐字节相同
- AddGuide 链式接线的测试：`latent` 全接 head、`positive` 串链、
  `guider` 接链尾
- AUTOGROW 序号从 1 递增、`with_audio` 占同号槽位
- `cargo test`

**提交**：`feat(core): 声明到节点图的确定性组装器`

---

### S3 校验规则 V1–V8

**做什么**：SPEC §6 那八条，全部在 `studio-core`，每条带 remedy。

**验证**：每条一个失败用例 + 断言 remedy 非空。V4（`17k+5` 帧网格）
额外加一个「现有 fixtures 是否合规」的检查——不合规就把 fixtures 一起修。

**提交**：`feat(core): prompt_pack 的组合合法性校验`

---

### S4 prompt_pack schema 按系列分派

**做什么**：`assets/schema/prompt_pack.json` 的 shot 形状按 SPEC §4 重做。
`minimax_h3` 用 `head` + `references` + `guides`；其它系列保留 `workflow`。
schema 按 `core_model_family` 动态收窄（沿用现有能力面校验的做法）。

**连带**：`capability.rs` 的对账数据源换成「片段库 + 组合规则」，
逻辑不变。`references` 的豁免规则收紧成必写。

**验证**：
- 两种形状各自的通过/失败用例
- 用 `minimax_h3` 的形状提交 `ltx2_5` 的镜头 → 报错且 remedy 说清楚
- `cargo test`

**提交**：`feat(core): prompt_pack 按模型系列分派两种 shot 形状`

---

### S5 pipeline 接上组装器

**做什么**：`studio-pipeline` 里实现片段加载，`generate_shot` 从
「加载基线 + apply」改成「组装 + 提交」。整图基线那条路保留给其它系列。

参考图/视频/音频的上传：调 `comfy.upload_image`（已存在），把 `asset_id`
解析成 bundle 内的实际文件再传。

**验证**：
- 用现有的本机 TCP 假节点跑通组装 → 提交 → 下载全链路
- `cargo test`

**提交**：`feat(pipeline): 渲染改走片段组装`

---

### S6 preview 的 turbo 组合

**做什么**：SPEC §5.4。preview 挂 turbo LoRA + 相应降 steps，
配置项 `comfy.preview_turbo`，**默认开**。

**验证**：单测确认 preview 模式下组装结果里有 LoRA 节点、steps 是 LoRA 的
步数；关掉配置后回到普通组合。

**提交**：`feat(pipeline): preview 默认走 turbo LoRA 组合`

---

### S7 Skill 与 doctrine

**做什么**：`prompt/SKILL.md` 从「填 workflow 字段」改成「按镜头意图选 head、
组参考、设 guide」（改 `studio-cli/src/assets.rs` 的源，不手改生成物）。
新增一份 doctrine：什么时候该接续、参考怎么组、guide 挂哪一帧。
能力卡改成片段清单 + 各自上限。

**验证**：`emit-assets --check` 一致；`studio-skill-eval` 的场景跟着更新。

**提交**：`docs(assets): prompt Skill 改为声明式，新增组装方法层`

---

### S8 真机验收

**做什么**：SPEC §8.2。

1. 三种典型镜头组装出的图提交到 ComfyUI，**只看图校验通过**
   （`node_errors` 为空），通过后立刻删队列，不烧 GPU
2. 挑一镜真跑完出片，确认画面不是坏的
3. preview 的 turbo 组合真机跑通，记录与非 turbo 的耗时对比

**工具**：复用 `scratchpad/probe_wiring.py` 的做法（提交 → 看 `node_errors`
→ 删队列）。

---

### S9 流程收尾

- CI 绿
- Codex 端到端：真实会话走 idea → prompt_pack，确认 Agent 能按新 schema
  正确声明（**这次改动碰了 MCP 工具面的 schema，按 CLAUDE.md 第 6 步必须跑**）
- Codex Review：`codex exec review --base main -m gpt-5.6-sol -c model_reasoning_effort="xhigh"`，
  只收 P0/P1
- 复盘写进 issue #14

---

## 依赖关系

```
S0 ADR ──┐
S1 片段库─┼─▶ S2 组装器 ─▶ S3 校验 ─▶ S4 schema ─▶ S5 pipeline ─▶ S6 preview
         │                                              │
         └──────────────────────────────────────────────┴─▶ S7 Skill ─▶ S8 真机 ─▶ S9 收尾
```

S2 可以在 S1 之前开始写（片段库以 trait 注入，测试用内存里的假片段），
但 S1 先做能让 S2 的测试直接用真片段，更实在。

---

## 风险与应对

| 风险 | 应对 |
|---|---|
| **按接口类型推断接线导致静默错接** | SPEC §2.4 已经踩过一次（`decode_audio` 接 `[0]` 不是 `[1]`）。S1 硬规则：逐字从基线抄；S8 用 ComfyUI 自己的图校验兜底 |
| 组装出的图能过校验但产出错画面 | S8 第 2 步必须真跑一镜看画面，不能只看校验通过 |
| `capability.rs` 返工 | 逻辑不变只换数据源，S4 一次改完；PR #16 已确认这块分层是对的 |
| V4 帧网格校验让现有 fixtures 不合规 | S3 里一并修 fixtures，不留红 |
| 其它系列被误伤 | S4 的测试专门有一条：用 minimax 的形状提交 ltx2_5 要报错 |

---

## 不在本计划内

- 卡片生成、FLUX.2 接入（#12，依赖本计划落地）
- 成片超分（#13，权重已就位，独立）
- e2e report 漏报修订往返（#17，独立）
