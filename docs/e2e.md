# 观测与验收

**两份独立的报告**，因为读者不同：

| 报告 | 命令 | 看什么 | 数据来自 |
|---|---|---|---|
| Agent 侧 | `studio-cli e2e report` | 阶段推进、确认门、修订往返、token、有没有绕过 MCP | `.studio/trace.jsonl` + Codex rollout |
| 执行侧 | `studio-cli exec report` | 逐镜头排在哪个节点、GPU 等了多久、后期哪一步慢 | `.studio/exec.jsonl` |

前者看协作，后者看吞吐。确定性阶段（渲染 / 后期 / 验收）跑在控制面的后台线程里，
不经过 MCP，所以**不会出现在 Agent 侧那份报告里**——这也是为什么要两份。

两份都支持 `--html` 出单文件报告。

# 端到端验收

**能跑到第几阶段取决于探针结果，不要预设。** render 需要真实 ComfyUI；
开发环境**可能有也可能没有**——`COMFY_NODE` 配好并且 `studio-cli doctor`
探到了，render 就能在这里真跑；探不到才只能在生产环境跑。同理，`post` /
`review` 取决于探到没探到 ffmpeg / ffprobe。

报验收结论时必须附上当时探到了什么（型号、显存、权重清单），否则下一个人
无从判断那次覆盖了多少。

**render 之前的六个阶段（idea → prompt_pack）不一定要等生产环境。** 如果
开发环境（Claude Code 容器）已经装好 Codex CLI 并配置了可用的 model
provider（`codex doctor` 通过、`codex exec` 能正常应答），就可以在这里用
真实 Codex 会话驱动 `studiod`（它没有子命令，被拉起就是 serve）走完
前六个阶段，停在 `render`
（`waiting_on: system`）。这比 `studio-skill-eval` 的脚本场景更真实：验证的是
Codex 读了 AGENTS.md / SKILL.md 之后会不会正确使用工具面，这是脚本模拟不了
的（见 [ADR-0004](decisions/ADR-0004-skill-evaluation.md) 里"脚本场景"与
"Agent 场景"的边界）。步骤同下面「在生产环境怎么跑」一节，只是走到第 7 步
就停，不必往下走。

分工是这样的：

| 在哪 | 跑什么 | 产出 |
|---|---|---|
| 开发环境（Claude Code） | `cargo test --workspace` | 单元、引擎、MCP 协议一致性 |
| 开发环境（已配置 Codex） | 真实 Codex 会话走 render 前六阶段 | `.studio/trace.jsonl`，停在 `render` |
| 生产环境（有 Codex 的机器） | 真人 + Codex 走完 render / 后期 | 完整 `.studio/trace.jsonl` |
| 任一侧 | `studio-cli e2e report` | `report.json` |
| 开发环境 | 读 `report.json` 分析、改代码 | 下一轮 |

CI 里**仍然不**跑端到端——这是改动碰了 MCP 工具面 / 阶段图时才做的验收，
不是每次 PR 必跑的检查（见 CLAUDE.md「标准工作流程」）。

## 在生产环境怎么跑

### 1. 建一部干净的作品

```bash
/opt/video-studio/studio-cli doctor          # 先体检
/opt/video-studio/studio-cli init ~/e2e/千岛湖.studio
cd ~/e2e/千岛湖.studio
```

### 2. 打开 Codex，照剧本走

用这段创意（取自 2026-09-03 那次真实会话，翻车就发生在第 4 步）：

> 10秒5个镜头的千岛湖游玩vlog；主角20岁女性，长发、黑发、白裙、板鞋；欢乐；以30度侧脸为主要机位

然后按顺序：

1. 让它做完 brief，进入选题
2. 选题门上答**确认**
3. 让它做剧本——**先让它交一版每镜头 2 秒的**
4. 在剧本门上说：**「不要固定2秒，要根据镜头内容智能分配」**
5. 它应当重新提交智能时长版；答**确认**
6. 继续走分镜、视觉资产、提示词，各自确认
7. 停在 `render`（`waiting_on: system`）

第 4 步是重点。前身项目在这里花了 10 分钟 18 次调用并绕去写 SQL；
健康的实现是一次 `studio.revise` 加一次 `studio.submit_stage`。

### 3. 出报告

```bash
studio-cli e2e report \
  --rollout ~/.codex/sessions/<本次会话>.jsonl \
  --html ~/e2e/report.html \
  -o ~/e2e/report.json
```

退出码非 0 表示未通过。报告同时打印人读的摘要。

`--rollout` 是关键：MCP server 只看得见自己被调用了什么，
**看不见 token 用量、Codex 读没读 SKILL.md、有没有绕过 MCP 直接跑命令**。
这些只有 Codex 自己的会话记录里有。合并之后报告才完整，
并且多一条验收「全程没有绕过 MCP」。

`--html` 出一份单文件报告（Tailwind CDN，其余全内联），拷到哪儿都能打开。

### 4. 渲染跑完之后再出一份执行侧报告

```bash
studio-cli exec report --html ~/e2e/exec.html -o ~/e2e/exec.json
```

这份覆盖 Agent 侧看不到的部分：

- **逐镜头**：选节点 / 提交 / 排队渲染 / 下载 各多久，落在哪个节点，`prompt_id` 是多少
- **节点负载**：几个节点分了几个镜头，各自累计多久——并行度够不够一眼能看出来
- **各步骤**：按耗时排序。后期那几步会带上细节，例如 `concat parts=5 stream_copied=true`
  （直接复制流没重编码）
- **失败**：哪一步、哪个镜头、什么错误码

渲染那一步通常占整条流水线九成以上——它是 GPU 时间。

### 4. 带回开发环境

把 `report.json` 交给 Claude Code 分析。不需要带整个作品目录——报告里已经
有全部结论所需的信息；确实要看产物时再带 `stages/*.json`。

## 报告里有什么

### 可观测与不可观测

| 想知道 | 从哪来 | 没有 rollout 时 |
|---|---|---|
| 走到哪个阶段、用了几次调用 | `.studio/trace.jsonl` | 有 |
| 各阶段耗时（区分等待用户） | 同上 | 有 |
| 每条阻塞有没有 remedy | 同上 | 有 |
| token 用量 | Codex rollout | 不可观测 |
| 读过哪些 SKILL.md | Codex rollout | 不可观测 |
| 有没有绕过 MCP 跑命令 | Codex rollout | 不可观测 |

耗时里**等待用户确认单独拆出来，不计入有效耗时**——那是人在看，不是系统在跑。
剩下的分成两段：控制面自己处理的时间，和两次调用之间 Agent 在想在写的时间。

## JSON 报告的结构

```jsonc
{
  "generated_at": "...",
  "bundle": "/home/you/e2e/千岛湖.studio",
  "total_calls": 14,
  "failed_calls": 0,
  "calls_by_tool":  { "studio.submit_stage": 7, "studio.answer": 5, ... },
  "calls_by_stage": { "idea": 1, "selection": 2, ... },
  "errors": [ { "at": "...", "tool": "...", "code": "...", "remedy_present": true } ],
  "revise_round_trips": [1],        // 每次 revise 到下一次成功 submit 用了几步
  "stages_reached": ["idea", "selection", ...],
  "verdicts": [ { "name": "...", "passed": true, "detail": "..." } ],
  "passed": true
}
```

## 四条验收

| 验收 | 怎么判 |
|---|---|
| 每条阻塞都带补救路径 | 所有失败调用的 `remedy_present` 都为 true |
| 状态未被外部改动 | 没有出现过 `state_drift` |
| 修订往返一次过 | 每次 `revise` 到下一次成功 `submit_stage` ≤ 2 步 |
| 提交 ComfyUI 前六阶段全部走到 | idea → prompt_pack 都在 `calls_by_stage` 里 |

`revise_round_trips` 是最能说明问题的一列。理想值是 `[1]`：修订之后紧接着就
重新提交。前身项目那次的形状是 18，报告会判它未过。

## 留痕是怎么产生的

MCP server 每次工具调用往 `.studio/trace.jsonl` 追加一行，只记调用的形状——
工具名、阶段、成功与否、错误码、那条阻塞有没有 remedy、耗时。
**不记产物内容**，产物本身在 `stages/*.json` 里。

`studio-cli pack` 不会把 trace 打进包。

## 换机器之后先跑协议层冒烟

`studio-skill-eval` 的脚本场景（见 [ADR-0004](decisions/ADR-0004-skill-evaluation.md)）
不用 Codex，直接跟真实编译出的 `studiod` 二进制说 JSON-RPC 走完六个阶段
（中间重放一次「不要固定 2 秒」的修订）。它**不能替代**端到端——真正的
端到端要验证的是 Codex 读了 AGENTS.md 和 SKILL.md 之后会不会正确使用工具面，
那是脚本模拟不了的。但它能在换了机器、换了构建之后快速确认协议层没坏，而且
是确定性的、不需要任何外部依赖，直接进 `cargo test --workspace`：

```bash
cargo test -p studio-skill-eval
```

这是原来 `scripts/replay-protocol.py`（一个独立的 Python 脚本，现已删除）
的等价物——同样的调用序列，换成跑真实二进制的 Rust 集成测试，不用额外装
Python，也不用手动导出 fixtures、手动 init 一个作品目录。

## 报告说未过之后

拿着 `report.json` 回开发环境。常见几种形状：

- **某条 `remedy_present: false`** → `StudioError::remedy()` 漏了这个变体的
  可执行路径。`crates/studio-core/src/error.rs` 有两个测试守着，但它们只能
  检查「非空」和「提到了某个工具」，措辞是否真的有用要靠这里的报告。
- **`revise_round_trips` 里有大数** → 中间夹了多余的调用。看 `calls_by_tool`
  里多出来的是什么，多半是 Agent 在反复 `status` 探路，说明信封没说清楚。
- **`stages_reached` 少了一段** → 卡在某个门上了。看 `errors` 里最后一条。
- **出现 `state_drift`** → 有人绕过 MCP 直接改了 `.studio/`。这是最严重的一种，
  说明沙箱配置或 AGENTS.md 的约束没起作用。
