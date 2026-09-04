# Studio Bundle 设计

> 在线版（含排版）：https://claude.ai/code/artifact/8a3fe961-bcec-4f92-9c38-183d2d4a4ade
>
> 本文是仓库内的权威版本。与在线版有出入时以本文为准——设计在实现过程中
> 有几处收敛，都记在这里。

## 1. 背景与证据

前身项目 douyin-video-studio（Python，7250 行）在 2026-09-03 的一次 Codex 会话
（`01a064ae-7583-7bf3-887d-233293c5af85`）中暴露了结构性问题。用户只说了五句话，
Agent 打了 41 次工具调用，其中 **10 分钟、18 次调用**花在一次「把每镜头 2 秒改成
智能分配时长」的修订上：

```
03:21:18  studio_revise_stage → {"status": "ready_for_redo"}
03:22:00  studio_submit_stage → -32602: task already claimed: stage.script.v1
03:25:14  python -c 'state_store.cancel_pending_questions(...)'
03:29:21  python -c 'con.execute("UPDATE questions SET status=\"pending\" ...")'
```

根因三条，没有一条是「模型不听话」：

1. **工具面缺口。** `revise_stage` 标了 `ready_for_redo` 却不释放任务锁，
   而没有任何工具能释放它——「用户要求改稿」这条最高频路径必然死锁。
2. **错误是死路。** 报错不含补救路径、不含锁持有者与过期时间；
   `studio_status` 的 `blocked_by` 字段全程为 `null`，明明是为此设计的。
3. **文档指路去读源码。** `AGENTS.md` 写着「工具清单的唯一事实源是
   `mcp_server.py` 的 TOOLS」「确认门的事实源是 `stage_graph.py`」，
   并明文授权「一次性小脚本」。Agent 是照做的。

另有一个独立现象：每次提交阶段产物前，Agent 都先去读**另一个不相干项目**的
`output.json`——因为 `submit_stage` 的参数声明是 `outputs: {[key]: unknown}`，
完全不约束，它唯一能确认「什么形状会被接受」的办法就是找一份被接受过的。

**设计目标：把上面每一条都变成结构上不可能，而不是写一条规则去禁止它。**

## 2. 核心决策

| # | 决策 | 消掉了什么 |
|---|---|---|
| D1 | 一个文件夹 = 一部作品（bundle 即文档） | run_id、run 注册表、跨 run 抄产物 |
| D2 | 全新项目，不在旧项目上修补 | 7250 行 Python 的历史包袱 |
| D3 | Rust 单二进制，无解释器、无 import 面 | `from studio import state_store` 这条绕过路径 |
| D4 | stdio MCP，一个项目一个进程 | 跨会话任务锁、锁过期、残留 lock 文件 |
| D5 | 11 个工具，全部无 `run_id` | 调用方需要知道 run 内部标识 |
| D6 | typestate 状态机 | 「revise 了但没释放锁」在编译期不可表达 |
| D7 | Markdown 由 Rust 生成 | 文档引用不存在的工具名；文档指向源码 |
| D8 | 状态变更只能从 MCP 进 | 直接跑 CLI 建 run 这条绕过路径 |

D3 后来在 [ADR-0002](decisions/ADR-0002-studiod-cli-split.md) 中演变为**两个**
二进制——`studiod`（只做 serve）与 `studio-cli`（其余全部）——但精神不变：
两者都是无解释器、无 import 面的 Rust 单体，Agent 能接触到的仍然只有前者，
且前者物理上没有变更型子命令。

## 3. 三个平面

| 平面 | 是谁 | 对外协议 |
|---|---|---|
| Agent 面 | Codex 会话 | MCP (stdio) |
| 控制面 | `studiod`（没有子命令，被拉起就是 serve） | HTTP → ComfyUI；exec → ffmpeg |
| 推理面 | ComfyUI 容器 | `/prompt`、`/history` |

**运行控制面的机器不需要 GPU。** 见 [architecture.md](architecture.md)。

## 4. 项目模型

见 [architecture.md](architecture.md) 与 [deployment.md](deployment.md)。要点：

- 工作格式是 bundle 目录，交换格式是单文件 `.dvs`（`studio-cli pack`）
- bundle 内一律相对路径，唯一指向外部的是 `.codex/config.toml` 里 `studiod`
  的程序路径，换机器用 `studio-cli doctor --fix` 修正
- 状态不外溢：不写 `~/.config`、不写全局注册表

## 5. MCP 工具面

十一个工具，没有一个带 `run_id`。见 [tool-surface.md](tool-surface.md)。

`blocked_by.remedy` 是硬要求：任何阻塞都必须给出下一步能调的工具。
`StudioError::remedy()` 是穷尽 match，测试强制每条 remedy 非空且指向一个
可调用的工具——写这两个测试时当场抓出 `project_busy` 与
`model_contract_violation` 两条只讲道理不给出路的补救说明。

## 6. 阶段图与确认门

见 [state-machine.md](state-machine.md)。十个阶段、六道门，
门在阶段**产出之后**暂停，`prompt_pack` 那道是花 GPU 时间前的最后一关，
`preview` 那道是花 GPU 时间**之后**（480p 便宜预览）、正式渲染之前的确认关卡——
唯一一个由控制面自动执行、但自己也带确认门的阶段。

### 收敛点：确认门选项自带 outcome

实现过程中新增。选项要自己声明 `outcome`：`approve` 通过，`revise` 打回草稿。

前身项目的门里混着 `approve_script` 和 `revise_script`，两者都只能走同一个
`answer` 接口——选中「修改」反而把阶段标成了通过。现在控制面只认 `outcome`，
不靠 id 的字面意思猜，且「一个通过选项都没有」的门会被 schema 校验挡下。

## 7. 状态机与撤销

typestate 把状态编码进类型参数，转换消耗自身。`Stage<AwaitingConfirmation>`
上**没有** `submit` 方法，`revise` 消耗旧值返回 `Stage<Draft>`。

两个 `compile_fail` doctest 钉住了验收标准：把旧实现那个 bug 写成 Rust
必须编译不过。

### 收敛点：revise 与 undo 的分工

讨论后定成编辑器的模型：

- **`revise(stage, message)` 是「改」。** 作品的进度整体退回到那个阶段，
  它之后的阶段一律变回未执行——分镜是照旧剧本做的，剧本一改它就不再成立。
  旧产物文件留着，可以 `stage_output` 读出来参考，重新提交时覆盖。
- **`undo()` 是「反悔」。** 每个改变状态的操作（提交、确认、修订）在动手前
  压一份快照，`undo` 弹出栈顶整个恢复。连着按就一步步往回走。

典型场景：剧本已通过、分镜已通过 → 要求改剧本，其后全部退回未执行 →
提交新剧本并确认，走到分镜 → 觉得不如原来那版，连按三次 undo →
旧剧本回来，分镜恢复已通过，下一步是视觉资产。

栈深上限 50。**这不是版本管理**：没有命名版本、没有历史列表。
要留版本请 `cp -r` 或 `studio-cli pack`。

## 8. 错误契约

17 个错误码，见 [tool-surface.md](tool-surface.md) 与生成的 `assets/AGENTS.md`。

新模型让 5 个旧错误码不再可能：`run_not_found`（当前目录就是项目）、
`task_claim_conflict` 与 `stale_lock`（单进程单项目，flock 随进程释放）、
`orphan_directory`（目录即事实源）、`retrospective_missing`（无归档流程）。

## 9. 文档生成

`AGENTS.md`、10 份 `SKILL.md`、10 份 JSON Schema 全部由 `studio-cli emit-assets`
从阶段图、工具注册表和错误码枚举生成，CI 跑 `--check` 守着。

一个测试专门守着「AGENTS.md 不得出现源码路径」——写这条时当场抓出生成模板里
残留的一句 `crates/studiod/src/assets.rs`。

## 10. 安全边界

三条硬规则：

1. **`studiod` 不提供任何子命令，唯一行为是 serve。** 状态变更只有 MCP
   一个入口，子命令列表怎么裁都消不掉「Agent 拿到二进制直接绕过 MCP」这
   条路径，只有物理上不存在子命令才行。`init` / `doctor` / `pack` /
   `unpack` / `list` / `emit-assets` / `e2e report` / `exec report` /
   `workflows check` 都在另一个二进制 `studio-cli` 里，且 `studio-cli`
   不出现在 Codex/Agent 的执行环境里。见 [ADR-0002](decisions/ADR-0002-studiod-cli-split.md)。
2. **`workspace_roots` 只含当前 bundle。** 兄弟作品物理不可达。
3. **`.studio/` 三层保护。** dotdir 约定 + AGENTS.md 明确禁止 + 沙箱写限制，
   外加完整性摘要兜底：外部改动会以 `state_drift` 暴露。

**待实测**：Codex 受限 sandbox 到底限读还是限写，决定第 3 条的强度。

## 11. 资产迁移

从前身项目带走的是资产不是代码：已验证的 ComfyUI API 基线、
固定模型契约（进 `config/models.toml`，**不再写进 AGENTS.md**）、
阶段图与门位置的设计判断、结构化错误码清单、10 个 Skill 的职责边界。

前身把 30 行 safetensors 文件名塞进每个会话的上下文，而前六个阶段一次都用不上。

### 收敛点：确定性阶段怎么推进

工具面上没有 `advance`。门一通过，控制面在后台线程里把 preview → render →
post → review 一路跑完，Agent 用 `studio.status` 观察——信封里的 `note`
显示此刻在做什么（例如「3/5 sh03 提交到 http://127.0.0.1:9002」）。
`preview` 执行完不会直接放行：480p 预览生成后挂起等确认，通过才轮到
花钱的正式 `render`——这是**唯一**一个自动执行、但自己也带门的阶段。

执行失败不会默默卡住：错误记进作品状态，`status` 把它变成带 remedy 的
`blocked_by`，并且**不会闷头重试**——等 Agent 用 `studio.retry_stage`
（执行失败，内容没问题）或 `studio.revise`（内容要改）处理之后再来。
孤立的一次轮询连接超时也不会立即判死——只有连续失败或总耗时超过
timeout 才真正报错；每个节点固定绑一个 worker 线程并发生成，谁先跑完
自己那镜就去认领下一镜。

## 12. 路线与验收

| | 范围 | 状态 |
|---|---|---|
| M0 | init / serve + idea → selection → script，含门与修订 | 完成 |
| M1 | storyboard → visual_assets → prompt_pack | 完成 |
| M2 | render（ComfyUI 接入、节点池、基线参数注入） | 完成 |
| M3 | post → review → export | 完成 |
| M4 | pack / unpack、doctor | 完成 |
| M5 | 渲染管线改进：轮询容错、节点并发、exclude_node/retry_stage、preview 阶段 | 完成 |

### 验收：重放那次会话

不是抽象的「测试通过」，而是把 2026-09-03 那次翻车原样重演：提交每镜头 2 秒的
剧本 → 用户说「不要固定 2 秒」→ 重新提交智能时长版 → 确认。

**通过标准**：全程零次 shell、零次绕行；修订往返是**一次** `revise` 加
**一次** `submit_stage`（旧实现在这里花了 18 次调用）；任何阻塞时
`blocked_by.remedy` 非空。

开发环境的对应用例见 `crates/studio-engine/tests/lifecycle.rs` 与
`crates/studio-mcp/tests/protocol.rs`；生产环境的端到端见 [e2e.md](e2e.md)。
