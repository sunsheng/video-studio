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
studiod         唯一二进制：init / serve / doctor / emit-assets / pack / unpack。
```

反向依赖一律禁止。`studio-core` 新增依赖需要在 PR 描述里说明理由。

## 硬规则

1. **二进制不提供任何变更型子命令。** 只有 `init`、`serve`、`doctor`、`emit-assets`、
   `pack`、`unpack`。绝不允许出现 `studiod submit-stage` 这类东西——
   状态变更只有 MCP 一个入口，绕过就不存在实现。
2. **Markdown 不手写。** `assets/AGENTS.md` 与各 `SKILL.md` 中涉及工具名、阶段名、
   确认门、错误码的段落由 `studiod emit-assets` 生成。CI 跑 `emit-assets --check`。
3. **每个错误都必须有 remedy。** `StudioError::remedy()` 是穷尽 match，不允许 `_ =>`。
   没有 remedy 的错误视为实现缺陷。
4. **bundle 内一律相对路径。** 数据库、`project.toml`、stages JSON 里不得出现绝对路径。
5. **不引入第二种运行时语言。** 没有 Python、没有 Node。`scripts/` 里的 shell 只做引导。

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
- **Codex 环境：** 如果本机满足 Codex 部署条件，可跑完整端到端测试

### 环境检测

- `studiod doctor` 检查本机环境（Codex、ComfyUI、ffmpeg、ffprobe 是否可用）
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

## 开发与 Codex 协同流程

### 角色分工

- **Claude Code**：写/改代码。
- **Codex（本地，按需）**：对本次提交跑 E2E + Review，只关注 P0/P1 级问题，
  P2 及以下（格式、非关键日志等）直接无视，不触发下面的循环。有没有 Codex
  跟 ffmpeg/ffprobe 一样是按需的——由 `OPENAI_API_KEY`/`OPENAI_BASE_URL`
  是否齐全决定，`studiod doctor` 判定，缺失不算环境异常，只是这一步的
  条件不满足（见上面「环境检测」）。
- **CI**：只跑 `cargo test` 和 `emit-assets --check`，不触发 Codex。

### 怎么装、怎么配本地 Codex

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
不会自动读 bundle 里 `studiod init` 生成的 `.codex/config.toml`
（那是给别的 Codex 版本/前端用的约定），要用
`codex mcp add video-studio -- <studiod 路径> serve` 全局注册，
用完 `codex mcp remove video-studio` 清掉。MCP 工具调用默认会卡在
审批——要不要绕过、用什么方式绕过，取决于当时运行 Codex 的那台机器
本身有没有更外层的沙箱防护，不要把某一次会话「这层已经沙箱化所以
绕过审批安全」的判断当成通用结论照抄到别的机器上。

### Codex E2E 的真实覆盖范围

开发环境**一定没有 GPU、没有 ComfyUI**。Codex 能端到端跑通、且真能验证到
东西的，只有 render 之前的六个阶段（idea → selection → script →
storyboard → visual_assets → prompt_pack）——走完整 MCP 协议，含门与修订。
`preview` / `render` / `post` / `review` 要真实 ComfyUI + GPU + ffmpeg，
本地 Codex 跑不出真信号，顶多验证到「提交后结构化阻塞在
`comfy_unavailable`」，**不能把这当成渲染链路已验证**。渲染链路的真实
验收只能在装了 ComfyUI 的生产机器跑 `scripts/smoke.sh`。

### 每轮提交

1. 写/改代码。
2. 本地 `cargo test` 自检，不过先修，不带着失败的单测往下走。
3. **按功能点或优先级划出有意义的独立提交**，由 Claude Code 动态判断
   哪些改动算一个完整单元——不是改一行就提交，也不是把所有改动攒成
   一个大提交。每个提交做完就 `git push`，不允许攒着多个提交再一次性推。
4. **第一个提交推送之后立刻建 PR**，不要等全部改动做完、攒着一堆提交
   才建。PR 一旦建好，后续每个提交都推到这同一个分支/PR 上，从下面的
   步骤 5 开始的流程（等 CI → 跑 Codex → 结果同步 PR → 有驳回就循环修复）
   在每一轮新提交之后都重新走一遍，而不是只在最后走一次。
5. 等 CI 跑完单元测试；CI 失败立刻在 PR 里说明、优先修复，不计入下面的
   Codex 循环次数。
6. 本机能跑 Codex 时（见「角色分工」），对本次提交执行 E2E + Review，
   只看 P0/P1。
7. 把 Codex 的结论（P0/P1 结论的文字摘要——不是截图，Claude Code 没有
   截图能力）同步进 PR 评论。无 P0/P1 驳回就通过；有驳回进入循环修复。

### 循环修复

Codex 报 P0/P1 后：改代码 → 本地单测 → commit + push（新 commit，
标注「修复轮次 N」）→ 等 CI 通过 → 再跑一次 Codex → 结果同步 PR。

**总共最多 3 次 Codex 审查**（第 6 步那次算第 1 次，循环里最多再跑 2 次）。
3 次仍未通过就停，在 PR 里写明「Codex 循环超限，需人工介入」，等人处理。
不为琐碎问题无限纠缠。

### 异常情况

- **本机跑不了 Codex**（没配 `OPENAI_API_KEY`/`OPENAI_BASE_URL`；或者按
  「怎么装、怎么配本地 Codex」那节装好配好之后 `codex doctor` 仍然报
  provider 不可达；或者这个执行环境本来就没有能装 npm 全局包、跑子
  进程的 shell）：跳过第 6、7 步，在 PR 里标注「Codex 不可用，需人工
  复核」，等人处理。这不是异常，是本来就有的两条腿之一——CI 单测该
  跑照跑。**不要只因为默认 provider 报 401 就判定不可用**——先按上面
  那节装好配好再下结论。
- **CI 单测失败**：立刻在 PR 里说明，优先修 CI，不计入 Codex 循环次数。
