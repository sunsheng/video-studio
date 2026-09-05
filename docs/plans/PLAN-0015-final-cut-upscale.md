# PLAN-0015 成片超分的执行计划

对应 [SPEC-0015](../specs/SPEC-0015-final-cut-upscale.md) / [#13](https://github.com/sunsheng/video-studio/issues/13)。

每一步做完仓库都是绿的（`cargo fmt --check` + `cargo clippy -D warnings` +
`cargo test --workspace`）。切提交的位置在每步末尾标出来。

---

## S1 先修那两个既有缺陷

超分链路会原样踩到 7.1，所以先修，且**先补上能抓住它的测试**。

1. `studio-comfy::collect_files` 只收 `type == "output"`。补两条单元测试：
   - `type: "input"` 的回显被丢掉
   - 缺 `type` 的仍然收（`default_type()` 的语义不变）
2. `Workflow::write_at` 认点号输入名（剩余部分 join 回去当输入名，为空才报错）。
   补测试：`a.inputs.resize_type.width` 写得进去，`a.b` 仍然报错。
3. `studio-core::assembly` 里的同名路径逻辑对齐——**一条规则两处实现**是这个
   项目栽过三次的坑，不留第二份。
4. `real_comfy.rs` 里 video 通道那条测试补成「下载下来核对帧数」。

> 验：`cargo test -p studio-comfy -p studio-core -p studio-pipeline`。
> 真机那条要 `COMFY_NODE` 才跑，本机有，顺手跑一遍。

**提交点 1**：`fix(comfy): 产物只认 type=output 的文件`（含 workflow 路径与测试）

---

## S2 放基线

1. `assets/workflows/seedvr2/upscale.json`——从官方模板展平出来的 API 图，
   `UNETLoader` 写 7B，`_studio.bindings` 按 SPEC §3。
2. `assets/workflows/seedvr2/SOURCE-README.md`——来源、展平时怎么解的三个
   `ComfySwitchNode`、`bindings_verified: true` 的依据（真机 run + 人眼看过）、
   §2.4 那条点号输入名的坑。
3. `studio-cli doctor` 的 `check_workflow_assets` 把 `seedvr2/upscale.json`
   纳入检查（缺了要报，不能等到 `post` 才炸）。

> 验：新增单元测试——基线 `parse` 得出来、`check()` 过、`is_verified()` 真。

**提交点 2**：`feat(assets): SeedVR2 超分基线`

---

## S3 交付尺寸与配置开关

1. `studio-pipeline`：`DELIVERY_SHORT_EDGE` 常量 + `delivery_dims()`，
   按 SPEC §5.1。单元测试覆盖 9:16 / 16:9 / 1:1 / 解析不出 / 源已超过 1080 /
   奇数取偶。
2. `studio-engine::ComfyConfig.upscale`（默认 true）+ `comfy_upscale()`
   读 `COMFY_UPSCALE`。测试跟 `preview_turbo` 那条对称。

**提交点 3**：`feat(pipeline): 交付尺寸推导与超分开关`

---

## S4 `post` 接上超分

1. `upscale_shots()`：按 `comfy_concurrency()` 起 worker，逐镜
   加载基线 → `apply` 四个参数 → 提交 → 等 → 下载到
   `media/upscaled/<shot_id>.mp4`。失败重试沿用 `MAX_SHOT_ATTEMPTS`。
2. 留痕：每镜一条 `step("upscale")`，带 `from` / `to` / `prompt_id`；
   跳过的记 `skipped` 和原因。
3. ComfyUI 不可达 → 结构化阻塞，remedy 给两条路（修好，或 `COMFY_UPSCALE=0`）。
4. `post` 产物加 `upscaled` / `delivery`。
5. `COMFY_UPSCALE=0` 的路径与今天逐字节一致——补一条测试钉住。

> 验：`cargo test --workspace`；`emit-assets --check`（如果碰到了随包文档）。

**提交点 4**：`feat(pipeline): post 逐镜超分到交付规格`

---

## S5 真机验收

`real_comfy.rs` 加一组（`COMFY_NODE` 没配就跳过并打印理由）：

- 一镜 768×1344 → 1080×1920：尺寸对、帧数不变、音轨还在
- 超分后的多镜仍然 `can_stream_copy`
- 带 `clip` 锚点的镜头下载到的是成片（S1 的回归）

跑完**人眼看一遍产出**——跑通证明不了画面是对的。

**提交点 5**：`test(pipeline): 超分链路的真机验收`

---

## S6 文档

- `docs/e2e.md`：真机那一节补上超分这组测试验的是什么
- `CLAUDE.md`：把 §2.4 那条「动态组合框的 API 形态」记进「图校验通过 ≠
  画面是对的」旁边——它是同一类失败（校验过、执行炸）的又一个实例
- `assembly/shots.md` 的画幅段落：确认「要更高分辨率走后期的超分」现在是
  真的，必要时补一句交付尺寸由 `post` 负责

**提交点 6**：`docs: 记录超分与动态组合框的 API 形态`

---

## 之后走标准流程

push → 建 PR → `subscribe_pr_activity` → CI 绿 →
这次碰没碰 MCP 工具面 / 阶段图 / Agent 可观察行为？**没碰**（阶段图没动，
工具面没动，提示词包形状没动），所以按 CLAUDE.md 第 6 步跳过 Codex 端到端，
并在 PR 里写明跳过的理由。
