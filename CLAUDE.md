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
studio-rollout  解析 Codex 会话记录（rollout jsonl）：token 用量、
                skills_read/doctrine_read、疑似绕过 MCP 的动作。★ 不依赖
                core/engine——纯外部 jsonl 格式解析，跟阶段图无关。被
                studio-cli（e2e report/exec report）与 studio-skill-eval
                （CodexDriver）共用，两者互不依赖。
studio-skill-eval Skill 评估：像测代码一样测 AGENTS.md / SKILL.md。依赖
                core + engine + mcp + rollout。只被 studio-cli 依赖，见
                ADR-0004。
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

## 六个阶段：从需求到复盘

**大改动**（新特性、跨 crate 重构、动契约的改动）按这六个阶段走。
小修小补直接进下面的「标准工作流程」，不必套这一层。

| 阶段 | 做什么 | 产物 |
|---|---|---|
| 一 意图理解 | 读清楚要什么、为什么、边界在哪 | 对齐过的问题清单 |
| 二 规格设计 | 定形状与契约，写清楚为什么这么定 | `docs/specs/SPEC-NNNN-*.md` |
| 三 执行规划 | 拆步骤、定每步怎么验、在哪里切提交 | `docs/plans/PLAN-NNNN-*.md` |
| 四 编码实现 | 按计划一步步做，每步做完仓库都是绿的 | 代码 + 提交 |
| 五 审核验证 | CI、真机、Codex 端到端与 Review | PR 上的结论摘要 |
| 六 运维复盘 | 把学到的东西写回文档和 issue | 复盘评论、文档更新 |

三条硬规矩：

1. **规格和计划必须落到文件里**，不能只存在于对话里——上下文一压缩就丢了。
   写进 `docs/specs/` 和 `docs/plans/`，随代码一起进版本库。
2. **进阶段二之前，意图里所有不明白的地方都要跟人核对完。** 靠查官方 API
   或纯逻辑能定的自己定，定不了的才问；但别什么都问。
3. **跟既有文档规约冲突时，两件事都不做**：不因为「已经规定了」就不改，
   也不自己直接改。**拿出来跟人确认，由人裁决。**

计划赶不上变化时**改计划文件，不要默默扩大范围**。SPEC-0014 的 V9、
PLAN-0014 的 S5 分波都是实现时才发现要做的，补写进了原文件并说明缘由。

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
model = "gpt-6-astra"            # 主力，见下面「选哪个模型」
model_reasoning_effort = "high"  # 见下面「选哪个 effort」

[model_providers.envproxy]
name = "envproxy"
base_url = "<$OPENAI_BASE_URL 的值>"   # 注意带上 /v1
env_key = "OPENAI_API_KEY"
wire_api = "responses"   # 这个版本不认 "chat" 了
```

**配置改完用 `--strict-config` 验一次**：

```bash
codex exec --strict-config --skip-git-repo-check "回答两个字：就绪"
```

不带这个开关时，**拼错的或不存在的键会被静默忽略**——你以为配了 `high`，
实际跑的是默认值。这类静默失效正是本项目最不能接受的失败方式，所以配置
一改就验，看输出里的 `model:` 和 `reasoning effort:` 两行是不是你要的。

#### 选哪个模型

主力 `gpt-6-astra`，往下退 `gpt-5.6-sol` → `gpt-5.6-terra`。
`gpt-6-astra` **不在网关的型号列表里，但直接请求完全正常**（2026-09-05 复测，
端到端与 Review 两条都跑过）——照列表判断会白白退到更弱的模型。

**判断可用与否只看一件事：真发一次请求。**

```bash
curl -sS -X POST "$OPENAI_BASE_URL/v1/responses" \
  -H "Authorization: Bearer $OPENAI_API_KEY" -H "Content-Type: application/json" \
  -d '{"model":"gpt-6-astra","input":"hi"}'
```

回来是正常 response 就是可用，是 `error` 就退到下一个。

**不要用 `/v1/models` 的列表做判断——它不准**，上面那条就是活例子：
响应的 `model` 字段回的也是 `gpt-6-astra`，网关没有静默改路由。

顺带一条仍然成立的老经验：**`model` 要写具体型号，不要写别名。**
`gpt-5.6` 是 `gpt-5.6-sol` 的别名，两者是同一个模型，但 Codex 的模型元数据表
里只有全名——写别名会每次报
`Model metadata for 'gpt-5.6' not found. Defaulting to fallback metadata`，
然后用兜底元数据跑。

#### 给 Review 换一个更强的模型

**`review_model` 这个键对 `codex exec review` 不生效。** 官方配置参考写得很
明确：它是「Optional model override used by `/review`」——只管交互式那个斜杠
命令。实测跑 `codex exec review` 时打印的仍是主力 `model`。

要让 Review 用更强的模型 / 更高的 effort，**在命令行给**：

```bash
codex exec review --base main -m gpt-6-astra -c model_reasoning_effort="xhigh"
```

#### 选哪个 effort

**按任务难度动态定，不要固定一个值。**

官方配置参考列的合法值是 `minimal｜low｜medium｜high｜xhigh`，
**没有 `max`**；但实测 CLI 与网关都接受 `max`。文档与实现不一致时，
生产配置取保守的一侧——**用 `xhigh` 封顶**，需要 `max` 时先自己验一次。

各模型对低档位的支持（2026-09-05 实测）：

| 模型 | `none` | `minimal` | `low` 及以上 |
|---|---|---|---|
| `gpt-6-astra` | ❌ 不支持 | — | ✅ |
| `gpt-5.6-sol` | ✅ | ✅（归一成 `none`） | ✅ |
| `gpt-5.6-terra` | ✅ | ✅（归一成 `none`） | ✅ |

**`gpt-6-astra` 不接受 `none`**，而 CLI 不显式配置时默认就是 `none`
（CLI 没把它真发出去所以没报错，但直接调 API 传 `none` 会被拒）。
用 astra 时显式写一个 effort，别赌默认值。

按任务挑：

| 任务 | effort |
|---|---|
| 冒烟、确认环境通不通、问一句话 | `low` |
| 走 MCP 端到端验收（照剧本填表、按 remedy 修正） | `high` |
| Code Review、要读懂跨 crate 契约再下判断的分析 | `xhigh` |

命令行临时覆盖用 `-c model_reasoning_effort="xhigh"`，不必改配置文件。

`base_url` 要带 `/v1`，不带会 404（`codex doctor` 的可达性探测仍会说 reachable，
它探的是别的路径，不能替代这一步）。

配好之后 `codex` / `codex exec` / `codex review` / `codex doctor`
都不用再带 `-c` 覆盖参数，直接跑。`codex doctor` 显示
`reachability mode: provider auth` 且对应 provider 的 endpoint
`reachable` 就算装配成功；这是判断「本机能不能跑 Codex」的标准，
不要只看默认 provider 报 401 就下结论。跑一次 `codex exec` 确认输出里
**没有** `Model metadata ... not found` 这类警告，才算真配好。

冒烟测 MCP 工具（比如让 Codex 调 `studio.status`）：这个 Codex 版本
不会自动读 bundle 里 `studio-cli init` 生成的 `.codex/config.toml`
（那是给别的 Codex 版本/前端用的约定），要用
`codex mcp add video-studio -- <studiod 路径>` 全局注册——`studiod`
没有子命令，不用带 `serve`。用完 `codex mcp remove video-studio` 清掉。
MCP 工具调用默认会卡在审批——要不要绕过、用什么方式绕过，取决于当时
运行 Codex 的那台机器本身有没有更外层的沙箱防护，不要把某一次会话
「这层已经沙箱化所以绕过审批安全」的判断当成通用结论照抄到别的机器上。

**`codex exec` 默认 `approval: never`，而这个值会把 MCP 调用直接拒掉**，
报「MCP tool call requires approval, but approval policy is never」——
它不是「不问直接放行」，是「不问直接拒绝」。这一档下 Codex 连一次工具都
调不出来，端到端等于没跑。判断那台机器外层确实有沙箱之后，用
`--dangerously-bypass-approvals-and-sandbox`（`sandbox: danger-full-access`）；
判断不了就别跑，跑出来的「零调用」不是结论。

#### provider 的流式连接会偶发挂死

2026-09-05 实测：`codex exec` 有时第一个 token 都不出，日志停在
`ERROR: Reconnecting... 1/5`，之后再无输出，rollout 里最后一条是
`token_count`。同一条命令重跑就通了。

**别急着改配置**——先把这三样分开验：

```bash
# 1. 非流式：应当 200
curl -sS -X POST "$OPENAI_BASE_URL/v1/responses" -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" -d '{"model":"gpt-6-astra","input":"hi"}' -w " %{http_code}\n" -o /dev/null
# 2. 流式：应当立刻吐 event: response.created
curl -sS -N -X POST "$OPENAI_BASE_URL/v1/responses" ... -d '{... ,"stream":true}' | head -c 200
# 3. 代理：recentRelayFailures 应当是空的
curl -sS "$HTTPS_PROXY/__agentproxy/status"
```

三样都正常就说明是上游偶发，重跑一次即可；**不要因此去动 provider 配置或
换模型**，那会把一个偶发问题变成一次没必要的降级。

### Codex 验收的真实覆盖范围

**别假设这台机器有什么或没有什么——先探针，按结果决定跑到哪一步。**

以前这里写着「开发环境一定没有 GPU、没有 ComfyUI」。那句话现在是错的：
云端会话可以配好 `COMFY_NODE` 指向一台真实的 ComfyUI（实测过 A800 80GB
的负载均衡代理），也可以装好 Codex CLI。**能力面是探出来的，不是写死的**——
把它写死的代价是明明能验的东西没验，或者反过来，宣称验过了其实没有。

探针清单，每次开工先跑一遍（`studio-cli doctor` 覆盖前三项）：

| 探什么 | 怎么探 | 探到了能多跑什么 |
|---|---|---|
| ComfyUI | `GET $COMFY_NODE/system_stats`（带 `Authorization: Bearer $COMFY_TOKEN`） | `preview` / `render`，以及 `studio-comfy` 的集成测试 |
| 集群构成 | 同上——**那一侧是多节点代理时返回的是「节点地址 → 该节点原始 stats」的对象**，`studio-cli doctor` 会把它归并成一行报出来 | 几台、什么卡、多少显存。报验收结论必须带上它 |
| 模型权重 | `GET $COMFY_NODE/models/<类型>` | 哪个系列能真跑——**节点在 `object_info` 里不等于权重下载了** |
| 节点类型 | `GET $COMFY_NODE/object_info/<节点类名>` | 某个 workflow 能不能编出来 |
| ffmpeg / ffprobe | `studio-cli doctor` | `post` / `review` |
| Codex | `codex doctor` + 跑一次 `codex exec` 确认没有 metadata 警告 | 走真实 MCP 会话的端到端验收 |

按探针结果分档：

- **什么都没探到**：只有 render 之前的六个阶段（idea → selection → script →
  storyboard → visual_assets → prompt_pack）能真跑，`preview` 往后顶多验证到
  「提交后结构化阻塞在 `comfy_unavailable`」——**不能把这当成渲染链路已验证**。
- **探到 ComfyUI**：`preview` / `render` 可以真跑，视觉资产可以真出图。
- **再探到 ffmpeg**：`post` / `review` 也能跑，十个阶段可以在这台机器上走完。

**不得声称集成通过而不说明当时探到了什么。** 报结论时把探针结果一起写出来
（型号、显存、权重清单），否则下一个人无从判断那次验收覆盖了多少。

### 图校验通过 ≠ 画面是对的

ComfyUI 的 `/prompt` 会先校验再入队，拿到 `prompt_id` 且 `node_errors` 为空
说明**接线合法**。这很有用（不烧 GPU 就能验一张图），但它证明不了画面是对的。

一个实例，2026-09-05：preview 的 turbo 叠加层，两个 head × 开关，
**四种组合的图校验全过**。真机出片一看，reference + 4 步的画面是坏的——
色带、光晕、底部有幻觉出来的字形。排查试了五种变体才定位：不是接线顺序
（把 LoRA 换到 SigmaShift 之后画面一模一样地坏），是**调度器**——`beta` 是
reference head 在 20 步下的配套档位，步数降到 4 就不成立。

所以：

- **要改成 `bindings_verified: true`，必须真机出片并且人眼看过**，
  图校验通过只够写「接线已验证」，不够写「这条组合可用」。
- 报验收结论时说清楚验到哪一层：图校验通过、跑完出片、还是人眼确认过画面。
- 机器能断言的只有下限（跑完了、有产出、参数换对了）。画面好不好机器验不了，
  别让测试的绿色假装它验了。

生产环境的最终验收仍然是在宿主机跑 `scripts/smoke.sh`——那是发布前的关口，
不因为开发环境也能跑渲染而取消。

### 校验器看不见的东西，它就当没看见

同一类失败还有更早的一档：**图校验通过，执行时直接抛异常**。
`/prompt` 对认不出来的输入键是**当没看见**的，不报错。

2026-09-05 的实例：`ResizeImageMaskNode.resize_type` 是动态组合框
（`COMFY_DYNAMICCOMBO_V3`），选一个键、那个键自带一组子输入。API 格式里
它是**平铺的点号兄弟键**：

```jsonc
"resize_type": "scale dimensions",
"resize_type.width": 1080,
"resize_type.height": 1920
```

试过的另外三种写法——`{"key": …, "multiplier": …}`、嵌套对象、二元组——
**图校验全部通过**，执行时抛
`TypeError: execute() missing 1 required positional argument: 'resize_type'`。
唯一说出真相的是纯字符串写法的验证错误，它点名了
`"input_name": "resize_type.multiplier"`。

所以探一个没见过的节点参数形态时：**别停在「提交成功」，要真跑一次**，
而且要核对产物（这次是输出尺寸）。「提交成功」这一档能挡住的错误比想象的少。

**同一个坑，更贵的一次。** `COMFY_AUTOGROW_V3` 的多参考槽位也是这套点号编码
（`"ref_images.ref_image_1": ["load", 0]`）。组装器一开始写成嵌套对象
`{"ref_images": {"ref_image_1": [...]}}`——图校验通过、图跑得完、有产出文件，
**参考一个都没进模型**，加载节点是死的。这个错随 PR #19 合并，一直活到
2026-09-05 才被抓到，期间 `references` 声明了等于没声明。

抓到它的方法值得单记：**先做阳性对照。** 查「音频参考为什么不生效」时，
先换一张完全不同的参考图，看输出变不变——不变就说明问题不在音频，在更
上游。再退一步做三档对拍（不挂 / 挂绿 / 挂红），三份输出逐字节相同，
话就说死了。

定位形状的判据同样便宜：**把加载节点指向一个不存在的文件。** 连线被认出来，
节点就是活的，图校验会拒；认不出来就是死节点，校验照过。不烧 GPU，
六种候选形态一轮就分得清。

**验收断言要落在「东西有没有起作用」上，不是「跑完了没有」。** 这个 bug
躲过了全部既有真机测试，因为它们只断言「图合法、跑完了、有产出」——而这
三样在错误形态下全部成立。现在 `real_comfy.rs` 里两条测试守着：换参考图
画面必须不同、换音频参考音轨必须不同。

### history 的 outputs 里不全是产物

加载类节点会把自己的输入原样回显进 `outputs`，`type` 是 `"input"`。
`LoadVideo` 就这样。挑产物时**只认 `type == "output"`**——不然带 clip 锚点
或 video 参考的镜头会把锚点素材当成渲染结果登记下来，一路绿到交付。

这条已经写进 `studio-comfy::collect_files`，测试守着。留在这里是因为它跟
上面两条是同一类：**机器给了个「成功」，但成功的不是你要的那件事。**

### 超时值不要压在实测耗时上

2026-09-05：上传参考图跟 `/prompt`、`/history` 共用一个 30 秒读超时，而量到
传一张 1.09 MB 的卡片图经当时那条代理要 25–38 秒。超时值正好压在观测耗时的
中位数上——不是留了余量偏小，是**根本没有余量**。

（**那几个数字不是干净的基线**：事后得知那条代理当时正有已知故障，同一张图
在故障期外量到过 14.5 秒。但这不改变结论——几 MB 的传输和几百字节的 JSON
凭什么共用一个上限。)

表现出来又是「机器说成功了」的那个形状：锚点视图图出来了、下载了、落盘了、
`status` 是 `ready`，只是传不回 ComfyUI，于是后面五个派生视图全部报
「参考图**还没生成出来**」——一句与事实相反的话，最难查的那种。

两条做法：

- **小 JSON 的控制面调用和大块传输（上传/下载）用不同的超时值。** 前者短一点
  能早发现节点没反应，后者要留一个数量级的余量。
- **失败原因要分类记，别让下游猜。**「没生成出来」和「生成了但传不回去」
  排查方向完全相反，而下游看到的现象是同一个「缺参考图」。

### 端到端能抓到单测和真机验收都抓不到的东西

上面这条，还有「部分视图失败却照样上了确认门」那条（规格 §6.8 早就写了
不放行，实现里只在**一个都没成**时才拦），都是**单元测试全绿、真机验收也过了**
之后，第一次跑完整十阶段 Codex 会话才现形的。

原因不神秘：这类分支只在坏路径上走到，而正路径的测试永远碰不到它；真机验收
又是一条一条验单点，不会把「锚点上传失败」和「后面五个视图」串起来看。

所以「标准工作流程」第 6 步不是可选的收尾动作。碰到 MCP 工具面或阶段图的
改动，Codex 端到端该跑就跑。

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

### 本机有条件通过（先探针，探到了才跑）

- **ffmpeg / ffprobe：** 探到就跑 `studio-media` 的集成测试
- **ComfyUI：** 探到就通过 HTTP 跑 `studio-comfy` 的集成测试，以及真实的
  `preview` / `render`。**接入方式是单个 URL**（`COMFY_NODE`）——那一侧通常
  是个代理，多节点的分发由它负责；需要鉴权就配 `COMFY_TOKEN`。
  **代理那一侧是多节点时，`503` 常常只是排队，不是故障**：所有节点都到了
  并发上限时它会一直排队等，直到调用方自己的 HTTP 客户端超时。
  `error` 字段是 `no healthy node` 才是集群真的挂了，其他（如
  `context deadline exceeded`）都是「有节点，只是都在忙」。见 SPEC-0017
- **Codex：** 探到 Codex CLI 且 `codex doctor` 通过，就用真实 Codex 会话走
  MCP 端到端。能走到第几阶段取决于上面两项探到了什么，见「Codex 验收的
  真实覆盖范围」

### 环境检测

- `studio-cli doctor` 检查 ComfyUI、ffmpeg、ffprobe 是否可用；在作品目录里
  运行时，还检查该作品 `.codex/config.toml` 指向的 `studiod` 路径是否仍然
  有效。**它不检测本机是否装有 Codex CLI 本身**——那用 `codex doctor` 查。
- 根据检测结果，选择性运行相应的集成测试
- **不得声称集成通过而不说明环境前置条件**，报结论时附上探针结果
- 环境变量里如果同时有 `OPENAI_API_KEY` 和 `OPENAI_BASE_URL`，就据此配置 Codex
  用于本机测试；缺一个都不算满足 Codex 部署条件，按未装处理
- **`COMFY_NODE` 与 `COMFY_TOKEN` 同理**：探到就用，探不到就按没有 ComfyUI
  处理，走结构化阻塞，不降级
- 用 Python 脚本直连 ComfyUI 调试时注意：某些代理前面的 Cloudflare 按 UA
  指纹拦 POST，`Python-urllib/*` 会拿到 403 / code 1010，同一个请求体用
  curl 或 `ureq` 就是 200。**控制面用的是 `ureq`，不受影响**，但临时脚本
  会撞上——加个常见 UA 即可
- **CI 中不运行 Codex 端到端测试。** 不管上述两个环境变量是否存在，CI 只跑
  本机必能通过的 `cargo test` 和 `emit-assets --check`；Codex 端到端测试只在
  本机手动按需触发，不接入 CI 流水线

### 真实验收

- 生产环境集成验收在宿主机跑 `scripts/smoke.sh`
- 不得用 mock 通过来宣称链路跑通
