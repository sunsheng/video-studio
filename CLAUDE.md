# video-studio 开发契约

本仓库用 Claude Code 开发，产物在 Codex 上运行。**开发环境和运行环境彻底分开**：
生产机器上只有两个二进制（`studiod` 服务 + `studio-cli` 工具）加若干
Markdown/配置，永远没有源码。

## 分层（由 crate 依赖强制）

```
studio-core     领域层。阶段图、typestate 状态机、错误枚举、契约类型、schema 校验。
                ★ 零 I/O 依赖：不得依赖 rusqlite / ureq / std::fs 之外的任何 I/O。
                  状态机逻辑必须能在没有 GPU、没有数据库的机器上纯单元测试跑完。
studio-store    SQLite 持久化。依赖 core。主要被 engine 依赖；studio-cli 的
                `list` 也直接读它做跨作品扫描（只读，不经 MCP）。
studio-engine   阶段循环、确认门、恢复、产物登记。依赖 core + store。
studio-comfy    ComfyUI HTTP 客户端。★ 本机不需要 GPU，一切经 HTTP。
studio-media    ffmpeg / ffprobe 外部进程编排。
studio-mcp      MCP 协议层：工具注册表、schema、决策信封。依赖 core + engine。
studio-pipeline 三个确定性阶段（渲染、后期、验收）的实现：向 ComfyUI 提交、
                用 ffmpeg 拼接、用 ffprobe 核对。依赖 core + engine + comfy
                + media。被 studiod 与 studio-cli 依赖。
studiod         MCP server 二进制。唯一职能 serve，没有子命令、不接受参数。
                由 Codex 自动拉起，Agent 不可见其命令行。依赖 mcp + pipeline。
studio-skill-eval Skill 评估：像测代码一样测 AGENTS.md / SKILL.md。依赖
                core + engine + mcp。只被 studio-cli 依赖，见 ADR-0003。
studio-cli      人类操作 + 开发者工具二进制：init / doctor / pack / unpack /
                list / emit-assets / e2e report / exec report /
                workflows check / skill-eval。不出现在 Codex/Agent 的执行
                环境里。
```

反向依赖一律禁止。`studio-core` 新增依赖需要在 PR 描述里说明理由。

## 硬规则

1. **`studiod` 没有子命令，不接受任何参数。** 唯一行为是 serve。绝不允许出现
   `studiod submit-stage` 这类东西——状态变更只有 MCP 一个入口，子命令列表
   怎么裁都消不掉「Agent 拿到二进制直接绕过 MCP」这条路径，只有物理上不
   存在子命令才行。项目管理（`init`/`doctor`/`pack`/`unpack`/`list`）和
   开发者工具（`emit-assets`/`e2e report`/`exec report`/`workflows check`/
   `skill-eval`）都在 `studio-cli` 里，且 `studio-cli` **不出现在 Codex/Agent 的执行环境
   里**——AGENTS.md / SKILL.md 不提这两个二进制的名字或命令行语法，见
   `docs/decisions/ADR-0002`。
2. **Markdown 不手写。** `assets/AGENTS.md` 与各 `SKILL.md` 中涉及工具名、阶段名、
   确认门、错误码的段落由 `studio-cli emit-assets` 生成。CI 跑 `emit-assets --check`。
3. **每个错误都必须有 remedy。** `StudioError::remedy()` 是穷尽 match，不允许 `_ =>`。
   没有 remedy 的错误视为实现缺陷。
4. **bundle 内一律相对路径。** 数据库、`project.toml`、stages JSON 里不得出现绝对路径。
5. **不引入第二种运行时语言。** 没有 Python、没有 Node。`scripts/` 里的 shell 只做引导。

## 标准工作流程

每次改动走完整流程，不在中间步骤停下：

1. **改代码/文档 → 本机验证**：`cargo fmt --all -- --check`、
   `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`；
   碰到随包文档（AGENTS.md / SKILL.md / JSON Schema）再加
   `cargo run -q -p studio-cli -- emit-assets --out assets --check`。
2. **commit → push** 到指定分支。提交怎么切分由 Claude Code 自行判断——
   按功能点或逻辑完整性划出有意义的单元，不强制拆到多小，也不强制每个
   commit 单独推；一批相关改动可以合成一个提交再推。
3. **create PR**。在阶段性成果完成、值得给人看时创建，不强制第一个提交
   推送后就立刻建。PR 建好之后，后续每个提交都推到这同一个分支/PR 上，
   从下面第 4 步开始的流程在每一轮新提交之后都重新走一遍，不是只在最后
   走一次。
4. **wait CI**：订阅该 PR 的活动（`subscribe_pr_activity`），不要创建完就结束。
5. **CI 红** → 定位失败、修复、push，回到第 4 步循环，直到绿。
6. **CI 绿** → 判断这次改动有没有碰到 MCP 工具面、阶段图，或任何 Agent
   可观察的行为：
   - **没碰到**（纯文档、纯内部重构、`cargo test` 范围内的改动）：跳过，
     并说明跳过的理由——不是默认不做。
   - **碰到了** → 看本机 Codex 环境是否可用（`codex doctor` 通过，见
     「本地配置 Codex」）：
     - **可用**：跑一轮 render 之前的阶段任务验收（idea → prompt_pack，
       见 `docs/e2e.md` 的「端到端验收」）+ Codex Review，只关注 P0/P1
       级问题（P2 及以下如格式、非关键日志直接无视）。确认 Agent 真的
       按协议走、没有绕过 MCP。
       - 有 P0/P1 驳回 → 改代码 → 本机验证 → commit + push（标注
         「修复轮次 N」）→ 回到第 4 步循环。**总共最多 3 次 Codex
         审查**（这次算第 1 次，循环里最多再跑 2 次）；3 次仍未通过就
         停，在 PR 里写明「Codex 循环超限，需人工介入」，等人处理，
         不为琐碎问题无限纠缠。
       - 无 P0/P1 驳回 → 通过，把结论的文字摘要（不是截图，没有截图
         能力）同步进 PR 评论。
     - **不可用**（没配 `OPENAI_API_KEY`/`OPENAI_BASE_URL`；配好之后
       `codex doctor` 仍报 provider 不可达；或者这个执行环境本来就没有
       能装 npm 全局包、跑子进程的 shell）：跳过这一步，在 PR 里标注
       「Codex 不可用，需人工复核」。这不是异常，是本来就有的两条腿
       之一——CI 单测该跑照跑。**不要只因为默认 provider 报 401 就判定
       不可用**——先按「本地配置 Codex」装好配好再下结论。
7. **进入下一个任务。**

CI 绿不等于任务完成：第 4-6 步是流程本身，不是可选的收尾动作。

### 角色分工

- **Claude Code**：写/改代码。
- **Codex（本地，按需）**：按上面第 6 步的条件跑 E2E + Review，只关注
  P0/P1。
- **CI**：只跑 `cargo test` 和 `emit-assets --check`，不触发 Codex。

### 本地配置 Codex

装二进制，全局装、不要每次 `npx` 现拉：

```bash
npm install -g @openai/codex
```

配 provider：内置的 `openai` provider 会硬连 `api.openai.com`，不认
`OPENAI_BASE_URL`——直接跑会 401，不代表 Codex 真的不可用。写
`~/.codex/config.toml`，加一个自定义 provider 指向 `$OPENAI_BASE_URL`
并设为默认（不要用 `[projects.*]` 段已有内容覆盖，追加进去）：

```toml
model_provider = "envproxy"

[model_providers.envproxy]
name = "envproxy"
base_url = "<$OPENAI_BASE_URL 的值>"
env_key = "OPENAI_API_KEY"
wire_api = "responses"   # 这个版本不认 "chat" 了
```

配好之后 `codex` / `codex exec` / `codex review` / `codex doctor`
都不用再带 `-c` 覆盖参数，直接跑。`codex doctor` 显示
`reachability mode: provider auth` 且对应 provider 的 endpoint
`reachable` 就算装配成功；这是判断「本机能不能跑 Codex」的标准，
不要只看默认 provider 报 401 就下结论。

冒烟测 MCP 工具（比如让 Codex 调 `studio.status`）：这个 Codex 版本
不会自动读 bundle 里 `studio-cli init` 生成的 `.codex/config.toml`
（那是给别的 Codex 版本/前端用的约定），要用
`codex mcp add video-studio -- <studiod 路径>` 全局注册——`studiod`
没有子命令，不用带 `serve`。用完 `codex mcp remove video-studio` 清掉。
MCP 工具调用默认会卡在审批——要不要绕过、用什么方式绕过，取决于当时
运行 Codex 的那台机器本身有没有更外层的沙箱防护，不要把某一次会话
「这层已经沙箱化所以绕过审批安全」的判断当成通用结论照抄到别的机器上。

### Codex 验收的真实覆盖范围

开发环境**一定没有 GPU、没有 ComfyUI**。Codex 能端到端跑通、且真能验证到
东西的，只有 render 之前的六个阶段（idea → selection → script →
storyboard → visual_assets → prompt_pack）——走完整 MCP 协议，含门与修订。
`preview` / `render` / `post` / `review` 要真实 ComfyUI + GPU + ffmpeg，
本地 Codex 跑不出真信号，顶多验证到「提交后结构化阻塞在
`comfy_unavailable`」，**不能把这当成渲染链路已验证**。渲染链路的真实
验收只能在装了 ComfyUI 的生产机器跑 `scripts/smoke.sh`。

## 工具链

`rust-toolchain.toml` 指定具体版本（不是 `stable`）。升级时改该文件，然后验证：
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**推代码前必须在本地跑一遍这两条**——CI 两条都查，只跑 clippy 不跑 fmt
会漏掉格式问题，等 CI 红了才发现。

## 测试边界

### 本机必能通过（无环境依赖）

- `cargo test` —— 单元测试、状态机、schema 校验、MCP 一致性

### 本机有条件通过（需要外部依赖）

- **ffmpeg / ffprobe：** 如果本机装有，可运行 `studio-media` 的集成测试
- **ComfyUI + GPU：** 如果本机装有，可通过 HTTP 运行 `studio-comfy` 的集成测试
- **Codex 环境（不含 render）：** 如果本机装有 Codex CLI 且已配置好可用的
  model provider（`codex doctor` 通过），可以用真实 Codex 会话走 render
  之前的六个阶段。装配方法和真实覆盖范围见「标准工作流程」下的
  「本地配置 Codex」「Codex 验收的真实覆盖范围」。

### 环境检测

- `studio-cli doctor` 检查 ComfyUI、ffmpeg、ffprobe 是否可用；在作品目录里
  运行时，还检查该作品 `.codex/config.toml` 指向的 `studiod` 路径是否仍然
  有效。**它不检测本机是否装有 Codex CLI 本身**——那用 `codex doctor` 查。
- 根据检测结果，选择性运行相应的集成测试
- **不得声称集成通过而不说明环境前置条件**
- 环境变量里如果同时有 `OPENAI_API_KEY` 和 `OPENAI_BASE_URL`，就据此配置 Codex
  用于本机测试；缺一个都不算满足 Codex 部署条件，按未装处理
- **CI 中不运行 Codex 端到端测试。** 不管上述两个环境变量是否存在，CI 只跑
  本机必能通过的 `cargo test` 和 `emit-assets --check`；Codex 端到端测试只在
  本机手动按需触发，不接入 CI 流水线

### 真实验收

- 生产环境集成验收在宿主机跑 `scripts/smoke.sh`
- 不得用 mock 通过来宣称链路跑通
