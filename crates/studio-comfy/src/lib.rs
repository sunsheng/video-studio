//! ComfyUI 客户端。
//!
//! **运行本程序的机器不需要 GPU。** 控制面与推理面之间只有 HTTP：
//! 健康检查、选节点、上传输入、提交 workflow、轮询 `/history/{prompt_id}`、
//! 下载输出。模型权重、custom node、CUDA 全在 ComfyUI 那一侧。
//!
//! 因此控制面可以跑在一台没有显卡的小机器上，甚至和 ComfyUI 不在同一台主机——
//! 节点地址在 `.env` 的 `COMFY_NODES` 里配。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};
use studio_core::{Result, StudioError};

/// 一个节点的健康状况。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub url: String,
    pub reachable: bool,
    /// 队列里排着的任务数（运行中 + 等待中）。选节点时越小越优先。
    pub queue_depth: usize,
    pub detail: Option<String>,
}

/// 一次提交的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    pub node: String,
    pub prompt_id: String,
}

/// 产出的一个文件在 ComfyUI 侧的位置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFile {
    pub filename: String,
    #[serde(default)]
    pub subfolder: String,
    #[serde(default = "default_type")]
    pub r#type: String,
}

fn default_type() -> String {
    "output".to_string()
}

pub struct Comfy {
    nodes: Vec<String>,
    timeout: Duration,
    poll: Duration,
}

impl Comfy {
    pub fn new(nodes: Vec<String>, timeout_secs: u64, poll_secs: u64) -> Comfy {
        Comfy {
            nodes,
            timeout: Duration::from_secs(timeout_secs.max(1)),
            poll: Duration::from_secs(poll_secs.clamp(1, 60)),
        }
    }

    pub fn from_settings(s: &studio_engine::Settings) -> Comfy {
        Comfy::new(s.comfy_nodes(), s.comfy_timeout_secs(), s.comfy_poll_secs())
    }

    pub fn nodes(&self) -> &[String] {
        &self.nodes
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(30))
            .build()
    }

    /// 逐个探活。不可达的节点不是错误，只是不可用。
    pub fn health(&self) -> Vec<NodeHealth> {
        let agent = self.agent();
        self.nodes
            .iter()
            .map(|url| match agent.get(&format!("{url}/queue")).call() {
                Ok(resp) => match resp.into_json::<Value>() {
                    Ok(v) => NodeHealth {
                        url: url.clone(),
                        reachable: true,
                        queue_depth: queue_depth(&v),
                        detail: None,
                    },
                    Err(e) => NodeHealth {
                        url: url.clone(),
                        reachable: false,
                        queue_depth: usize::MAX,
                        detail: Some(format!("返回不是 JSON：{e}")),
                    },
                },
                Err(e) => NodeHealth {
                    url: url.clone(),
                    reachable: false,
                    queue_depth: usize::MAX,
                    detail: Some(short_error(&e)),
                },
            })
            .collect()
    }

    /// 选一个健康且队列最短的节点。一个都没有就结构化阻塞，**不降级**。
    pub fn pick_node(&self) -> Result<String> {
        let mut healthy: Vec<NodeHealth> =
            self.health().into_iter().filter(|h| h.reachable).collect();
        healthy.sort_by_key(|h| h.queue_depth);
        healthy
            .first()
            .map(|h| h.url.clone())
            .ok_or_else(|| StudioError::ComfyUnavailable {
                tried: self.nodes.clone(),
            })
    }

    /// 提交一张 API 格式的节点图。
    pub fn submit(&self, node: &str, api_graph: &Value, client_id: &str) -> Result<Submission> {
        let body = serde_json::json!({ "prompt": api_graph, "client_id": client_id });
        let resp = self
            .agent()
            .post(&format!("{node}/prompt"))
            .send_json(body)
            .map_err(|e| StudioError::ComfyFailed {
                node: node.into(),
                detail: short_error(&e),
            })?;
        let v: Value = resp.into_json().map_err(|e| StudioError::ComfyFailed {
            node: node.into(),
            detail: format!("提交返回不是 JSON：{e}"),
        })?;
        let prompt_id = v["prompt_id"]
            .as_str()
            .ok_or_else(|| StudioError::ComfyFailed {
                node: node.into(),
                detail: format!("提交返回里没有 prompt_id：{v}"),
            })?;
        Ok(Submission {
            node: node.to_string(),
            prompt_id: prompt_id.to_string(),
        })
    }

    /// 轮询直到出结果或超时。返回该次执行产出的文件清单。
    pub fn wait(&self, sub: &Submission) -> Result<Vec<RemoteFile>> {
        let started = Instant::now();
        loop {
            if let Some(files) = self.try_history(sub)? {
                return Ok(files);
            }
            if started.elapsed() > self.timeout {
                return Err(StudioError::ComfyFailed {
                    node: sub.node.clone(),
                    detail: format!(
                        "等待 {} 超过 {} 秒仍无结果",
                        sub.prompt_id,
                        self.timeout.as_secs()
                    ),
                });
            }
            std::thread::sleep(self.poll);
        }
    }

    /// 查一次历史。还没跑完返回 `Ok(None)`。
    pub fn try_history(&self, sub: &Submission) -> Result<Option<Vec<RemoteFile>>> {
        let url = format!("{}/history/{}", sub.node, sub.prompt_id);
        let resp = self
            .agent()
            .get(&url)
            .call()
            .map_err(|e| StudioError::ComfyFailed {
                node: sub.node.clone(),
                detail: short_error(&e),
            })?;
        let v: Value = resp.into_json().map_err(|e| StudioError::ComfyFailed {
            node: sub.node.clone(),
            detail: format!("历史返回不是 JSON：{e}"),
        })?;

        let Some(entry) = v.get(&sub.prompt_id) else {
            return Ok(None);
        };

        if let Some(status) = entry.get("status") {
            if status.get("status_str").and_then(|s| s.as_str()) == Some("error") {
                return Err(StudioError::ComfyFailed {
                    node: sub.node.clone(),
                    detail: extract_error(status),
                });
            }
            let completed = status
                .get("completed")
                .and_then(|c| c.as_bool())
                .unwrap_or(false);
            if !completed && entry.get("outputs").is_none() {
                return Ok(None);
            }
        }

        let files = collect_files(entry.get("outputs").unwrap_or(&Value::Null));
        if files.is_empty() {
            return Ok(None);
        }
        Ok(Some(files))
    }

    /// 把产出下载到本地。
    pub fn download(&self, node: &str, file: &RemoteFile, dest: &std::path::Path) -> Result<u64> {
        let url = format!(
            "{node}/view?filename={}&subfolder={}&type={}",
            urlencode(&file.filename),
            urlencode(&file.subfolder),
            urlencode(&file.r#type)
        );
        let resp = self
            .agent()
            .get(&url)
            .call()
            .map_err(|e| StudioError::ComfyFailed {
                node: node.into(),
                detail: short_error(&e),
            })?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StudioError::internal(format!("建目录失败：{e}")))?;
        }
        let mut reader = resp.into_reader();
        let mut out = std::fs::File::create(dest)
            .map_err(|e| StudioError::internal(format!("创建 {} 失败：{e}", dest.display())))?;
        std::io::copy(&mut reader, &mut out)
            .map_err(|e| StudioError::internal(format!("下载失败：{e}")))
    }

    /// 上传一张参考图作为 workflow 的输入。
    pub fn upload_image(&self, node: &str, name: &str, bytes: &[u8]) -> Result<String> {
        let boundary = "----videostudioboundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"image\"; filename=\"{name}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let resp = self
            .agent()
            .post(&format!("{node}/upload/image"))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| StudioError::ComfyFailed {
                node: node.into(),
                detail: short_error(&e),
            })?;
        let v: Value = resp.into_json().map_err(|e| StudioError::ComfyFailed {
            node: node.into(),
            detail: format!("上传返回不是 JSON：{e}"),
        })?;
        Ok(v["name"].as_str().unwrap_or(name).to_string())
    }
}

fn queue_depth(v: &Value) -> usize {
    let running = v["queue_running"].as_array().map(|a| a.len()).unwrap_or(0);
    let pending = v["queue_pending"].as_array().map(|a| a.len()).unwrap_or(0);
    running + pending
}

fn collect_files(outputs: &Value) -> Vec<RemoteFile> {
    let mut files = Vec::new();
    let Some(map) = outputs.as_object() else {
        return files;
    };
    for node_out in map.values() {
        let Some(obj) = node_out.as_object() else {
            continue;
        };
        for (key, list) in obj {
            // ComfyUI 按类型分组：images / gifs / videos / audio ...
            if !matches!(
                key.as_str(),
                "images" | "gifs" | "videos" | "audio" | "files"
            ) {
                continue;
            }
            let Some(arr) = list.as_array() else { continue };
            for item in arr {
                if let Ok(f) = serde_json::from_value::<RemoteFile>(item.clone()) {
                    files.push(f);
                }
            }
        }
    }
    files
}

fn extract_error(status: &Value) -> String {
    if let Some(msgs) = status.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            if m.get(0).and_then(|s| s.as_str()) == Some("execution_error") {
                if let Some(detail) = m.get(1) {
                    let node = detail
                        .get("node_type")
                        .and_then(|s| s.as_str())
                        .unwrap_or("未知节点");
                    let msg = detail
                        .get("exception_message")
                        .and_then(|s| s.as_str())
                        .unwrap_or("未提供原因");
                    return format!("{node}: {msg}");
                }
            }
        }
    }
    status
        .get("status_str")
        .and_then(|s| s.as_str())
        .unwrap_or("执行失败")
        .to_string()
}

fn short_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => format!("连接失败：{t}"),
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// 一个只回固定内容的假 ComfyUI，用来在没有 GPU 的机器上验证客户端逻辑。
    struct Stub {
        url: String,
        _handle: std::thread::JoinHandle<()>,
    }

    fn stub(routes: Vec<(&'static str, Value)>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(16) {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = routes
                    .iter()
                    .find(|(p, _)| path.starts_with(p))
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| "{}".to_string());
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        Stub {
            url: format!("http://127.0.0.1:{port}"),
            _handle: handle,
        }
    }

    #[test]
    fn unreachable_nodes_block_instead_of_degrading() {
        // 端口 1 上不会有 ComfyUI。
        let c = Comfy::new(vec!["http://127.0.0.1:1".into()], 1, 1);
        let e = c.pick_node().unwrap_err();
        assert_eq!(e.code(), "comfy_unavailable");
        assert!(e.remedy().contains("COMFY_NODES"));
        assert!(
            e.remedy().contains("不要降级"),
            "缺节点时必须明确禁止换模型：{}",
            e.remedy()
        );
    }

    #[test]
    fn health_reports_queue_depth() {
        let s = stub(vec![(
            "/queue",
            json!({ "queue_running": [1], "queue_pending": [1, 2] }),
        )]);
        let c = Comfy::new(vec![s.url.clone()], 5, 1);
        let h = c.health();
        assert!(h[0].reachable);
        assert_eq!(h[0].queue_depth, 3);
    }

    #[test]
    fn the_shortest_queue_wins() {
        let busy = stub(vec![(
            "/queue",
            json!({ "queue_running": [1], "queue_pending": [1, 2, 3] }),
        )]);
        let idle = stub(vec![(
            "/queue",
            json!({ "queue_running": [], "queue_pending": [] }),
        )]);
        let c = Comfy::new(vec![busy.url.clone(), idle.url.clone()], 5, 1);
        assert_eq!(c.pick_node().unwrap(), idle.url);
    }

    #[test]
    fn submit_returns_the_prompt_id_for_traceability() {
        let s = stub(vec![(
            "/prompt",
            json!({ "prompt_id": "abc-123", "number": 1 }),
        )]);
        let c = Comfy::new(vec![s.url.clone()], 5, 1);
        let sub = c
            .submit(
                &s.url,
                &json!({ "1": { "class_type": "X", "inputs": {} } }),
                "cid",
            )
            .unwrap();
        assert_eq!(sub.prompt_id, "abc-123");
    }

    #[test]
    fn history_collects_output_files() {
        let s = stub(vec![(
            "/history/",
            json!({ "abc-123": {
                "status": { "status_str": "success", "completed": true },
                "outputs": { "9": { "videos": [ { "filename": "sh01.mp4", "subfolder": "", "type": "output" } ] } }
            }}),
        )]);
        let c = Comfy::new(vec![s.url.clone()], 5, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        let files = c.try_history(&sub).unwrap().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "sh01.mp4");
    }

    #[test]
    fn a_still_running_prompt_is_not_an_error() {
        let s = stub(vec![("/history/", json!({}))]);
        let c = Comfy::new(vec![s.url.clone()], 5, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        assert!(c.try_history(&sub).unwrap().is_none());
    }

    #[test]
    fn execution_errors_name_the_node_and_reason() {
        let s = stub(vec![(
            "/history/",
            json!({ "abc-123": { "status": { "status_str": "error", "completed": false,
                "messages": [["execution_error", { "node_type": "KSampler",
                                                   "exception_message": "CUDA out of memory" }]] } }}),
        )]);
        let c = Comfy::new(vec![s.url.clone()], 5, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        let e = c.try_history(&sub).unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(e.message().contains("KSampler"));
        assert!(e.message().contains("CUDA out of memory"));
        assert!(e.remedy().contains("studio.timeline"));
    }

    #[test]
    fn urlencoding_handles_chinese_filenames() {
        assert_eq!(urlencode("a b.mp4"), "a%20b.mp4");
        assert!(urlencode("千岛湖.mp4").starts_with('%'));
    }

    #[test]
    fn output_grouping_covers_images_videos_and_audio() {
        let outputs = json!({
            "1": { "images": [{ "filename": "c01.png" }] },
            "2": { "videos": [{ "filename": "sh01.mp4" }] },
            "3": { "audio":  [{ "filename": "amb.wav" }] },
            "4": { "ui": { "ignored": true } }
        });
        let files = collect_files(&outputs);
        assert_eq!(files.len(), 3);
    }
}
