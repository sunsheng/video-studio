//! 读 Codex 的会话记录（rollout jsonl）。
//!
//! MCP server 只看得见自己被调用了什么，看不见 Codex 那一侧发生了什么——
//! 用了多少 token、读没读 SKILL.md、有没有绕过 MCP 直接跑 shell。
//! 这些只有 Codex 自己的记录里有。
//!
//! 所以端到端报告可以合并两份数据：服务端的 `.studio/trace.jsonl` 给出
//! 阶段推进与耗时，Codex 的 rollout 给出 token 与绕行行为。
//! 没有 rollout 也能出报告，只是这几列标成「不可观测」。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
    pub reasoning_output: u64,
    pub total: u64,
    /// 模型上下文窗口，用来看占用比例。
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexCalls {
    /// 调到本项目 MCP 工具的次数。
    pub studio_mcp: usize,
    /// 其它 MCP server。
    pub other_mcp: usize,
    /// 本地命令。
    pub shell: usize,
    pub web: usize,
    pub other: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rollout {
    pub source: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub reasoning_blocks: usize,
    pub tokens: Tokens,
    pub calls: CodexCalls,
    /// 读到过哪些 SKILL.md——这是唯一能观测 skill 是否被用上的办法。
    pub skills_read: Vec<String>,
    /// 疑似绕过 MCP 的动作：碰 `.studio/`、直接跑 studiod 子命令、改 stages/。
    pub bypasses: Vec<String>,
}

pub fn parse(path: &Path) -> std::io::Result<Rollout> {
    let text = std::fs::read_to_string(path)?;
    let mut r = Rollout {
        source: path.display().to_string(),
        ..Default::default()
    };
    let mut skills: BTreeSet<String> = BTreeSet::new();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(String::from);
        if r.started_at.is_none() {
            r.started_at = ts.clone();
        }
        if ts.is_some() {
            r.ended_at = ts;
        }

        let payload = v.get("payload").unwrap_or(&Value::Null);
        match v.get("type").and_then(|t| t.as_str()) {
            Some("session_meta") => {
                r.session_id = payload
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .map(String::from);
            }
            Some("turn_context") => {
                if r.model.is_none() {
                    r.model = payload
                        .get("model")
                        .and_then(|s| s.as_str())
                        .map(String::from);
                }
            }
            Some("event_msg") => {
                if payload.get("type").and_then(|t| t.as_str()) == Some("token_count") {
                    take_tokens(payload, &mut r.tokens);
                }
            }
            Some("response_item") => match payload.get("type").and_then(|t| t.as_str()) {
                Some("message") => match payload.get("role").and_then(|s| s.as_str()) {
                    Some("user") => r.user_messages += 1,
                    Some("assistant") => r.assistant_messages += 1,
                    _ => {}
                },
                Some("reasoning") => r.reasoning_blocks += 1,
                Some("custom_tool_call") | Some("function_call") => {
                    let input = payload.get("input").and_then(|s| s.as_str()).unwrap_or("");
                    classify(input, &mut r.calls);
                    collect_skills(input, &mut skills);
                    if let Some(b) = sniff_bypass(input) {
                        if !r.bypasses.contains(&b) {
                            r.bypasses.push(b);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    r.skills_read = skills.into_iter().collect();
    Ok(r)
}

/// token_count 事件是累计值，后面的覆盖前面的。
fn take_tokens(payload: &Value, t: &mut Tokens) {
    let Some(info) = payload.get("info") else {
        return;
    };
    if let Some(w) = info.get("model_context_window").and_then(|v| v.as_u64()) {
        t.context_window = Some(w);
    }
    let Some(total) = info.get("total_token_usage") else {
        return;
    };
    let g = |k: &str| total.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    t.input = g("input_tokens");
    t.cached_input = g("cached_input_tokens");
    t.output = g("output_tokens");
    t.reasoning_output = g("reasoning_output_tokens");
    t.total = g("total_tokens");
}

fn classify(input: &str, c: &mut CodexCalls) {
    if input.contains("studio.") || input.contains("studio_") && input.contains("mcp") {
        c.studio_mcp += 1;
    } else if input.contains("mcp__") {
        c.other_mcp += 1;
    } else if input.contains("exec_command") || input.contains("write_stdin") {
        c.shell += 1;
    } else if input.contains("web__run") || input.contains("web_search") {
        c.web += 1;
    } else {
        c.other += 1;
    }
}

fn collect_skills(input: &str, out: &mut BTreeSet<String>) {
    // 形如 .agents/skills/<name>/SKILL.md
    let needle = "skills/";
    let mut idx = 0;
    while let Some(pos) = input[idx..].find(needle) {
        let start = idx + pos + needle.len();
        let rest = &input[start..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !name.is_empty() && rest[name.len()..].starts_with("/SKILL.md") {
            out.insert(name.clone());
        }
        idx = start.max(idx + 1);
    }
}

/// 绕过 MCP 的迹象。这是重写这个项目的初衷，报告里必须显眼。
fn sniff_bypass(input: &str) -> Option<String> {
    let patterns: [(&str, &str); 5] = [
        (".studio/studio.db", "直接读写状态库 .studio/studio.db"),
        (".studio/studiod.lock", "直接碰锁文件"),
        ("UPDATE ", "在会话里执行了 SQL UPDATE"),
        ("sqlite3", "直接使用 sqlite3"),
        ("studiod submit", "试图用 CLI 推进阶段"),
    ];
    for (needle, desc) in patterns {
        if input.contains(needle) {
            return Some(desc.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(lines: Vec<Value>) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("rollout.jsonl");
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&p, body).unwrap();
        (d, p)
    }

    #[test]
    fn tokens_take_the_latest_cumulative_value() {
        let (_d, p) = write(vec![
            json!({"timestamp":"2026-09-03T00:00:00Z","type":"event_msg","payload":{
                "type":"token_count","info":{"model_context_window":258400,
                "total_token_usage":{"input_tokens":100,"cached_input_tokens":10,
                                     "output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120}}}}),
            json!({"timestamp":"2026-09-03T00:01:00Z","type":"event_msg","payload":{
                "type":"token_count","info":{"model_context_window":258400,
                "total_token_usage":{"input_tokens":900,"cached_input_tokens":300,
                                     "output_tokens":80,"reasoning_output_tokens":40,"total_tokens":980}}}}),
        ]);
        let r = parse(&p).unwrap();
        assert_eq!(r.tokens.input, 900);
        assert_eq!(r.tokens.output, 80);
        assert_eq!(r.tokens.total, 980);
        assert_eq!(r.tokens.context_window, Some(258400));
        assert_eq!(r.started_at.as_deref(), Some("2026-09-03T00:00:00Z"));
        assert_eq!(r.ended_at.as_deref(), Some("2026-09-03T00:01:00Z"));
    }

    #[test]
    fn skill_reads_are_detected() {
        let (_d, p) = write(vec![json!({"type":"response_item","payload":{
            "type":"custom_tool_call","name":"exec",
            "input":"sed -n '1,200p' .agents/skills/script/SKILL.md; cat .agents/skills/director/SKILL.md"}})]);
        let r = parse(&p).unwrap();
        assert_eq!(r.skills_read, vec!["director", "script"]);
    }

    /// 重写这个项目就是为了让这种事不再发生，报告里必须点名。
    #[test]
    fn bypassing_mcp_is_called_out() {
        let (_d, p) = write(vec![
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec",
                "input":"python -c 'con.execute(\"UPDATE questions SET status=1\")'"}}),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec",
                "input":"sqlite3 .studio/studio.db 'select * from stages'"}}),
        ]);
        let r = parse(&p).unwrap();
        assert_eq!(r.bypasses.len(), 2);
        assert!(r.bypasses.iter().any(|b| b.contains("SQL UPDATE")));
        assert!(r.bypasses.iter().any(|b| b.contains("状态库")));
    }

    #[test]
    fn a_clean_session_reports_no_bypass() {
        let (_d, p) = write(vec![json!({"type":"response_item","payload":{
            "type":"custom_tool_call","name":"exec",
            "input":"tools.mcp__video_studio__studio.submit_stage({outputs: {...}})"}})]);
        let r = parse(&p).unwrap();
        assert!(r.bypasses.is_empty());
        assert_eq!(r.calls.studio_mcp, 1);
    }

    #[test]
    fn message_and_reasoning_counts() {
        let (_d, p) = write(vec![
            json!({"type":"response_item","payload":{"type":"message","role":"user"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant"}}),
            json!({"type":"response_item","payload":{"type":"reasoning"}}),
        ]);
        let r = parse(&p).unwrap();
        assert_eq!(
            (r.user_messages, r.assistant_messages, r.reasoning_blocks),
            (1, 2, 1)
        );
    }

    #[test]
    fn a_malformed_line_does_not_abort_the_parse() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("r.jsonl");
        std::fs::write(
            &p,
            "{ 坏行\n{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\"}}\n",
        )
        .unwrap();
        assert_eq!(parse(&p).unwrap().reasoning_blocks, 1);
    }
}
