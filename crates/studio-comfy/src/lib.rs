//! ComfyUI 客户端。
//!
//! **运行本程序的机器不需要 GPU。** 控制面与推理面之间只有 HTTP：
//! 健康检查、上传输入、提交 workflow、轮询 `/history/{prompt_id}`、下载输出。
//! 模型权重、custom node、CUDA 全在 ComfyUI 那一侧。
//!
//! 因此控制面可以跑在一台没有显卡的小机器上，甚至和 ComfyUI 不在同一台主机——
//! 地址在 `.env` 的 `COMFY_NODE` 里配。
//!
//! **入口只有一个 URL。** 多节点的分发与故障转移是那一侧的事（通常是个负载
//! 均衡代理），控制面不再维护节点集合、不再挑节点。需要鉴权的代理在
//! `COMFY_TOKEN` 里配 Bearer token，没配就不带 `Authorization` 头。

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

/// 单次轮询 in a row 允许的最大连续「联系不上节点」次数。
///
/// 超过这个数才当真失败——单次孤立的连接超时（例如 `os error 10060`）
/// 不该打断一个实际还在正常跑着的渲染。总耗时超过 `timeout` 仍是兜底上限。
const MAX_CONSECUTIVE_UNREACHABLE: u32 = 5;

/// 控制面调用的读超时：`/prompt`、`/history`、`/queue`、`/object_info`。
/// 这些都是小 JSON，30 秒绰绰有余，短一点能早点发现节点没反应。
const CONTROL_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 大块传输的读超时：上传参考图、下载产物。
///
/// **这个值不能跟控制面共用。** 几 MB 的多段上传和一个几百字节的 JSON 请求，
/// 耗时不在一个量级上，凭什么共用一个上限。
///
/// 触发这次改动的是 2026-09-05 的一轮观测：一张 1.09 MB 的卡片图经当时那条
/// 代理传上去要 25–38 秒，而上传走的正是控制面那 30 秒，于是稳定地时好时坏。
/// **那几个数字不能当成干净的基线**——事后得知那条代理当时正有已知故障，
/// 数字多半被它撑大了（同一张图在故障期外量到过 14.5 秒）。
///
/// 但结论不依赖具体数字，依赖的是失败形态：上传超时不表现为「上传失败」，
/// 而是锚点视图图出来了、落盘了、状态是 ready，只是传不回 ComfyUI，于是
/// **后面每一个派生视图都报「参考图还没生成出来」**——一句与事实相反的话，
/// 最难查的那种。这种失败不该由一个压得很紧的超时值来决定发不发生。
///
/// 取 5 分钟：够 1 MB 图慢一个数量级、够几十 MB 的成片下载，又不至于让一条
/// 真死掉的连接挂到天亮。
const BULK_READ_TIMEOUT: Duration = Duration::from_secs(300);

/// 提交失败后的退避间隔。长度就是最多提交几次。
///
/// 这是给「刚好撞上一次调度抖动」留的余地——真正的长时间排队由
/// [`Comfy::submit_timeout`] 覆盖，代理会替我们等，不需要客户端在这里死磕。
const SUBMIT_RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

pub struct Comfy {
    node: String,
    /// 代理的 Bearer token。None 表示对端不需要鉴权。
    token: Option<String>,
    timeout: Duration,
    poll: Duration,
}

/// 轮询一次 `/history` 的结果。
enum PollOutcome {
    /// 连上了节点，但这次提交还没跑完。
    Running,
    /// 跑完了，带着产出文件。
    Done(Vec<RemoteFile>),
    /// 连接层错误：节点这一次联系不上，可能只是网络抖动，不代表渲染失败。
    Unreachable(String),
    /// ComfyUI 返回的结构化失败，或响应本身不成形——视为真实失败，立即报错。
    Failed(StudioError),
}

impl Comfy {
    pub fn new(node: String, token: Option<String>, timeout_secs: u64, poll_secs: u64) -> Comfy {
        Comfy {
            node: node.trim_end_matches('/').to_string(),
            token,
            timeout: Duration::from_secs(timeout_secs.max(1)),
            poll: Duration::from_secs(poll_secs.clamp(1, 60)),
        }
    }

    pub fn from_settings(s: &studio_engine::Settings) -> Comfy {
        Comfy::new(
            s.comfy_node(),
            s.comfy_token(),
            s.comfy_timeout_secs(),
            s.comfy_poll_secs(),
        )
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    fn agent(&self) -> ureq::Agent {
        self.agent_with(CONTROL_READ_TIMEOUT)
    }

    fn agent_with(&self, read: Duration) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(read)
            .build()
    }

    /// 带上鉴权头。token 没配就原样返回——直连一个不设防的 ComfyUI 时
    /// 本来就不需要这个头。
    fn auth(&self, req: ureq::Request) -> ureq::Request {
        match &self.token {
            Some(t) => req.set("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    fn get(&self, url: &str) -> ureq::Request {
        self.auth(self.agent().get(url))
    }

    /// 大块传输专用的 GET：下载产物。读超时按 [`BULK_READ_TIMEOUT`]。
    fn bulk_get(&self, url: &str) -> ureq::Request {
        self.auth(self.agent_with(BULK_READ_TIMEOUT).get(url))
    }

    /// 大块传输专用的 POST：上传参考图。读超时按 [`BULK_READ_TIMEOUT`]。
    fn bulk_post(&self, url: &str) -> ureq::Request {
        self.auth(self.agent_with(BULK_READ_TIMEOUT).post(url))
    }

    /// 探活。不可达不是错误，只是不可用。
    pub fn health(&self) -> NodeHealth {
        match self.get(&format!("{}/queue", self.node)).call() {
            Ok(resp) => match resp.into_json::<Value>() {
                Ok(v) => NodeHealth {
                    url: self.node.clone(),
                    reachable: true,
                    queue_depth: queue_depth(&v),
                    detail: None,
                },
                Err(e) => NodeHealth {
                    url: self.node.clone(),
                    reachable: false,
                    queue_depth: usize::MAX,
                    detail: Some(format!("返回不是 JSON：{e}")),
                },
            },
            Err(e) => NodeHealth {
                url: self.node.clone(),
                reachable: false,
                queue_depth: usize::MAX,
                detail: Some(short_error(e)),
            },
        }
    }

    /// 确认入口活着。不活就结构化阻塞，**不降级**。
    ///
    /// 单入口之后这里不再有「挑一个」的余地：能连上就用它，连不上就停。
    /// 后端有几个节点、坏了哪个、怎么转移，都是代理那一侧的事。
    pub fn ensure_reachable(&self) -> Result<()> {
        let h = self.health();
        if h.reachable {
            return Ok(());
        }
        Err(StudioError::ComfyUnavailable {
            tried: vec![self.node.clone()],
        })
    }

    /// 提交用的读超时。
    ///
    /// **提交跟其他控制面调用不是一回事。** `/queue`、`/history` 是即时返回的
    /// 小 JSON，而 `/prompt` 那一侧是个负载均衡代理：所有节点都到了
    /// `MAX_CONCURRENT_PER_NODE` 时，它会**一直排队等节点空出来，直到调用方
    /// 自己的 HTTP 客户端超时或取消**。八台节点、默认队列深度 16，排队是常态。
    ///
    /// 拿控制面那 30 秒去接排队，等于把「排了 31 秒」报成「渲染失败」。
    ///
    /// 取 `COMFY_TIMEOUT_SECS` 的一半：那个值本来就是「这一镜我愿意等多久」，
    /// 排队占掉一半还没轮上，等下去也没意义了。夹在 [60, 600] 是为了不让一个
    /// 手滑写小的配置把提交变成必失败。
    fn submit_timeout(&self) -> Duration {
        Duration::from_secs((self.timeout.as_secs() / 2).clamp(60, 600))
    }

    /// 提交一张 API 格式的节点图。
    ///
    /// `503` 会指数退避重试（[`SUBMIT_RETRY_BACKOFF`]）——按代理的约定那是
    /// 临时的基础设施状况，不是这一镜的内容有问题。`400`/`401`/`404` 立即
    /// 失败，重试没有意义。
    pub fn submit(&self, api_graph: &Value, client_id: &str) -> Result<Submission> {
        let body = serde_json::json!({ "prompt": api_graph, "client_id": client_id });
        let url = format!("{}/prompt", self.node);
        let mut last: Option<Failure> = None;

        let resp = 'attempt: {
            for (i, backoff) in SUBMIT_RETRY_BACKOFF.iter().enumerate() {
                let req = self.auth(self.agent_with(self.submit_timeout()).post(&url));
                match req.send_json(body.clone()) {
                    Ok(r) => break 'attempt r,
                    Err(e) => match classify(e) {
                        // 请求本身有问题，再提交一百次也是同一个结果。
                        f @ Failure::Fatal(_) => return Err(self.submit_error(f)),
                        f @ Failure::Retryable(_) => {
                            last = Some(f);
                            // 最后一次失败之后不必再睡。
                            if i + 1 < SUBMIT_RETRY_BACKOFF.len() {
                                std::thread::sleep(*backoff);
                            }
                        }
                    },
                }
            }
            return Err(self.submit_error(last.expect("重试循环至少跑过一次")));
        };

        let v: Value = resp.into_json().map_err(|e| StudioError::ComfyFailed {
            node: self.node.clone(),
            detail: format!("提交返回不是 JSON：{e}"),
        })?;
        let prompt_id = v["prompt_id"]
            .as_str()
            .ok_or_else(|| StudioError::ComfyFailed {
                node: self.node.clone(),
                detail: format!("提交返回里没有 prompt_id：{v}"),
            })?;
        Ok(Submission {
            node: self.node.clone(),
            prompt_id: prompt_id.to_string(),
        })
    }

    /// 提交最终失败时报哪个错。
    ///
    /// **`no healthy node` 和「节点都在忙」是两件事，remedy 指的方向也不同：**
    /// 前者是集群不可用（去配置、去恢复节点），后者是这一次执行没成
    /// （`retry_stage` 重来一遍就好）。混成一个，人就会去查错的地方。
    fn submit_error(&self, f: Failure) -> StudioError {
        if f.is_no_healthy_node() {
            return StudioError::ComfyUnavailable {
                tried: vec![format!("{}（{}）", self.node, f.detail())],
            };
        }
        StudioError::ComfyFailed {
            node: self.node.clone(),
            detail: f.detail().to_string(),
        }
    }

    /// 轮询直到出结果或超时。返回该次执行产出的文件清单。
    ///
    /// 连接层错误（网络抖动、孤立的连接超时）不会立即失败——只有连续失败
    /// 超过 [`MAX_CONSECUTIVE_UNREACHABLE`] 次，或总耗时超过 `timeout`，
    /// 才真正判定这次渲染失败。ComfyUI 自己报的结构化错误
    /// （`status_str == "error"`）不受此宽限，立即失败。
    pub fn wait(&self, sub: &Submission) -> Result<Vec<RemoteFile>> {
        let started = Instant::now();
        let mut consecutive_unreachable: u32 = 0;
        loop {
            match self.poll(sub) {
                PollOutcome::Done(files) => return Ok(files),
                PollOutcome::Failed(e) => return Err(e),
                PollOutcome::Running => {
                    consecutive_unreachable = 0;
                }
                PollOutcome::Unreachable(detail) => {
                    consecutive_unreachable += 1;
                    if consecutive_unreachable > MAX_CONSECUTIVE_UNREACHABLE {
                        return Err(StudioError::ComfyFailed {
                            node: sub.node.clone(),
                            detail: format!(
                                "连续 {consecutive_unreachable} 次轮询都联系不上节点\
                                 （最近一次：{detail}）"
                            ),
                        });
                    }
                }
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

    /// 查一次历史。还没跑完，或者暂时联系不上节点，都返回 `Ok(None)`——
    /// 「查不到结果」不等于「失败了」。只有 ComfyUI 自己报的结构化错误
    /// 才在这里就返回 `Err`。反复联系不上的容错阈值只在 [`Comfy::wait`] 里生效。
    pub fn try_history(&self, sub: &Submission) -> Result<Option<Vec<RemoteFile>>> {
        match self.poll(sub) {
            PollOutcome::Running | PollOutcome::Unreachable(_) => Ok(None),
            PollOutcome::Done(files) => Ok(Some(files)),
            PollOutcome::Failed(e) => Err(e),
        }
    }

    fn poll(&self, sub: &Submission) -> PollOutcome {
        let url = format!("{}/history/{}", self.node, sub.prompt_id);
        let resp = match self.get(&url).call() {
            Ok(r) => r,
            // **`Fatal` 不进宽限期。** 「路径写错了」「token 不对」再轮询五轮
            // 也是同一个结果，而把它们记成「联系不上节点」会把排查引向网络。
            Err(e) => {
                return match classify(e) {
                    Failure::Retryable(d) => PollOutcome::Unreachable(d),
                    Failure::Fatal(d) => PollOutcome::Failed(StudioError::ComfyFailed {
                        node: sub.node.clone(),
                        detail: format!("查历史被拒（{d}），重试没有意义"),
                    }),
                };
            }
        };
        let v: Value = match resp.into_json() {
            Ok(v) => v,
            Err(e) => return PollOutcome::Unreachable(format!("历史返回不是 JSON：{e}")),
        };

        let Some(entry) = v.get(&sub.prompt_id) else {
            return PollOutcome::Running;
        };

        if let Some(status) = entry.get("status") {
            if status.get("status_str").and_then(|s| s.as_str()) == Some("error") {
                return PollOutcome::Failed(StudioError::ComfyFailed {
                    node: sub.node.clone(),
                    detail: extract_error(status),
                });
            }
            let completed = status
                .get("completed")
                .and_then(|c| c.as_bool())
                .unwrap_or(false);
            if !completed && entry.get("outputs").is_none() {
                return PollOutcome::Running;
            }
        }

        let files = collect_files(entry.get("outputs").unwrap_or(&Value::Null));
        if files.is_empty() {
            PollOutcome::Running
        } else {
            PollOutcome::Done(files)
        }
    }

    /// 把产出下载到本地。
    pub fn download(&self, file: &RemoteFile, dest: &std::path::Path) -> Result<u64> {
        let url = format!(
            "{}/view?filename={}&subfolder={}&type={}",
            self.node,
            urlencode(&file.filename),
            urlencode(&file.subfolder),
            urlencode(&file.r#type)
        );
        let resp = self
            .bulk_get(&url)
            .call()
            .map_err(|e| StudioError::ComfyFailed {
                node: self.node.clone(),
                detail: short_error(e),
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
    pub fn upload_image(&self, name: &str, bytes: &[u8]) -> Result<String> {
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
            .bulk_post(&format!("{}/upload/image", self.node))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| StudioError::ComfyFailed {
                node: self.node.clone(),
                detail: short_error(e),
            })?;
        let v: Value = resp.into_json().map_err(|e| StudioError::ComfyFailed {
            node: self.node.clone(),
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

/// 从 history 的 `outputs` 里挑出**产物**。
///
/// **只认 `type == "output"`。** 加载类节点会把自己的输入原样回显进
/// `outputs`——`LoadVideo` 就是这样，history 里长这个样子：
///
/// ```jsonc
/// "guide1_src_load": { "images": [{ "filename": "anchor.mp4", "type": "input" }] },
/// "save_video":      { "images": [{ "filename": "sh01_00001_.mp4", "type": "output" }] }
/// ```
///
/// 节点 id 是排序遍历的，`guide1_src_load` / `ref1_load` 都排在 `save_video`
/// 前面，而调用方取的是第一个。不过滤的话，**带 clip 锚点或 video 参考的镜头
/// 会把锚点素材当成渲染结果登记下来**——图能跑、有文件、下载得到，一路绿到
/// 交付才看得出不对。
///
/// 缺 `type` 的按 `output` 算（见 [`default_type`]）：老的 fixture 和某些
/// 节点不写这个字段，把它们判成非产物才是新的错。
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
                    if f.r#type == "output" {
                        files.push(f);
                    }
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

/// 一次失败该不该重试。
///
/// 依据是代理的 `docs/usage.md`：`503` 是临时的基础设施状况，应当指数退避
/// 重试；`400` / `401` / `404` 是请求本身有问题，「原样重试没有意义」。
///
/// **把这两类混在一起的代价不是白等几轮**，是把一个必然失败的请求包装成
/// 「联系不上节点」，然后把排查引向网络。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 退避重试还有意义：503、连接层抖动。
    Retryable(String),
    /// 重试没有意义：请求本身、鉴权、路径写错了。
    Fatal(String),
}

impl Failure {
    pub fn detail(&self) -> &str {
        match self {
            Failure::Retryable(d) | Failure::Fatal(d) => d,
        }
    }

    /// 代理报的是「一个健康节点都没有」，而不是「节点都在忙」。
    ///
    /// 两种都要退避重试，区别只在重试到顶之后报什么：前者是集群不可用
    /// （`comfy_unavailable`，remedy 指向配置与恢复），后者是这次执行失败
    /// （`comfy_failed`，remedy 指向 `retry_stage`）。
    pub fn is_no_healthy_node(&self) -> bool {
        self.detail().contains("no healthy node")
    }
}

/// 把 ureq 的错误分类，**并且读出代理写在 body 里的原因**。
///
/// 代理自己生成的错误一律是 `{"error": "<具体原因>"}`。丢掉它就丢掉了区分
/// 两种 503 的唯一依据（`no healthy node` vs `context deadline exceeded`），
/// 报给人的也只剩一句「HTTP 503」，没有 remedy 能指的方向。
///
/// 节点自己返回的错误连状态码带 body 原样透传，不是这个形状——读不出来就
/// 退回 `HTTP {code}`，不猜。
fn classify(e: ureq::Error) -> Failure {
    match e {
        ureq::Error::Status(code, resp) => {
            let reason = resp
                .into_json::<Value>()
                .ok()
                .and_then(|v| v["error"].as_str().map(String::from))
                .map(|r| format!("HTTP {code}：{r}"))
                .unwrap_or_else(|| format!("HTTP {code}"));
            // 未列出的状态码按可重试处理——保守的一侧是多试一次，
            // 不是把一个可能能成的请求判死。
            match code {
                400 | 401 | 403 | 404 => Failure::Fatal(reason),
                _ => Failure::Retryable(reason),
            }
        }
        ureq::Error::Transport(t) => Failure::Retryable(format!("连接失败：{t}")),
    }
}

fn short_error(e: ureq::Error) -> String {
    classify(e).detail().to_string()
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// 一个只回固定内容的假 ComfyUI，用来在没有 GPU 的机器上验证客户端逻辑。
    struct Stub {
        url: String,
        _handle: std::thread::JoinHandle<()>,
    }

    /// 一个先「断线」几次、之后才正常应答的假节点——用来模拟孤立的连接抖动：
    /// 前 `fail_times` 次连接直接被服务端丢弃、不回任何字节，
    /// 之后按 `routes` 正常应答。
    fn flaky_stub(fail_times: usize, routes: Vec<(&'static str, Value)>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(32) {
                let Ok(mut stream) = stream else { continue };
                let n = seen.fetch_add(1, Ordering::SeqCst);
                if n < fail_times {
                    // 直接断开，不读也不写——ureq 在读响应时会遇到连接层错误。
                    drop(stream);
                    continue;
                }
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

    /// 记下每个请求的 `Authorization` 头（没有就记 `(none)`），其余一律回 200。
    /// 用来证明鉴权头真的贴在了每一类请求上，而不是只贴在提交那一个。
    fn auth_recording_stub(seen: Arc<Mutex<Vec<String>>>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(32) {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut auth = "(none)".to_string();
                let mut line = String::new();
                // 读完整个头部，找 Authorization。
                while reader.read_line(&mut line).is_ok() {
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(v) = trimmed
                        .strip_prefix("Authorization: ")
                        .or_else(|| trimmed.strip_prefix("authorization: "))
                    {
                        auth = v.to_string();
                    }
                    line.clear();
                }
                seen.lock().unwrap().push(auth);
                let body = json!({
                    "queue_running": [], "queue_pending": [],
                    "prompt_id": "p1", "name": "ref.png"
                })
                .to_string();
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

    /// 一个按脚本回状态码和 body 的假代理，并数清楚被打了几次。
    ///
    /// `script` 里每一项是 `(状态码, body)`，按顺序用；用完之后一直用最后一项。
    /// 这是验「该不该重试」唯一诚实的办法——只断言最终错误码，分不清
    /// 「重试了三次才放弃」和「一次都没试就放弃」。
    struct ScriptedStub {
        url: String,
        hits: Arc<AtomicUsize>,
        _handle: std::thread::JoinHandle<()>,
    }

    impl ScriptedStub {
        /// 这个桩被打了几次。
        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    fn scripted_stub(script: Vec<(u16, String)>) -> ScriptedStub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(64) {
                let Ok(mut stream) = stream else { continue };
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                // 请求体要读掉，否则对端可能拿到 broken pipe 而不是我们写的响应。
                let mut line = String::new();
                let mut len = 0usize;
                while reader.read_line(&mut line).is_ok() {
                    let t = line.trim_end().to_string();
                    if t.is_empty() {
                        break;
                    }
                    if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length: ") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    line.clear();
                }
                if len > 0 {
                    let mut body = vec![0u8; len];
                    use std::io::Read;
                    let _ = reader.read_exact(&mut body);
                }
                let (code, body) = &script[n.min(script.len() - 1)];
                let reason = if *code == 200 { "OK" } else { "Error" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        ScriptedStub {
            url: format!("http://127.0.0.1:{port}"),
            hits,
            _handle: handle,
        }
    }

    /// 代理把「请求本身不对」和「集群暂时忙不过来」分得很清楚，
    /// 我们也得分清楚——**混起来的代价不是白等几轮**，是把一个必然失败的
    /// 请求包装成「联系不上节点」，然后把排查引向网络。
    #[test]
    fn a_bad_request_is_fatal_and_a_busy_cluster_is_retryable() {
        let cases: Vec<(u16, &str, bool, &str)> = vec![
            (
                400,
                r#"{"error":"prompt is required"}"#,
                false,
                "prompt is required",
            ),
            (401, r#"{"error":"unauthorized"}"#, false, "unauthorized"),
            (404, r#"{"error":"not found"}"#, false, "not found"),
            (
                503,
                r#"{"error":"no healthy node"}"#,
                true,
                "no healthy node",
            ),
            (
                503,
                r#"{"error":"context deadline exceeded"}"#,
                true,
                "context deadline exceeded",
            ),
        ];
        for (code, body, retryable, needle) in cases {
            let s = scripted_stub(vec![(code, body.to_string())]);
            let c = Comfy::new(s.url.clone(), None, 5, 1);
            let e = c.get(&format!("{}/queue", s.url)).call().unwrap_err();
            let f = classify(e);
            assert_eq!(
                matches!(f, Failure::Retryable(_)),
                retryable,
                "HTTP {code} 的可重试判断错了：{f:?}"
            );
            assert!(
                f.detail().contains(needle),
                "代理写在 body 里的原因被丢掉了：{f:?}"
            );
            // classify 是**纯分类**，这一层一次都不重试——重试是调用方的事
            // （`submit` 在 S2 里做）。这条断言在那时会换成「重试了几次」。
            assert_eq!(s.hits(), 1, "HTTP {code} 上不该有隐式重试");
        }
    }

    /// 两种 503 都要重试，但重试到顶之后报的不是一回事。
    #[test]
    fn only_no_healthy_node_means_the_cluster_is_down() {
        let busy = classify_body(503, r#"{"error":"context deadline exceeded"}"#);
        let down = classify_body(503, r#"{"error":"no healthy node"}"#);
        assert!(!busy.is_no_healthy_node(), "排队不是集群挂了：{busy:?}");
        assert!(down.is_no_healthy_node(), "{down:?}");
    }

    /// 节点自己返回的错误是原样透传的，不是代理那个 `{"error":...}` 形状。
    /// 读不出来就退回 `HTTP {code}`，不能 panic，也不能瞎猜一个原因。
    #[test]
    fn an_error_body_that_is_not_the_proxy_shape_falls_back_to_the_status_code() {
        let f = classify_body(400, "<html>node said no</html>");
        assert_eq!(f, Failure::Fatal("HTTP 400".into()));
    }

    fn classify_body(code: u16, body: &str) -> Failure {
        let s = scripted_stub(vec![(code, body.to_string())]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        classify(c.get(&format!("{}/queue", s.url)).call().unwrap_err())
    }

    /// **排队不是失败。** 八台节点都在忙时代理回 503，退一会儿再来就成了。
    /// 这条守的是整份 SPEC-0017 的落点。
    #[test]
    fn a_submission_that_queues_behind_a_busy_cluster_eventually_succeeds() {
        let s = scripted_stub(vec![
            (503, r#"{"error":"context deadline exceeded"}"#.into()),
            (503, r#"{"error":"context deadline exceeded"}"#.into()),
            (200, r#"{"prompt_id":"p1","number":1}"#.into()),
        ]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let sub = c.submit(&json!({}), "video-studio").unwrap();
        assert_eq!(sub.prompt_id, "p1");
        assert_eq!(s.hits(), 3, "该重试两次之后才成功");
    }

    /// 请求本身不对时**一次都不重试**——重试没有意义，而且会把一个即时的
    /// 错误拖成十几秒。只断言最终错误码分不清这两种，所以要数请求次数。
    #[test]
    fn a_bad_submission_is_not_retried() {
        let s = scripted_stub(vec![(400, r#"{"error":"prompt is required"}"#.into())]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let e = c.submit(&json!({}), "video-studio").unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(
            e.message().contains("prompt is required"),
            "{}",
            e.message()
        );
        assert_eq!(s.hits(), 1, "400 上不该重试");
    }

    /// 一个健康节点都没有 → 集群不可用，remedy 该指向配置与恢复。
    #[test]
    fn no_healthy_node_reports_the_cluster_as_unavailable() {
        let s = scripted_stub(vec![(503, r#"{"error":"no healthy node"}"#.into())]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let e = c.submit(&json!({}), "video-studio").unwrap_err();
        assert_eq!(e.code(), "comfy_unavailable", "{}", e.message());
        assert!(e.remedy().contains("COMFY_NODE"), "{}", e.remedy());
        assert_eq!(s.hits(), SUBMIT_RETRY_BACKOFF.len(), "该退避重试到顶");
    }

    /// 节点在、只是一直忙 → 这次执行失败，remedy 该指向 retry_stage，
    /// **不是**「去检查 COMFY_NODE 配置」。两者混起来人就会去查错的地方。
    #[test]
    fn a_cluster_that_stays_busy_reports_a_failed_run_not_a_broken_config() {
        let s = scripted_stub(vec![(
            503,
            r#"{"error":"context deadline exceeded"}"#.into(),
        )]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let e = c.submit(&json!({}), "video-studio").unwrap_err();
        assert_eq!(e.code(), "comfy_failed", "{}", e.message());
        assert!(e.remedy().contains("retry_stage"), "{}", e.remedy());
    }

    /// 提交超时不跟控制面共用：代理会替我们排队，等的是「有节点空出来」，
    /// 不是「节点没反应」。夹住上下限是为了不让手滑写小的配置把提交变成必失败。
    #[test]
    fn the_submit_timeout_is_clamped_not_inherited() {
        let tiny = Comfy::new("http://x".into(), None, 10, 1);
        assert_eq!(tiny.submit_timeout(), Duration::from_secs(60), "下限没夹住");
        let huge = Comfy::new("http://x".into(), None, 36000, 1);
        assert_eq!(
            huge.submit_timeout(),
            Duration::from_secs(600),
            "上限没夹住"
        );
        let mid = Comfy::new("http://x".into(), None, 600, 1);
        assert_eq!(
            mid.submit_timeout(),
            Duration::from_secs(300),
            "区间内取一半"
        );
        // 默认的 COMFY_TIMEOUT_SECS=1800 会被上限夹到 600——这是有意的，
        // 排了十分钟还没轮上，等下去也没意义了。
        let default = Comfy::new("http://x".into(), None, 1800, 1);
        assert_eq!(default.submit_timeout(), Duration::from_secs(600));
    }

    /// 轮询遇到 `Fatal` 立即失败，不进那五轮宽限期。
    #[test]
    fn a_history_lookup_that_is_rejected_fails_immediately() {
        let s = scripted_stub(vec![(404, r#"{"error":"not found"}"#.into())]);
        let c = Comfy::new(s.url.clone(), None, 30, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "p1".into(),
        };
        let e = c.wait(&sub).unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(e.message().contains("重试没有意义"), "{}", e.message());
        assert_eq!(s.hits(), 1, "404 上不该白等五轮");
    }

    #[test]
    fn an_unreachable_entrypoint_blocks_instead_of_degrading() {
        // 端口 1 上不会有 ComfyUI。
        let c = Comfy::new("http://127.0.0.1:1".into(), None, 1, 1);
        let e = c.ensure_reachable().unwrap_err();
        assert_eq!(e.code(), "comfy_unavailable");
        assert!(e.remedy().contains("COMFY_NODE"));
        assert!(
            e.remedy().contains("不要降级"),
            "连不上时必须明确禁止换模型：{}",
            e.remedy()
        );
    }

    #[test]
    fn health_reports_queue_depth() {
        let s = stub(vec![(
            "/queue",
            json!({ "queue_running": [1], "queue_pending": [1, 2] }),
        )]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let h = c.health();
        assert!(h.reachable);
        assert_eq!(h.queue_depth, 3);
    }

    /// 配了 token 就必须每个请求都带上——代理靠它放行，漏一个就是 403。
    #[test]
    fn every_request_carries_the_bearer_token_when_configured() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = auth_recording_stub(Arc::clone(&seen));
        let c = Comfy::new(s.url.clone(), Some("tok123".into()), 5, 1);

        let _ = c.health();
        let sub = c.submit(&json!({}), "test").unwrap();
        let _ = c.try_history(&sub);
        let dest = tempfile::tempdir().unwrap().path().join("out.bin");
        let _ = c.download(
            &RemoteFile {
                filename: "out.mp4".into(),
                subfolder: String::new(),
                r#type: "output".into(),
            },
            &dest,
        );
        let _ = c.upload_image("ref.png", b"bytes");

        let got = seen.lock().unwrap();
        assert!(got.len() >= 5, "五类请求都该打到桩上：{got:?}");
        assert!(
            got.iter().all(|h| h == "Bearer tok123"),
            "每个请求都要带 token：{got:?}"
        );
    }

    /// 没配 token 就不该凭空造一个头出来——直连不设防的 ComfyUI 是正常用法。
    #[test]
    fn no_authorization_header_when_token_is_absent() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = auth_recording_stub(Arc::clone(&seen));
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let _ = c.health();
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &["(none)".to_string()],
            "没配 token 时不该带 Authorization"
        );
    }

    #[test]
    fn submit_returns_the_prompt_id_for_traceability() {
        let s = stub(vec![(
            "/prompt",
            json!({ "prompt_id": "abc-123", "number": 1 }),
        )]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let sub = c
            .submit(&json!({ "1": { "class_type": "X", "inputs": {} } }), "cid")
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
        let c = Comfy::new(s.url.clone(), None, 5, 1);
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
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        assert!(c.try_history(&sub).unwrap().is_none());
    }

    /// try_history 对孤立的连接层错误也当作「还没查到结果」，不报错。
    #[test]
    fn try_history_treats_a_connection_hiccup_as_still_pending() {
        let s = flaky_stub(1, vec![("/history/", json!({}))]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        assert!(
            c.try_history(&sub).unwrap().is_none(),
            "连接层错误不该冒泡成 Err"
        );
    }

    /// 这是本 issue 复盘的那次故障：一次孤立的轮询连接超时不该打断一个
    /// 实际正常运行、最终能跑完的渲染。
    #[test]
    fn an_isolated_connection_hiccup_does_not_fail_a_wait_that_eventually_succeeds() {
        let s = flaky_stub(
            2,
            vec![(
                "/history/",
                json!({ "abc-123": {
                    "status": { "status_str": "success", "completed": true },
                    "outputs": { "9": { "videos": [ { "filename": "sh01.mp4" } ] } }
                }}),
            )],
        );
        let c = Comfy::new(s.url.clone(), None, 30, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        let files = c.wait(&sub).unwrap();
        assert_eq!(files[0].filename, "sh01.mp4");
    }

    /// 但节点是真的挂了、连续联系不上超过阈值，还是要报错——不能无限等下去。
    #[test]
    fn wait_gives_up_after_too_many_consecutive_unreachable_polls() {
        // 端口 1 上不会有 ComfyUI；timeout 给得很宽，逼出的是连续失败阈值而不是总超时。
        let c = Comfy::new("http://127.0.0.1:1".into(), None, 120, 1);
        let sub = Submission {
            node: "http://127.0.0.1:1".into(),
            prompt_id: "abc-123".into(),
        };
        let started = Instant::now();
        let e = c.wait(&sub).unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(e.message().contains("联系不上"));
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "应当在远小于总超时的时间内因连续失败而放弃"
        );
    }

    /// ComfyUI 自己报的结构化错误不受连接容错宽限，立即失败。
    #[test]
    fn a_structured_comfy_error_still_fails_immediately() {
        let s = stub(vec![(
            "/history/",
            json!({ "abc-123": { "status": { "status_str": "error", "completed": false,
                "messages": [["execution_error", { "node_type": "KSampler",
                                                   "exception_message": "CUDA out of memory" }]] } }}),
        )]);
        let c = Comfy::new(s.url.clone(), None, 30, 1);
        let sub = Submission {
            node: s.url.clone(),
            prompt_id: "abc-123".into(),
        };
        let started = Instant::now();
        let e = c.wait(&sub).unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(e.message().contains("KSampler"));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "结构化错误不该等待重试"
        );
    }

    #[test]
    fn execution_errors_name_the_node_and_reason() {
        let s = stub(vec![(
            "/history/",
            json!({ "abc-123": { "status": { "status_str": "error", "completed": false,
                "messages": [["execution_error", { "node_type": "KSampler",
                                                   "exception_message": "CUDA out of memory" }]] } }}),
        )]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
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

    /// 加载类节点会把输入回显进 outputs。收下它的后果是把锚点素材当成
    /// 渲染结果登记下来——图能跑、有文件、下载得到，一路绿到交付。
    #[test]
    fn an_echoed_input_file_is_not_a_product() {
        // 键名有意让 load 排在 save 前面：调用方取的是第一个。
        let outputs = json!({
            "guide1_src_load": {
                "images": [{ "filename": "anchor.mp4", "subfolder": "", "type": "input" }]
            },
            "save_video": {
                "images": [{ "filename": "sh01_00001_.mp4", "subfolder": "", "type": "output" }]
            }
        });
        let files = collect_files(&outputs);
        assert_eq!(files.len(), 1, "输入回显不算产物");
        assert_eq!(files[0].filename, "sh01_00001_.mp4");
    }

    /// 缺 `type` 的按 output 算——把它们判成非产物才是新的错。
    #[test]
    fn a_file_without_a_type_still_counts_as_a_product() {
        let outputs = json!({ "9": { "videos": [{ "filename": "sh01.mp4" }] } });
        let files = collect_files(&outputs);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].r#type, "output");
    }

    #[test]
    fn download_writes_the_response_body_to_the_destination_file() {
        let s = stub(vec![("/view", json!({ "bytes": "这是产物内容" }))]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested/sh01.mp4");
        let file = RemoteFile {
            filename: "sh01.mp4".into(),
            subfolder: String::new(),
            r#type: "output".into(),
        };
        let n = c.download(&file, &dest).unwrap();
        assert!(n > 0);
        assert!(dest.is_file(), "download 应当顺带建好中间目录");
        let body = std::fs::read_to_string(&dest).unwrap();
        assert!(body.contains("这是产物内容"));
    }

    #[test]
    fn download_from_an_unreachable_node_fails_with_the_node_named() {
        let c = Comfy::new("http://127.0.0.1:1".into(), None, 1, 1);
        let file = RemoteFile {
            filename: "sh01.mp4".into(),
            subfolder: String::new(),
            r#type: "output".into(),
        };
        let e = c
            .download(&file, &std::path::PathBuf::from("/tmp/x.mp4"))
            .unwrap_err();
        assert_eq!(e.code(), "comfy_failed");
        assert!(e.message().contains("127.0.0.1:1"));
    }

    #[test]
    fn upload_image_returns_the_name_comfyui_assigned() {
        let s = stub(vec![("/upload/image", json!({ "name": "已改名.png" }))]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let name = c.upload_image("参考图.png", b"fake-bytes").unwrap();
        assert_eq!(name, "已改名.png");
    }

    #[test]
    fn upload_image_falls_back_to_the_given_name_when_the_response_omits_it() {
        let s = stub(vec![("/upload/image", json!({}))]);
        let c = Comfy::new(s.url.clone(), None, 5, 1);
        let name = c.upload_image("参考图.png", b"fake-bytes").unwrap();
        assert_eq!(name, "参考图.png");
    }
}
