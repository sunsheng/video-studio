# MCP 工具面

九个工具，**没有一个带 `run_id`**：服务端启动时认定当前工作目录就是当前项目。

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
  "progress":   { "completed": 3, "total": 9 }
}
```

`blocked_by.remedy` 是硬要求：任何阻塞都必须给出下一步能调的工具。

## 为什么没有 capability 参数

阶段唯一决定 capability，参数冗余且能填错。映射关系由 `next_action.capability` 告知。

## 为什么没有 advance

deterministic 阶段（render / post / review）在门通过后由控制面自动推进，
Agent 用 `status()` 观察即可。多一个工具就多一种被误用的方式。
