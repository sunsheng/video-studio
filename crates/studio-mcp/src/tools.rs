//! 工具注册表。
//!
//! **没有一个带 `run_id`**：服务端启动时认定当前工作目录就是当前项目。
//! 这份注册表是工具面的唯一事实源——`studiod emit-assets` 从这里生成
//! AGENTS.md 与各 SKILL.md 的「Studio MCP」小节，所以文档不可能引用到
//! 不存在的工具名。

use serde_json::{json, Value};

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema，作为 MCP `tools/list` 的 inputSchema。
    pub input_schema: fn() -> Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn stage_arg(desc: &str) -> Value {
    json!({
        "type": "string",
        "description": desc,
        "enum": studio_core::StageId::all().map(|s| s.as_str()).collect::<Vec<_>>()
    })
}

pub const TOOLS: [ToolSpec; 11] = [
    ToolSpec {
        name: "studio.status",
        description:
            "读取决策信封：现在在哪个阶段、该谁行动、下一步要交什么。任何时候不确定就先调它。",
        input_schema: no_args,
    },
    ToolSpec {
        name: "studio.schema",
        description:
            "取回某个阶段产物的 JSON Schema。提交前先看它，不要去猜字段，也不要参考别处的产物。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": { "stage": stage_arg("要查看契约的阶段") },
                "required": ["stage"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.submit_stage",
        description: "提交当前阶段的产物。有确认门的阶段必须同时给出 confirmation；\
                      选项要用 outcome 声明是通过还是打回，不要靠 id 的字面意思暗示。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "outputs": {
                        "type": "object",
                        "description": "阶段产物。顶层键由 studio.status 的 next_action.required_outputs 给出。"
                    },
                    "summary": { "type": "string", "description": "一句话说明这次提交做了什么，会出现在时间线里。" },
                    "confirmation": {
                        "type": "object",
                        "description": "确认门。只有带门的阶段需要。",
                        "properties": {
                            "prompt": { "type": "string", "description": "问用户的话" },
                            "selection_type": { "type": "string", "enum": ["single", "multi"] },
                            "options": {
                                "type": "array",
                                "minItems": 1,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "id": { "type": "string" },
                                        "label": { "type": "string" },
                                        "outcome": {
                                            "type": "string",
                                            "enum": ["approve", "revise"],
                                            "description": "approve 通过并进入下一阶段；revise 把本阶段打回草稿。至少要有一个 approve。"
                                        }
                                    },
                                    "required": ["id", "label"]
                                }
                            }
                        },
                        "required": ["prompt", "options"]
                    }
                },
                "required": ["outputs"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.answer",
        description: "把用户对确认门的选择交回来。选中 outcome=revise 的选项会自动把阶段打回草稿。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "question_id": { "type": "string", "description": "来自 status 的 pending_question.question_id" },
                    "answer": { "type": "string", "description": "选项 id" }
                },
                "required": ["question_id", "answer"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.revise",
        description: "用户提出修改意见时调它。阶段回到草稿，可以立刻重新提交。\
                      它不会失败，也不需要先解除什么占用。作品的进度会整体退回到该阶段，\
                      它之后的阶段一律变回未执行——旧产物留着可以读出来参考。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "stage": stage_arg("要修改的阶段"),
                    "message": { "type": "string", "description": "用户的原话或归纳后的修改意见" }
                },
                "required": ["stage", "message"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.undo",
        description: "撤销上一次修订，把作品整个恢复到那次 studio.revise 之前——\
                      旧产物回来，被退回的下游阶段也恢复已通过。只有一层，且恢复后即失效。",
        input_schema: no_args,
    },
    ToolSpec {
        name: "studio.stage_output",
        description: "读取某个阶段的完整产物。上游被改后，下游的旧产物仍可在这里读到，供参考着改。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": { "stage": stage_arg("要读取的阶段") },
                "required": ["stage"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.timeline",
        description: "读取用户可见的操作历史：每个阶段何时提交、何时挂门、何时被修订。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 } },
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.export",
        description: "把交付物投递到作品的 output/ 目录。后期阶段通过之后才可用。",
        input_schema: no_args,
    },
    ToolSpec {
        name: "studio.retry_stage",
        description: "干净地重试一个卡住的确定性阶段（preview / render / post / review）：\
                      先停掉可能还在跑的执行，再重新跑一次。用在「内容没问题，\
                      只是这次执行失败了」——节点抖动、连接超时、偶发故障。\
                      内容/提示词本身要改，用 studio.revise，不要用这个。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": { "stage": stage_arg("要重试的确定性阶段") },
                "required": ["stage"],
                "additionalProperties": false
            })
        },
    },
    ToolSpec {
        name: "studio.self_review",
        description: "验收的另一半：对成片做内容自评。技术验收（时长、画幅、镜头数、音轨）\
                      由控制面用 ffprobe 实测，它只证明片子是完整的；这个工具收的是\
                      「它好不好看」。固定五个维度各给一条结论，每条都要带一个\
                      **可指认的时间点**和在那一刻看见/听见了什么。\
                      它不改变技术验收的 passed，但不交这一份，作品就不算收尾。",
        input_schema: || {
            let criteria: Vec<&str> = studio_core::rubric::CRITERIA
                .iter()
                .map(|(c, _)| *c)
                .collect();
            let desc = studio_core::rubric::CRITERIA
                .iter()
                .map(|(c, d)| format!("{c}：{d}"))
                .collect::<Vec<_>>()
                .join("；");
            json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "description": format!("逐维度一条，五条齐全。{desc}"),
                        "minItems": 5,
                        "items": {
                            "type": "object",
                            "properties": {
                                "criterion": { "type": "string", "enum": criteria },
                                "verdict": { "type": "string", "enum": studio_core::rubric::VERDICTS },
                                "at_seconds": { "type": "number", "minimum": 0,
                                                "description": "可指认的时间点，必须落在成片时长之内" },
                                "evidence": { "type": "string",
                                              "description": "在那个时间点上看见/听见了什么，以及它为什么支持这个结论。写「还不错」会被退回" }
                            },
                            "required": ["criterion", "verdict", "at_seconds", "evidence"],
                            "additionalProperties": false
                        }
                    },
                    "summary": { "type": "string", "description": "一句话：最强的一点和最弱的一点" }
                },
                "required": ["items", "summary"],
                "additionalProperties": false
            })
        },
    },
];

pub fn tool_list() -> Value {
    json!({
        "tools": TOOLS.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": (t.input_schema)()
        })).collect::<Vec<_>>()
    })
}

pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tool_takes_a_run_id() {
        for t in TOOLS.iter() {
            let s = (t.input_schema)();
            let props = s["properties"].as_object().unwrap();
            assert!(
                !props.contains_key("run_id"),
                "{} 不该有 run_id 参数",
                t.name
            );
            assert!(
                !props.contains_key("project"),
                "{} 不该有 project 参数",
                t.name
            );
        }
    }

    #[test]
    fn no_tool_leaks_internal_paths_or_ids() {
        // 抽象层判据：工具若要求调用方知道 commit、分支名、节点地址、prompt_id
        // 或 bundle 内文件路径，抽象层次就错了。
        for t in TOOLS.iter() {
            let s = (t.input_schema)().to_string();
            for leak in [
                "commit",
                "branch",
                "prompt_id",
                "node_url",
                "db_path",
                "file_path",
            ] {
                assert!(!s.contains(leak), "{} 的参数泄露了实现细节：{leak}", t.name);
            }
        }
    }

    #[test]
    fn names_are_unique_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for n in tool_names() {
            assert!(n.starts_with("studio."), "{n} 缺少 studio. 前缀");
            assert!(seen.insert(n), "工具名重复：{n}");
        }
        assert_eq!(seen.len(), TOOLS.len());
    }

    #[test]
    fn every_tool_has_a_useful_description() {
        for t in TOOLS.iter() {
            assert!(t.description.chars().count() > 12, "{} 的描述太短", t.name);
        }
    }

    #[test]
    fn stage_enums_cover_the_whole_graph() {
        let s = (TOOLS
            .iter()
            .find(|t| t.name == "studio.schema")
            .unwrap()
            .input_schema)();
        let e = s["properties"]["stage"]["enum"].as_array().unwrap();
        assert_eq!(e.len(), 10);
    }
}
