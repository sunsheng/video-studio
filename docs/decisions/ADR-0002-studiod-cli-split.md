# ADR-0002：studiod 只做 serve，其余进 studio-cli

## 背景

issue #4 指出 `studiod` 把 MCP server 核心和一堆 CLI 工具（`emit-assets`、
`e2e report`、`exec report`、`workflows check`）混在一个二进制里，
`main.rs` 已经 490 行。第一次评估给的是保守方案：`studiod` 保留
`init`/`doctor`/`pack`/`unpack`，只挪走开发/CI 用的报告类命令。

复查之后发现这个保守方案不够——真正的风险不是代码整洁度，是**能力面**。
只要 `studiod` 这个二进制上挂着任何子命令，就存在"Agent 拿到这个二进制、
在沙箱里直接执行子命令绕过 MCP"的路径。CLAUDE.md 开头引用的历史事故正是
这个模式：前身项目的 `cli.py conversation start` 被 Agent 在卡住时直接拿
来用，绕过了整个协议层。子命令列表怎么剪裁都不能消除这个路径，只有物理上
不存在子命令才能。

## 决策

**`studiod` 只有一个行为：`serve`。没有子命令，没有参数。**

```bash
studiod        # 唯一用法。读 cwd 找作品目录，读 current_exe() 找程序目录，
               # 跟现在的 cmd_serve() 完全一样，不新增任何 flag。
```

`init`/`doctor`/`pack`/`unpack`/`list`/`emit-assets`/`e2e report`/
`exec report`/`workflows check` 全部移到新二进制 **`studio-cli`**（人类操作
+ 开发者工具，随生产环境一起分发，`studiod` 不再是唯一二进制）。

### 为什么不加 `--bundle <path>`

最初考虑给 `studiod` 加一个 `--bundle` 参数，显式指定作品目录，替代隐式的
cwd 发现。查证后放弃：

- `codex mcp add --help` 和 `.codex/config.toml` 的 `mcp_servers` schema
  里没有 `cwd` 字段（只有 `command`/`args`/`env`/`url`/`oauth*`），Codex
  没有官方机制向 stdio MCP server 子进程显式传工作目录，子进程的 cwd 只能
  继承 Codex CLI 自己的 cwd。
- Skill（`SKILL.md`）是静态文档，不是运行时传参通道——MCP server 子进程
  的拉起由 Codex 应用层在会话开始时根据 `.codex/config.toml` 静态配置完
  成，跟 Skill 内容无关，Skill 没有办法介入这个时机。
- 现有的 `cwd()` 自动发现已经够用，而且比硬编码绝对路径更健壮：整个 bundle
  目录被 `mv` 之后，cwd 方式不需要做任何事；硬编码 `--bundle` 绝对路径的
  方式则需要额外跑一次 `doctor --fix`。

结论：**不加参数**。`studiod` 是真正意义上的零配置——`main.rs` 里甚至不需
要 `clap::Parser` 之外的任何字段，`--help`/`--version` 是 derive 送的，
其余一律拒绝。

## Codex／Agent 的可见性隔离

`studiod` 被 Codex 拉起走的是 `.codex/config.toml` 里 `mcp_servers` 的自动
加载，这是允许的路径。`studio-cli` **不出现在 Agent 能看到的任何地方**：

- AGENTS.md / SKILL.md（`studio-cli emit-assets` 生成、Agent 会读到的文档）
  完全不提 `studiod` 或 `studio-cli` 这两个名字，也不提任何命令行语法。
- 现状审计：`crates/studiod/src/assets.rs` 里生成的 AGENTS.md 文案目前有
  三处泄露：
  - `想归档或发给别人：studiod pack。`
  - `不要试图直接执行 studiod 的子命令来推进阶段——它根本没有那种子命令。`
  - `让用户新建一个目录（studiod init）。`

  第一、三处要改写成不点名具体命令的引导语（"这超出你的能力范围，提醒
  用户自己在终端处理，不要代劳"）；第二处直接删掉——`studiod` 物理上没有
  子命令之后，不需要再警告一个不存在的东西。

## 二进制划分

```
studiod       唯一职能：serve。无子命令、无参数。
              Codex 通过 MCP 协议自动拉起，Agent 不可见其命令行。

studio-cli    人类操作 + 开发者工具二进制：
                init / doctor(--fix) / pack / unpack / list   ← 终端用户日常操作
                emit-assets / e2e report / exec report /      ← 开发 / CI / 分析
                workflows check
```

## 这消掉了什么 / 改了什么

| 旧机制 | 现在 |
|---|---|
| `studiod <子命令>` 一个二进制承担全部职责 | `studiod`（serve）+ `studio-cli`（其余全部） |
| `.codex/config.toml` 里 `args = ["serve"]` | `args` 整个字段去掉，`command` 直接指向 `studiod` |
| AGENTS.md 提及 `studiod pack` / `studiod init` | 不点名任何二进制或命令行 |
| CLAUDE.md「生产机器上只有一个二进制」 | 改写为「两个二进制：`studiod`（服务）+ `studio-cli`（工具），都没有源码」 |

## 代价 / 需要连带修改的文件

这是一次跨 crate 的重构，不是纯文档改动：

- 新建 `crates/studio-cli`：把 `assets.rs`/`doctor.rs`/`e2e.rs`/
  `exec_report.rs`/`html.rs`/`list.rs`/`pack.rs`/`rollout.rs` 从 `studiod`
  迁移过去（这些模块本来就是干净的 `pub mod`，依赖单向，迁移成本可控）。
  `codex_config()` / `bundle_files()`（`init` 用到）也跟着搬。
- `crates/studiod/src/main.rs`：从 8 个子命令的 clap 分发精简成十几行——
  开项目、起 `Server::with_executor`、`serve(stdin, stdout)`。
- `crates/studiod/Cargo.toml`：依赖大幅减少（不再需要 `zip`/`toml`/
  `walkdir`/`chrono` 这些只有 pack/assets/e2e 才用的库）；`description`
  改成"唯一职能是 serve"。
- `.github/workflows/ci.yaml`：`emit-assets --check` 那一步改成调用
  `studio-cli`。
- `.github/workflows/release.yaml`：矩阵要构建两个二进制；冒烟测试里
  `init`/`list`/`doctor`/`workflows check` 全部改成走 `studio-cli`，
  `studiod` 那部分冒烟只剩"能被拉起、能完成一次 MCP `initialize` 握手"。
- `scripts/install.sh` / `install.ps1`：安装两个二进制；`studiod init`
  改成 `studio-cli init`。
- `docs/deployment.md`：产物目录结构、安装说明同步两个二进制。
- CLAUDE.md：开篇哲学陈述、分层图、硬规则 1 都要改写（见上面「二进制划
  分」和「这消掉了什么」两节的具体文案）。

## 不在这次范围内

- `studio-cli` 内部要不要再拆（比如把纯开发/CI 用的 `emit-assets`/
  `e2e`/`exec`/`workflows-check` 和终端用户用的 `init`/`doctor`/`pack`/
  `unpack` 分成两个二进制）——本次决策是不拆，理由是这些命令都不面向
  Agent，拆分对安全边界没有增量收益，只有维护成本。
- 沙箱层面（Codex `sandbox_permissions`/PATH 隔离）阻止 Agent 执行
  `studio-cli` 的具体配置，这取决于生产环境部署方式，超出本仓库代码能
  控制的范围，留给 `docs/deployment.md` 写清楚安装建议（`studio-cli`
  不放进会被 Agent shell 命中的 PATH）。
