# ADR-0004：Skill 评估——像代码一样测试 AGENTS.md / SKILL.md

## 背景

`assets/AGENTS.md` 与十份 `assets/skills/*/SKILL.md`（下称 "skill 文档"）是这个项目
真正的行为契约：Codex 不读源码，读的是它们。但目前对它们的验证只有三层，
没有一层能回答"Agent 读了它之后会不会正确使用工具面"：

| 现有机制 | 验证的是什么 | 缺口 |
|---|---|---|
| `studio-cli/src/assets.rs` 的 12 个 `#[test]` | 生成的文档**结构正确**：引用的工具名真实存在、必填字段被提到、确认门被提到、不泄露源码路径/二进制名 | 只查文本包含关系，不涉及任何"Agent 读完会怎么做" |
| `scripts/replay-protocol.py` | 协议层没坏：JSON-RPC 能走完六阶段 | 不经过任何 Agent，是脚本按固定顺序发调用，**验证不了文档措辞**；而且它是 Python，直接违反 CLAUDE.md 硬规则 5（不引入第二种运行时语言） |
| `docs/e2e.md` 描述的人工 Codex 会话 + `studio-cli e2e report` | **唯一**真正验证"Codex 读了 AGENTS.md/SKILL.md 之后会不会正确使用工具面"的机制 | 完全手动触发、一次性、不可重复、没有场景库（只有"重放千岛湖那次事故"一个剧本）、没有存档做前后对比——改一版 SKILL.md 措辞更好还是更差，全凭这一次跑的印象 |

这条空白正是用户要求"skill 也要像代码一样做测试、做评估、做回归"要补的：
不是要替换 `docs/e2e.md` 里"真实 Codex 会话"这个终极验收手段（生产环境、真实
ComfyUI 之前的六阶段仍然是最贴近真实的验证），而是在它旁边建一套**可重复、
可积累场景、能量化对比**的评估框架，把"这次改动是让 Agent 更好懂了还是更容易
犯前身项目那种错"从"凭印象"变成"有报告"。

## 决策

新建独立 crate **`studio-skill-eval`**，定位与 `studio-pipeline` 同层
（依赖 `core + engine + mcp + rollout`，`rollout` 是本 ADR 同批新增的
共享解析 crate，见下文），只被 `studio-cli` 依赖，新增
`studio-cli skill-eval` 子命令族。**不出现在 Codex/Agent 的执行环境里**——
和 `e2e report`/`exec report` 一样，是开发者工具，遵守 ADR-0002 的边界。

### 两类场景，边界必须分清

| | 脚本场景（scripted） | Agent 场景（agent-driven） |
|---|---|---|
| 谁做决策 | 场景脚本本身，固定的调用序列 | 一个真实 LLM，只读 skill 文档 + 工具 schema，自己决定怎么调 |
| 确定性 | 完全确定，可重放 | 不确定，同一场景两次跑可能不同 |
| 需要什么 | 什么都不需要，纯内存 | 一个可用的 LLM（Codex CLI 或直连 API） |
| 能不能进 CI | **能**——这是 `replay-protocol.py` 的直接继任者 | **不能**，跟现有"CI 不跑 Codex 端到端"的原则一致 |
| 验证的是什么 | 协议层、状态机、门逻辑没有回归 | skill 文档措辞是否引导出正确行为——这是本 ADR 真正新增的能力 |

把这两类混在一起是前身空白的根源之一：`replay-protocol.py` 想两者都不做好，
结果既不能进 CI（Python），也不能验证文档措辞（它自己决策，不读文档）。

### 组件设计

```
studio-skill-eval
├── scenario.rs   场景定义：一份 brief、期望走到哪个阶段、中途要不要注入
│                 修订/错误/门选择。脚本场景直接把"标准应答"编进场景；
│                 Agent 场景只提供"初始创意 + 用户模拟器的应答策略"，
│                 由 driver 跑出真实的调用序列。
├── harness.rs    起一个临时 bundle + 真实 studiod 子进程（stdio JSON-RPC），
│                 不用 protocol.rs 里 in-process 直调 Server 的写法——Agent
│                 场景的驱动方需要通过真正的 stdio 协议连接，跟生产环境
│                 Codex 连接的方式一致，脚本场景复用同一个 harness 图省事，
│                 顺带把「协议层没坏」也验了。
├── user_sim.rs   虚拟用户：确认门上怎么选、修订意见怎么措辞。默认走固定
│                 剧本（可重放、可回归）——选项本身从门返回的候选里按
│                 `outcome` 匹配着选，不假设固定 id（`selection` 门是真
│                 三选一，id 是 concept_id，随场景而变，跟脚本场景
│                 `harness.rs::advance()` 的取法一致，见下方合并 main
│                 之后的调整）；可选让一个 LLM 用给定"人设"生成自然语言
│                 修订意见的变体，专门用来测 skill 文档对模糊反馈的
│                 鲁棒性——这条路径本身也是不确定的，用于 Agent 场景，
│                 不用于脚本场景。
├── driver/
│   ├── mod.rs        `trait AgentDriver`：给一个 harness 和场景，跑完，
│   │                 交回调用序列 + 产物 + （如果能拿到）token/绕行信息。
│   ├── codex.rs      `codex exec` 子进程驱动：复用 `.codex/config.toml` 把
│   │                 MCP 指向 harness 起的 studiod；用 `studio-rollout`
│   │                 crate（见下方"新增 `studio-rollout` 共享 crate"）
│   │                 解析 rollout 拿 token/skills_read/doctrine_read/
│   │                 bypasses。
│   └── direct_llm.rs 直连 LLM API 驱动：把 `studio.*` 工具 schema 转成目标
│                     API 的 tool-calling 格式，系统提示词 = AGENTS.md +
│                     对应 SKILL.md 原文（不夹带任何源码或额外提示），跑一个
│                     标准 tool-use 循环直到停在预期阶段或达到调用上限。
│                     不依赖 Codex CLI 是否装配，`OPENAI_API_KEY`/
│                     `OPENAI_BASE_URL` 存在即可跑，覆盖"本机没配 Codex"
│                     的情况。这条驱动天然读不到 rollout（没有 Codex 会话
│                     记录），`skills_read`/`doctrine_read`/`bypasses` 这几
│                     列在它的报告里标"不可观测"，不强行伪造。
├── judge/
│   ├── structural.rs 结构化裁判：复用 `studio-cli::e2e` 现成的四条思路
│                     （remedy 覆盖、无 state_drift、revise 往返 ≤2、六阶段
│                     全部走到），加两条新的、专门针对 skill 意图的：
│                     - trigger/not_trigger 边界有没有被越界（比如 script
│                       阶段的产物里出现了机位描述，那是 director 的事）；
│                     - 明显的反模式规则（比如 script.story_arc 每段时长
│                       完全相等，大概率是没理解"按内容分配时长"）。
│                     这一档不需要 LLM，纯规则，可重复。
│   └── semantic.rs   LLM-judge：不能只看 SKILL.md——提示词架构重构
│                     （PR #11）之后大半指导性内容搬进了 `.agents/doctrine/`
│                     （运镜语法、故事结构、质量清单/禁用词等）和
│                     `.agents/models/*.md`（模型能力卡），SKILL.md 本身
│                     只剩"什么时候用、调用形状"。按阶段维护一张"相关
│                     doctrine 文件"映射表（比如 storyboard 阶段关联
│                     `camera/grammar.md`、`camera/blocking.md`、
│                     `exemplars/storyboard.md`），连同该阶段 SKILL.md 的
│                     "职责"条款和最终产物一起交给评审 LLM，逐条给
│                     pass/fail + 理由。同时对照 `studio-rollout` 观测到
│                     的 `doctrine_read`：产物质量差但对应文档根本没被
│                     读到，报告要标出这是"文档没被打开"而不是"文档写得
│                     不好"——两者需要的修复动作不一样，混在一起会导致
│                     改错地方。评审 LLM 与被测 driver 的 LLM 允许不是
│                     同一个，避免"自己评自己"的偏置。
└── report.rs     汇总成 `SkillEvalReport`（JSON），本机产物，**不进版本库**
                  （`.gitignore` 加一条）——LLM judge 的评分本身会随模型版本
                  漂移，长期存档会把"漂移"和"skill 文档真的变差了"混在一起，
                  参考价值有限。需要留痕就像 `e2e report`/`exec report` 现在
                  的做法一样，人工挑关键结果贴进 PR 描述。
```

### 新增 `studio-rollout` 共享 crate

`driver/codex.rs` 需要解析 Codex 的 rollout jsonl 拿 token/skills_read/
doctrine_read/bypasses——`crates/studio-cli/src/rollout.rs` 已经有一份，
而且是被真实的字段变化逼出来的三条教训：新旧 Codex 版本参数字段名不同
（`input` vs `arguments`）、必须按工具的 `name`/`namespace` 分类而不是对
参数做子串匹配（否则 `<作品名>.studio/` 这种路径会把读文件误判成 MCP
调用）、`doctrine_read` 要单独于 `skills_read` 之外收集（方法层是按需
加载的，没读到就不能把产出干巴赖到文档头上）。这些坑不该在
`studio-skill-eval` 里用一份平行代码再踩一遍。

新建 `studio-rollout`：纯解析库，不依赖 `studio-core`/`studio-engine`——
它解析的是外部 jsonl 格式，跟本项目阶段图无关，挂在分层表最底部。
`studio-cli` 与 `studio-skill-eval` 都依赖它，彼此不互相依赖；
`studio-cli::rollout` 原有的公开类型/函数改为对新 crate 的 `pub use`，
`e2e report`/`exec report` 的调用点不用改。这是这次唯一碰到 CLAUDE.md
「分层」契约本身的改动，已在那张 crate 表里加了一行并说明理由。

`studio-cli` 新增子命令：

```
studio-cli skill-eval list                              列出内置场景
studio-cli skill-eval run --scenario <id> [--driver codex|direct-llm|scripted]
                                                          跑一个场景，出 JSON + 人读摘要
studio-cli skill-eval diff <old.json> <new.json>         对比两次结果，标出退步项
```

### `scripts/replay-protocol.py` 的下场

顺手迁移掉这个已知的硬规则违规。它做的事（发 JSON-RPC 走六阶段、重放一次
"不要固定 2 秒"的修订）原样变成 `studio-skill-eval` 的一个**脚本场景**
`golden_six_stage_with_revise`，用新 harness（真实 studiod 子进程 + stdio）
跑，行为完全等价，额外收益是它现在可以直接进 `cargo test --workspace`
（脚本场景不需要外部 LLM），不再需要人换了机器之后手动敲一遍。
`docs/e2e.md`、`docs/design.md` 里引用它的地方改指向新命令；
`scripts/replay-protocol.py` 删除。

### 内置场景库（起步集合，后续按需再加）

脚本场景（进 CI/`cargo test`，已实现，见 `crates/studio-skill-eval`）：

1. `golden_six_stage_with_revise` —— 原 `replay-protocol.py` 的等价物。
2. `concurrent_open_reports_busy_with_pid` —— 两个真实子进程打开同一
   bundle，断言第二个拿到的 `project_busy` 附带第一个进程真实的 PID，
   且 remedy 里**不含任何二进制名**（这一条直接对应下面"测试补齐"里
   发现的真实缺陷）。

#### 收敛点：`preview_gate_redirects_to_prompt_pack` 没有进脚本场景库

设计时以为这也能是个脚本场景，实现时发现不行：这条 harness 跑的是
**真实编译出的 `studiod` 二进制**，它内部接的是真实 `studio_pipeline::
Pipeline`，`preview` 是控制面自动执行的确定性阶段——没有真实 ComfyUI，
它根本走不到自己的确认门，只会卡在 `comfy_unavailable`。要在这条 harness
里让它跑到确认门，唯一办法是给 `studiod` 加一个"注入假执行器"的旁路，
而那正是 ADR-0002 要消灭的东西：`studiod` 物理上不能有除 serve 之外的
任何行为分支。所以这条行为该测的地方一直就是对的地方：
`crates/studio-core/src/stage.rs` 的 `preview_revises_back_to_prompt_pack`
（类型层面）和 `crates/studio-engine/tests/deterministic.rs`（配假执行器
的集成测试），不需要也不应该挪进 `studio-skill-eval`。

Agent 场景（本机/按需跑，不进 CI，设计已定、留给 Phase C 实现）：

1. `incident_replay_2026_09_03` —— 就是 `docs/e2e.md` 现在手工跑的那个
   剧本：交一版每镜头 2 秒的剧本 → 用户说"不要固定 2 秒" → 应当一次
   `revise` 加一次 `submit_stage`。这是把人工验收自动化，不是新场景。
2. `ambiguous_user_input_handling` —— 故意给一句有笔误的创意（"20色女性"
   这类），检查 Agent 是否按 `idea` skill 的指示"按最合理理解处理并写进
   assumptions"，而不是卡住反复追问。
3. `retry_vs_revise_confusion_probe` —— 在一个确定性阶段人为制造一次
   `comfy_unavailable`，看 Agent 会不会正确选 `studio.retry_stage` 而不是
   误用 `studio.revise`（`comfyui` skill 明确写了这条区分，这个场景专门
   验证措辞有没有起作用）。
4. `capability_boundary_probe` —— PR #11 落地的能力面双向校验
   （`capability.rs`）之后新增。给一个会让 Agent 想加负面提示词的创意
   方向，走到 `prompt_pack` 阶段、目标模型是 `minimax_h3` 时，检查
   Agent 会不会遵守 `assets/models/minimax_h3.md` 里"不要写 `negative`"
   的边界；就算没读到那张能力卡而误提交，`capability.rs` 应该在 submit
   时就报 `schema_violation` 并给出可执行 remedy，不会等到渲染才发现。
   同时验证文档措辞和运行时兜底两层，其中一层失守另一层也要能兜住。
5. `decision_archive_crosses_stages` —— PR #11 落地的决定档案
   （ADR-0003）之后新增。在某个早期阶段（比如剧本）让用户说一句
   "不要平均切分镜头时长"触发 `studio.revise`，走到后面某个阶段时不再
   重复这句话，检查 Agent 是否真的从 `next_action.decisions` 里读到这条
   历史决定并遵守，而不是要用户在每个阶段重新说一遍——这是 ADR-0003
   "决定档案"整个设计初衷的直接回归测试，不测这个，档案有没有被真正
   用上就只能靠印象判断。

## 测试补齐同批发现的真实缺陷

普查测试现状时发现 `StudioError::remedy()` 里 `ProjectBusy` / `NotAProject`
两个变体的补救文案直接写着 `` `studiod init <路径>` ``——这正是 ADR-0002
要消灭的那类泄露，只是发生在运行时的 `blocked_by.remedy` 通道而不是生成的
静态文档里，所以 `assets.rs` 现有的"文档不提二进制名"测试完全没覆盖到它。
`blocked_by.remedy` 是 Agent 卡住时**第一个**会读的东西，比静态文档更危险。

这次一并修：改写这两条 remedy 为不点名二进制的引导语（参照 AGENTS.md 里
"这超出你的能力范围，提醒用户自己在终端处理"的既有措辞），并在
`studio-core` 里补一个穷尽性测试——对 `StudioError` 每个变体的 `remedy()`
都断言不含 `studiod`/`studio-cli` 字样，防止同类问题再从别的变体冒出来。

## 不在这次范围内

- 渲染 / 后期 / 验收（`preview` 之后）的 Agent 场景，包括
  `studio.self_review`（ADR-0003 新增的内容自评工具，要片子真的存在
  才能调）——`docs/e2e.md` 已经说清楚这一段只能在真实 ComfyUI + GPU 上
  验证，`studio-skill-eval` 不改变这条边界，它跟"真实 Codex 会话"覆盖
  的是同一段（idea → prompt_pack）。
- 场景库的全覆盖——先给起步集合把框架立住，之后照实际踩过的坑往里加，
  跟单元测试一样"发现一个问题、补一个用例"，不追求一次穷尽。
- `direct_llm.rs` 具体接的是哪家 API——用 `OPENAI_API_KEY`/`OPENAI_BASE_URL`
  这一套已有的环境变量约定（跟 CLAUDE.md「本地配置 Codex」一节一致），
  工具调用格式适配 OpenAI 兼容的 `tools`/`tool_calls` 协议；换供应商时
  这一份适配器可能要跟着改，但不影响 `AgentDriver` trait 和上层场景/裁判。
- CI 是否要跑脚本场景——建议跑（它们确定性、无外部依赖，跟其它
  `cargo test` 一样快），但这是"要不要"的产品判断，留到实现时按
  `cargo test --workspace` 现有耗时预算决定，不在架构层面强制。
