# 端到端验收

**端到端测试不在开发环境跑。** 它需要一个真实的 Codex 会话驱动真实的 MCP
server——开发环境没有 Codex，也没有 GPU、ComfyUI 和 ffmpeg。

分工是这样的：

| 在哪 | 跑什么 | 产出 |
|---|---|---|
| 开发环境（Claude Code） | `cargo test --workspace` | 单元、引擎、MCP 协议一致性 |
| 生产环境（有 Codex 的机器） | 真人 + Codex 走一遍 | `.studio/trace.jsonl` |
| 生产环境 | `studiod e2e report` | `report.json` |
| 开发环境 | 读 `report.json` 分析、改代码 | 下一轮 |

CI 里**不**跑端到端。

## 在生产环境怎么跑

### 1. 建一部干净的作品

```bash
/opt/video-studio/studiod doctor          # 先体检
/opt/video-studio/studiod init ~/e2e/千岛湖.studio
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
/opt/video-studio/studiod e2e report -o ~/e2e/report.json
```

退出码非 0 表示未通过。报告同时打印人读的摘要。

### 4. 带回开发环境

把 `report.json` 交给 Claude Code 分析。不需要带整个作品目录——报告里已经
有全部结论所需的信息；确实要看产物时再带 `stages/*.json`。

## 报告里有什么

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

`studiod pack` 不会把 trace 打进包。

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
