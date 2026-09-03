//! stdio 上的 MCP 服务端。
//!
//! 一个进程只服务一部作品——启动时打开当前目录的 bundle 并独占它。
//! 这就是为什么工具面上没有 `run_id`。

use crate::tools::{tool_list, TOOLS};
use crate::trace::{now, Trace, TraceRecord};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use studio_core::{Confirmation, Outputs, StageId, StudioError};
use studio_engine::Project;

/// 我们认得的协议修订，新的在前。
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct Server {
    project: Option<Project>,
    /// 打不开项目时保留原因，让每次工具调用都能给出同样清楚的解释。
    open_error: Option<StudioError>,
    root: PathBuf,
}

impl Server {
    pub fn new(cwd: &Path, program_dir: Option<&Path>) -> Server {
        match Project::open(cwd, program_dir) {
            Ok(p) => {
                let root = p.bundle().root().to_path_buf();
                Server { project: Some(p), open_error: None, root }
            }
            Err(e) => Server { project: None, open_error: Some(e), root: cwd.to_path_buf() },
        }
    }

    /// 跑 stdio 消息循环直到对端关闭。
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(response) = self.handle_line(line) else { continue };
            writeln!(output, "{response}")?;
            output.flush()?;
        }
        Ok(())
    }

    /// 处理一条 JSON-RPC 消息。通知（无 id）返回 None。
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(
                    error_response(&Value::Null, -32700, &format!("JSON 解析失败：{e}")).to_string(),
                )
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        // 通知没有 id，不需要回应。
        let id = id?;

        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "tools/list" => Ok(tool_list()),
            "ping" => Ok(json!({})),
            "tools/call" => return Some(self.call_tool(&id, &params).to_string()),
            other => Err((-32601, format!("未知方法：{other}"))),
        };

        Some(match result {
            Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }).to_string(),
            Err((code, msg)) => error_response(&id, code, &msg).to_string(),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
        let version = if SUPPORTED_PROTOCOLS.contains(&requested) { requested } else { SUPPORTED_PROTOCOLS[0] };
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "video-studio", "version": env!("CARGO_PKG_VERSION") }
        })
    }

    fn call_tool(&mut self, id: &Value, params: &Value) -> Value {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        if !TOOLS.iter().any(|t| t.name == name) {
            return error_response(id, -32602, &format!("未知工具：{name}。可用工具见 tools/list。"));
        }

        let started = std::time::Instant::now();
        let outcome = self.dispatch(&name, &args);
        let elapsed = started.elapsed().as_millis() as u64;

        let (payload, is_error, rec) = match outcome {
            Ok(v) => {
                let stage = v.get("project").and_then(|p| p.get("stage")).and_then(|s| s.as_str()).map(String::from);
                let waiting = v.get("waiting_on").and_then(|s| s.as_str()).map(String::from);
                let rec = TraceRecord {
                    at: now(),
                    tool: name.clone(),
                    stage,
                    ok: true,
                    error_code: None,
                    remedy_present: None,
                    waiting_on: waiting,
                    duration_ms: elapsed,
                };
                (v, false, rec)
            }
            Err(e) => {
                // 出错也返回信封，让 blocked_by.remedy 一定在。
                let envelope = self
                    .project
                    .as_ref()
                    .map(|p| serde_json::to_value(p.envelope_for_error(&e)).unwrap_or(Value::Null));
                let remedy = e.remedy();
                let body = match envelope {
                    Some(env) if env != Value::Null => env,
                    _ => json!({
                        "blocked_by": { "code": e.code(), "message": e.message(), "remedy": remedy },
                        "waiting_on": "agent"
                    }),
                };
                let rec = TraceRecord {
                    at: now(),
                    tool: name.clone(),
                    stage: None,
                    ok: false,
                    error_code: Some(e.code().to_string()),
                    remedy_present: Some(!e.remedy().trim().is_empty()),
                    waiting_on: None,
                    duration_ms: elapsed,
                };
                (body, true, rec)
            }
        };

        Trace::at(&self.root).append(&rec);

        let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": text }],
                "isError": is_error,
                "structuredContent": payload
            }
        })
    }

    fn project(&self) -> studio_core::Result<&Project> {
        match (&self.project, &self.open_error) {
            (Some(p), _) => Ok(p),
            (None, Some(e)) => Err(e.clone()),
            (None, None) => Err(StudioError::NotAProject { path: self.root.display().to_string() }),
        }
    }

    fn dispatch(&self, name: &str, args: &Value) -> studio_core::Result<Value> {
        let p = self.project()?;
        match name {
            "studio.status" => to_value(p.status()?),
            "studio.schema" => Ok(p.schema_of(stage_arg(args)?)),
            "studio.submit_stage" => {
                let outputs: Outputs = match args.get("outputs") {
                    Some(Value::Object(m)) => m.clone(),
                    _ => {
                        return Err(StudioError::SchemaViolation {
                            stage: p.current_stage()?.unwrap_or(StageId::Idea),
                            violations: vec![studio_core::Violation::new("outputs", "必须是一个对象")],
                        })
                    }
                };
                let summary = args.get("summary").and_then(|v| v.as_str());
                let confirmation: Option<Confirmation> = match args.get("confirmation") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| StudioError::SchemaViolation {
                        stage: p.current_stage().ok().flatten().unwrap_or(StageId::Idea),
                        violations: vec![studio_core::Violation::new("confirmation", e.to_string())],
                    })?),
                };
                to_value(p.submit_stage(outputs, summary, confirmation)?)
            }
            "studio.answer" => {
                let qid = str_arg(args, "question_id")?;
                let ans = str_arg(args, "answer")?;
                to_value(p.answer(&qid, &ans)?)
            }
            "studio.revise" => {
                let stage = stage_arg(args)?;
                let msg = str_arg(args, "message")?;
                to_value(p.revise(stage, &msg)?)
            }
            "studio.undo" => to_value(p.undo()?),
            "studio.stage_output" => p.stage_output(stage_arg(args)?),
            "studio.timeline" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                to_value(p.timeline(limit)?)
            }
            "studio.export" => to_value(p.export()?),
            other => Err(StudioError::internal(format!("工具 {other} 未接线"))),
        }
    }
}

fn to_value<T: serde::Serialize>(v: T) -> studio_core::Result<Value> {
    serde_json::to_value(v).map_err(|e| StudioError::internal(e.to_string()))
}

fn str_arg(args: &Value, key: &str) -> studio_core::Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| StudioError::internal(format!("缺少必填参数 {key}")))
}

fn stage_arg(args: &Value) -> studio_core::Result<StageId> {
    let s = str_arg(args, "stage")?;
    StageId::parse(&s).ok_or_else(|| {
        StudioError::internal(format!(
            "未知阶段 {s}。合法取值：{}",
            StageId::all().map(|x| x.as_str()).collect::<Vec<_>>().join(" / ")
        ))
    })
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
