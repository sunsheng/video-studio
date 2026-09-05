//! 直连 LLM API 驱动：不依赖 Codex CLI 是否装配，`OPENAI_API_KEY`/
//! `OPENAI_BASE_URL` 存在即可跑，覆盖"本机没配 Codex"的情况。
//!
//! 跟 [`super::codex::CodexDriver`] 角色相反：这里我们自己的代码就是
//! MCP 客户端，直接拿 [`Harness`] 发调用；工具面从 `studio_mcp::TOOLS`
//! 转成 OpenAI 兼容的 `tools`/`tool_calls` 协议，自己跑一个标准
//! tool-use 循环。
//!
//! **已知的简化**：系统提示词是 AGENTS.md + **全部** SKILL.md 原文一次性
//! 塞进去，不是真实 Agent 那种"按阶段渐进披露、用到 doctrine 才读"。
//! 这条驱动的定位是覆盖手段（本机没有 Codex 时也能跑一版信号），不是
//! 对生产行为的高保真复现——高保真那条路是 `CodexDriver`。
//!
//! 这条驱动天然读不到 Codex 的会话记录，`DriverRun.rollout` 恒为
//! `None`，报告里这几列标"不可观测"，不强行伪造。

use super::{read_decisions, read_gate, AgentDriver, AgentScenario, DriverRun};
use crate::harness::Harness;
use crate::user_sim::{GateState, UserSim};
use serde_json::{json, Value};
use std::time::Duration;

pub struct DirectLlmDriver {
    base_url: String,
    api_key: String,
    model: String,
    max_turns: usize,
}

impl DirectLlmDriver {
    /// 从 `OPENAI_API_KEY`/`OPENAI_BASE_URL` 环境变量构造——跟
    /// CLAUDE.md「本地配置 Codex」一节用的是同一套约定。缺一个都返回
    /// `Err`，不假装能跑。
    pub fn from_env(model: impl Into<String>) -> Result<DirectLlmDriver, String> {
        let base_url =
            std::env::var("OPENAI_BASE_URL").map_err(|_| "没有配置 OPENAI_BASE_URL".to_string())?;
        let api_key =
            std::env::var("OPENAI_API_KEY").map_err(|_| "没有配置 OPENAI_API_KEY".to_string())?;
        Ok(DirectLlmDriver {
            base_url,
            api_key,
            model: model.into(),
            max_turns: 24,
        })
    }

    fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({ "model": self.model, "messages": messages, "tools": tools });
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(120))
            .send_json(body)
            .map_err(|e| format!("调 {url} 失败：{e}"))?;
        let v: Value = resp
            .into_json()
            .map_err(|e| format!("响应不是合法 JSON：{e}"))?;
        if v["choices"][0].is_null() {
            return Err(format!("响应里没有 choices[0]：{v}"));
        }
        Ok(v["choices"][0]["message"].clone())
    }
}

/// OpenAI 兼容 Chat Completions 的 function name 只认 `[a-zA-Z0-9_-]+`，
/// MCP 工具名里的 `.`（`studio.status`、`studio.comfy.exclude_node`）会
/// 被直接拒收——把点换成下划线，调用时再用 [`resolve_tool_name`] 换回去。
fn sanitize_tool_name(name: &str) -> String {
    name.replace('.', "_")
}

/// 把 OpenAI 返回的（消毒过的）工具名换回真正的 MCP 工具名。换不回去
/// 就原样返回——那种情况下 `Harness::call` 会自己报"没有这个工具"，
/// 不在这里假装知道该调用什么。
fn resolve_tool_name(sanitized: &str) -> String {
    studio_mcp::TOOLS
        .iter()
        .find(|t| sanitize_tool_name(t.name) == sanitized)
        .map(|t| t.name.to_string())
        .unwrap_or_else(|| sanitized.to_string())
}

fn openai_tools() -> Vec<Value> {
    studio_mcp::TOOLS
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": sanitize_tool_name(t.name),
                    "description": t.description,
                    "parameters": (t.input_schema)()
                }
            })
        })
        .collect()
}

/// AGENTS.md + 全部 SKILL.md 原文，跟 `driver::codex::copy_generated_docs_into`
/// 读的是同一份已生成产物，理由一样：生成逻辑只该活在 `studio-cli`。
fn system_prompt() -> Result<String, String> {
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut out = std::fs::read_to_string(assets.join("AGENTS.md"))
        .map_err(|e| format!("读 assets/AGENTS.md 失败：{e}"))?;
    let skills_dir = assets.join("skills");
    let mut names: Vec<_> = std::fs::read_dir(&skills_dir)
        .map_err(|e| format!("读 assets/skills 失败：{e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    names.sort();
    for dir in names {
        let path = dir.join("SKILL.md");
        if path.is_file() {
            out.push_str("\n\n");
            out.push_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?);
        }
    }
    Ok(out)
}

impl AgentDriver for DirectLlmDriver {
    fn run(
        &mut self,
        scenario: &AgentScenario,
        user: &mut dyn UserSim,
    ) -> Result<DriverRun, String> {
        let mut h = Harness::fresh();
        let tools = openai_tools();
        let mut messages = vec![
            json!({"role": "system", "content": system_prompt()?}),
            json!({"role": "user", "content": scenario.brief}),
        ];
        let mut turns = 0usize;
        let mut reached_stage = None;

        for _ in 0..self.max_turns {
            turns += 1;
            let message = self.complete(&messages, &tools)?;
            let tool_calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            messages.push(message);

            if tool_calls.is_empty() {
                // 模型直接说话，没调工具——多半是在等用户回应。
                let (stage, pending) = read_gate(&mut h)?;
                reached_stage = Some(stage);
                if pending.is_none() && stage == scenario.expected_stage {
                    break;
                }
                let reply = user.reply(&GateState {
                    stage,
                    pending_question: pending.as_ref(),
                });
                messages.push(json!({"role": "user", "content": reply}));
                continue;
            }

            for call in &tool_calls {
                let sanitized = call["function"]["name"].as_str().unwrap_or_default();
                let name = resolve_tool_name(sanitized);
                let args: Value = call["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_else(|| json!({}));
                let (result, _err) = h.call(&name, args);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call["id"],
                    "content": result.to_string(),
                }));
            }

            let (stage, pending) = read_gate(&mut h)?;
            reached_stage = Some(stage);
            if pending.is_none() && stage == scenario.expected_stage {
                break;
            }
        }

        let decisions = read_decisions(&mut h);
        Ok(DriverRun {
            trace: h.trace(),
            bundle_root: h.root.clone(),
            reached_stage,
            turns,
            decisions,
            rollout: None,
            _dir: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// 起一个只应答一次的假 OpenAI 兼容端点，验证请求体形状和响应解析——
    /// 不打真实网络，`OPENAI_API_KEY`/`OPENAI_BASE_URL` 也不需要配。
    fn fake_endpoint(reply_content: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            std::io::Read::read_exact(&mut reader, &mut body).unwrap();
            let body_text = String::from_utf8(body).unwrap();

            let resp_body = json!({
                "choices": [{"message": {"role": "assistant", "content": reply_content}}]
            })
            .to_string();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                resp_body.len(),
                resp_body
            )
            .unwrap();
            body_text
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    #[test]
    fn complete_sends_model_and_tools_and_parses_the_message() {
        let (url, handle) = fake_endpoint("你好");
        let d = DirectLlmDriver {
            base_url: url,
            api_key: "test-key".into(),
            model: "test-model".into(),
            max_turns: 1,
        };
        let msg = d
            .complete(&[json!({"role": "user", "content": "hi"})], &openai_tools())
            .unwrap();
        assert_eq!(msg["content"], "你好");

        let sent = handle.join().unwrap();
        let sent: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(sent["model"], "test-model");
        assert!(
            sent["tools"].as_array().unwrap().len() >= 10,
            "工具面应该带上全部 studio.* 工具"
        );
    }

    #[test]
    fn openai_tools_cover_every_registered_tool() {
        let tools = openai_tools();
        assert_eq!(tools.len(), studio_mcp::TOOLS.len());
        assert!(tools
            .iter()
            .all(|t| t["type"] == "function" && t["function"]["name"].is_string()));
    }

    /// OpenAI 兼容 Chat Completions 的 function name 只认
    /// `[a-zA-Z0-9_-]+`——MCP 工具名里的点必须先换掉，换完还要能唯一地
    /// 换回来，调用真正的工具时不能认错。
    #[test]
    fn sanitized_names_are_openai_legal_and_round_trip_uniquely() {
        let mut seen = std::collections::HashSet::new();
        for t in studio_mcp::TOOLS.iter() {
            let sanitized = sanitize_tool_name(t.name);
            assert!(
                sanitized
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{sanitized} 不满足 OpenAI function name 的字符集要求"
            );
            assert!(
                seen.insert(sanitized.clone()),
                "{sanitized} 跟别的工具名消毒后撞了"
            );
            assert_eq!(resolve_tool_name(&sanitized), t.name);
        }
    }
}
