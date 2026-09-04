# MCP 工具面

十一个工具，**没有一个带 `run_id`**：服务端启动时认定当前工作目录就是当前项目。

```
studio.status()                                     -> Envelope
studio.schema(stage)                                -> JsonSchema
studio.submit_stage(outputs, summary?, confirmation?) -> Envelope
studio.answer(question_id, answer)                  -> Envelope
studio.revise(stage, message)                       -> Envelope
studio.undo(stage)                                  -> Envelope
studio.stage_output(stage)                          -> Outputs
studio.timeline(limit?)                             -> Event[]
studio.export()                                     -> ExportResult
studio.comfy.exclude_node(node)                     -> Envelope
studio.retry_stage(stage)                           -> Envelope
```

## 决策信封

```jsonc
{
  "project":    { "title": "...", "stage": "script", "status": "active" },
  "waiting_on": "user",              // agent | user | system
  "blocked_by": null,                // 或 { code, message, remedy }
  "pending_question": { "question_id": "...", "prompt": "...",
                        "selection_type": "single", "options": [...] },
  "next_action": { "kind": "submit_stage", "stage": "...", "capability": "...",
                   "gate": "...", "inputs": {...},
                   "required_outputs": [...], "schema_ref": "..." },
  "progress":   { "completed": 3, "total": 10 }
}
```

`blocked_by.remedy` 是硬要求：任何阻塞都必须给出下一步能调的工具。

## 为什么没有 capability 参数

阶段唯一决定 capability，参数冗余且能填错。映射关系由 `next_action.capability` 告知。

## 为什么没有 advance

deterministic 阶段（preview / render / post / review）在门通过后由控制面
自动推进，Agent 用 `status()` 观察即可。多一个工具就多一种被误用的方式。
`preview` 是其中唯一自己也带确认门的一个：执行完不直接判过，挂起等
480p 预览的构图/内容被确认，才轮到花钱的正式 `render`——所以这一步
仍然只需要 `status()` 观察加 `answer()` 应答，不需要 `advance`。

## 为什么有 exclude_node 和 retry_stage

`revise` 面向「内容不对，要 Agent 重新交」；确定性阶段的执行失败
（节点抖动、连接超时）是另一类问题，内容本身没错，只是这次没跑成。
`studio.retry_stage(stage)` 干净地重跑它：先停掉可能还在跑的 worker
再清错误重试，不会像 `revise` 那样递增 attempt、把下游全部退回未执行。
`studio.comfy.exclude_node(node)` 把怀疑有问题的节点从选节点候选里
临时摘掉，只在这次会话内生效。
