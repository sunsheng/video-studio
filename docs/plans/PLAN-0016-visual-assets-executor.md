# PLAN-0016 视觉资产执行器的执行计划

对应 [SPEC-0016](../specs/SPEC-0016-visual-assets-executor.md) / [#12](https://github.com/sunsheng/video-studio/issues/12)。

每步做完仓库都是绿的（`cargo fmt --check` + `clippy -D warnings` +
`cargo test --workspace` + `emit-assets --check`）。

---

## S1 两份 FLUX.2 基线

1. `assets/workflows/flux2_dev/t2i.json`——从 `image_flux2` 模板展平，
   摘掉参考分支与 turbo 开关，VAE 换成机器上装着的那份。
2. `assets/workflows/flux2_dev/multiref_edit.json`——同上，保留参考分支，
   `_studio.reference_chain` 描述可变的参考链。
3. `SOURCE-README.md`：来源、两处必须偏离模板的理由、真机 run、选型依据。
4. 两份都标 `"role": "card"`，不进 Agent 能力面。

> 验：新增单测——两份都 `parse` 得出、`check()` 过、`verified_names()` 里没有
> `flux2_dev/`；真机各跑一次原样提交。

**提交点 1**：`feat(assets): FLUX.2 dev 卡片基线`

---

## S2 参考链展开

`reference_chain` 的确定性展开：给 N 个参考文件名，按模板节点复制 N 段
`LoadImage → VAEEncode → ReferenceLatent`，`conditioning` 从 `head` 串到 `tail`。

跟 AUTOGROW 的平铺编号并列，是第二种可变槽位形态。node id 规则沿用
`ref{n}_<原名>`，确定性由测试钉住（展开两次逐字节相同）。

> **落点改了**（实现时才看清）：SPEC §3.1 说放 `studio-core`，但
> `reference_chain` 是**整图基线**的元数据，而 `Workflow` 住在
> `studio-pipeline`。放 `studio-core` 就得把 `Workflow` 也搬过去，
> 或者在两个 crate 之间拆一半。所以展开写在 `studio-pipeline::workflow`，
> 只复用 `studio_core::assembly::split_target` 那一条路径规则。
> 片段库那边的 AUTOGROW 展开仍在 `studio-core`——两种形态本来就属于
> 两种基线形状。

**提交点 2**：`feat(pipeline): 参考链槽位展开`

---

## S3 `derived_from` 改列表 + V10–V12

1. schema：`derived_from` 从 `text` 改成字符串数组。
2. 校验 V10（有且仅有一个主视图且排第一）、V11（非主视图指向本卡前面的视图、
   首项是主视图）、V12（≤10）。每条带 remedy。
3. `fixtures` 与金样跟着改。

> 这一步碰 Agent 契约，`emit-assets` 要重跑。

**提交点 3**：`feat(core): derived_from 支持累积锁定`

---

## S4 Hybrid 阶段真的会执行

1. `StageKind::Hybrid` 的文档串改成「控制面执行，然后在门上给人看产物」。
2. `Project::next_action`：已提交未执行的 Hybrid → `WaitingOn::System` + 起 worker。
3. `retry_stage` 放开给 Hybrid。
4. 门的时机：执行完才挂门。

> 验：`lifecycle.rs` 补一条——提交 `asset_plan` 之后 `waiting_on` 是 `system`，
> 执行完才轮到确认门。

**提交点 4**：`feat(engine): Hybrid 阶段先执行再上门`

---

## S5 执行器本体

`Pipeline::visual_assets`：按 SPEC §6。卡间并发、卡内串行，逐视图落盘回填。

**提交点 5**：`feat(pipeline): 视觉资产执行器`

---

## S6 真机验收 + 文档

1. `real_comfy.rs`：一张卡四视图真的出来；**换参考出的图必须不同**。
2. doctrine：`consistency/character-sheet.md` 按累积锁定改写视图表。
3. `docs/TODO.md`、`docs/e2e.md` 跟上。

**提交点 6**：`test+docs: 卡片链路的真机验收`

---

## 之后

push → PR → `subscribe_pr_activity` → CI 绿 → **这次碰了 Agent 可观察的东西**
（`derived_from` 的形状、`visual_assets` 的 `waiting_on` 时序），
所以要跑 Codex 端到端 + Review，按 CLAUDE.md 第 6 步走。
