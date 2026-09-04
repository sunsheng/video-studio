# video-studio 开发契约

本仓库用 Claude Code 开发，产物在 Codex 上运行。**开发环境和运行环境彻底分开**：
生产机器上只有一个二进制加若干 Markdown/配置，永远没有源码。

## 分层（由 crate 依赖强制）

```
studio-core     领域层。阶段图、typestate 状态机、错误枚举、契约类型、schema 校验。
                ★ 零 I/O 依赖：不得依赖 rusqlite / ureq / std::fs 之外的任何 I/O。
                  状态机逻辑必须能在没有 GPU、没有数据库的机器上纯单元测试跑完。
studio-store    SQLite 持久化。只被 engine 依赖。
studio-engine   阶段循环、确认门、恢复、产物登记。依赖 core + store。
studio-comfy    ComfyUI HTTP 客户端。★ 本机不需要 GPU，一切经 HTTP。
studio-media    ffmpeg / ffprobe 外部进程编排。
studio-mcp      MCP 协议层：工具注册表、schema、决策信封。依赖 engine。
studiod         唯一二进制：init / serve / doctor / emit-assets / pack / unpack /
                list / e2e report / exec report / workflows check。
```

反向依赖一律禁止。`studio-core` 新增依赖需要在 PR 描述里说明理由。

## 硬规则

1. **二进制不提供任何变更型子命令。** 只有 `init`、`serve`、`doctor`、`emit-assets`、
   `pack`、`unpack`，以及只读的 `list`、`e2e report`、`exec report`、
   `workflows check`。绝不允许出现 `studiod submit-stage` 这类东西——
   状态变更只有 MCP 一个入口，绕过就不存在实现。
2. **Markdown 不手写。** `assets/AGENTS.md` 与各 `SKILL.md` 中涉及工具名、阶段名、
   确认门、错误码的段落由 `studiod emit-assets` 生成。CI 跑 `emit-assets --check`。
3. **每个错误都必须有 remedy。** `StudioError::remedy()` 是穷尽 match，不允许 `_ =>`。
   没有 remedy 的错误视为实现缺陷。
4. **bundle 内一律相对路径。** 数据库、`project.toml`、stages JSON 里不得出现绝对路径。
5. **不引入第二种运行时语言。** 没有 Python、没有 Node。`scripts/` 里的 shell 只做引导。

## 标准工作流程

每次改动走完整流程，不在中间步骤停下：

1. **改代码/文档 → 本机验证**：`cargo fmt --all -- --check`、
   `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`；
   碰到随包文档（AGENTS.md / SKILL.md / JSON Schema）再加
   `emit-assets --out assets --check`。
2. **commit → push** 到指定分支。
3. **create PR**。
4. **wait CI**：订阅该 PR 的活动（`subscribe_pr_activity`），不要创建完就结束。
5. **CI 红** → 定位失败、修复、push，回到第 4 步循环，直到绿。
6. **CI 绿** → 判断这次改动有没有碰到 MCP 工具面、阶段图，或任何 Agent
   可观察的行为：
   - 碰到了，且本机 Codex 环境可用（`codex doctor` 通过）：跑一轮
     render 之前的阶段任务验收（见 `docs/e2e.md` 的「端到端验收」），确认
     Agent 真的按协议走、没有绕过 MCP。
   - 没碰到（纯文档、纯内部重构、`cargo test` 范围内的改动）：跳过，
     并说明跳过的理由——不是默认不做。
7. **进入下一个任务。**

CI 绿不等于任务完成：第 4-6 步是流程本身，不是可选的收尾动作。

## 工具链

`rust-toolchain.toml` 指定具体版本（不是 `stable`）。升级时改该文件，然后运行：
```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## 测试边界

### 本机必能通过（无环境依赖）

- `cargo test` —— 单元测试、状态机、schema 校验、MCP 一致性

### 本机有条件通过（需要外部依赖）

- **ffmpeg / ffprobe：** 如果本机装有，可运行 `studio-media` 的集成测试
- **ComfyUI + GPU：** 如果本机装有，可通过 HTTP 运行 `studio-comfy` 的集成测试
- **Codex 环境（不含 render）：** 如果本机装有 Codex CLI 且已配置好可用的
  model provider（`codex doctor` 通过、`codex exec` 能正常应答），可以用
  真实 Codex 会话驱动 `studiod serve` 走 render 之前的六个阶段
  （idea → prompt_pack），验证 Agent 是否正确使用工具面。这不是「完整端到端」：
  render 及之后仍然需要真实 ComfyUI + GPU，只能在生产环境跑。

### 环境检测

- `studiod doctor` 检查 ComfyUI、ffmpeg、ffprobe 是否可用；在作品目录里运行时，
  还检查该作品 `.codex/config.toml` 指向的程序路径是否仍然有效。
  **它不检测本机是否装有 Codex CLI 本身**——那用 `codex doctor` 查。
- 根据检测结果，选择性运行相应的集成测试
- **不得声称集成通过而不说明环境前置条件**

### 真实验收

- 生产环境集成验收在宿主机跑 `scripts/smoke.sh`
- 不得用 mock 通过来宣称链路跑通
