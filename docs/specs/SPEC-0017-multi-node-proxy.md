# SPEC-0017 对齐多节点代理：排队不是失败

| | |
|---|---|
| 起因 | `sunsheng/comfyui-proxy` 重新部署，接入方式从「一台 ComfyUI」变成「一个代理 + 8 台节点」 |
| 依据 | 该仓库的 [`docs/usage.md`](https://github.com/sunsheng/comfyui-proxy/blob/main/docs/usage.md)，以及 2026-09-05 对新部署的只读探针 |
| 状态 | 规格 |

---

## 1. 变了什么

**接入方式没变**——仍然是单个 `COMFY_NODE` URL 加 `COMFY_TOKEN`，
`studio-comfy` 不需要知道后端有几台机器。变的是**那一个 URL 背后的语义**。

2026-09-05 对新部署的只读探针（不烧 GPU）：

| 探什么 | 结果 |
|---|---|
| `GET /healthz`（不带 token） | `200`，`{"status":"ok"}`——代理自身的健康探测，**不属于 ComfyUI API** |
| `GET /system_stats` | 以**节点地址为 key** 的对象，**8 个 key**：`host.docker.internal:9001` … `:9008` |
| 每个节点 | A800 80GB PCIe，79 GiB，实时空闲 78 GiB |
| `GET /queue` | 合并所有在线节点，`200`；全部健康时没有额外响应头 |
| `GET /history/<不存在的 id>` | `200 {}`——**不是 400**，所以轮询照旧把它当「还没跑完」 |
| `GET /view?type=temp` | `400`，代理不再支持 |

**从一台变八台**，这条对我们的影响比接口形状本身大得多：并发压进去的镜头
一定会有一部分在排队，而排队在代理那一侧是**等待**，在我们这一侧长得像
**超时**。

## 2. 五处不对

按严重程度排。

### 2.1 排队被当成渲染失败（最严重）

`Comfy::submit` 把 `/prompt` 的任何错误映射成 `ComfyFailed`，这一镜就废了。
但按 usage.md，`503` 是**临时基础设施状况，应当指数退避重试**，而且有两种
完全不同的成因，从 `error` 字段区分：

| `error` | 含义 | 该怎么做 |
|---|---|---|
| `"no healthy node"` | 等了 5 秒，集群里一个健康节点都没有 | 退避久一点再试；久等不来才算 `comfy_unavailable` |
| 其他（如 `"context deadline exceeded"`） | **有健康节点，只是都在忙**（到了 `MAX_CONCURRENT_PER_NODE`） | 这不是故障，是排队。重试，或者把调用方超时调大 |

更麻烦的是第二种的时序：代理会**一直排队等节点空出来，直到调用方自己的
HTTP 客户端超时或取消**。而 `/prompt` 现在走的是控制面那 30 秒读超时——
于是「排队 31 秒」在我们这里表现为「渲染失败」。

**八台节点 × 队列深度默认 16**，这条必然被踩到。

### 2.2 `503` 的真正原因被丢掉

`short_error` 只保留 `HTTP {code}`，把 body 扔了。而代理**所有自己生成的
错误都是同一个形状**：`{"error": "<具体原因>"}`。丢掉它等于丢掉上表里
区分两种 503 的唯一依据，报给人的也只有一句「HTTP 503」——没有 remedy
能指的方向。

### 2.3 `400` 被当成可以重试

`poll` 把任何非 2xx 都归成 `Unreachable`，重试到 `MAX_CONSECUTIVE_UNREACHABLE`
才失败。usage.md 写得很清楚：`400` 是**请求本身有问题**（缺字段、不支持的
操作、body 不是合法 JSON），「原样重试没有意义」。`401`（token 不对）、
`404`（路径不对）同理。

白等五轮不算大事，**把一个必然失败的请求包装成「联系不上节点」才是**——
那是一句不对的话，会把排查引向网络。

### 2.4 集群视图不完整时 `doctor` 说得像完整的

部分节点失联时 `/queue`、`/history`、`/system_stats` 仍返回 `200`，
但结果只含在线节点，代理通过响应头告知：

| Header | 什么时候出现 |
|---|---|
| `X-Comfy-Unreachable-Nodes` | `GET /queue` / `/history` / `/system_stats` 有节点没响应 |

`Comfy::health` 不读这个头。于是八台里挂了三台时，`doctor` 报的仍然是干净的
「可达，队列深度 N」——**这正是本项目反复栽的那种失败方式**：机器说成功了，
但成功的不是你以为的那件事。

### 2.5 `doctor` 说不出探到了什么

CLAUDE.md 要求「不得声称集成通过而不说明当时探到了什么」，并把探针清单
列成表。现在 `doctor` 只说「可达，队列深度 0」，说不出集群有几台、几台健康、
显存多大。`/system_stats` 一次调用就能拿到，而且不碰 GPU。

## 3. 定下来的形状

### 3.1 错误分三类，不再一律 `Unreachable`

```
Retryable   503 / 连接层错误（超时、断连）      → 退避重试
Fatal       400 / 401 / 404                     → 立即失败，不重试
Ok          2xx                                  → 正常处理
```

其余状态码按 Retryable 处理——保守的一侧是多试一次，不是把可能能成的判死。

**`503` 的两种成因不在这一层区分。** 两种都要退避重试，区别只在「重试到顶
之后报什么」：`no healthy node` 报 `comfy_unavailable`（集群不可用，remedy
指向配置和恢复），其他报 `comfy_failed`（这次执行失败，remedy 指向
`retry_stage`）。

### 3.2 提交要能扛住排队

`/prompt` 从控制面超时里拿出来，单独给一档：

| 用途 | 读超时 | 理由 |
|---|---|---|
| 控制面小 JSON（`/history`、`/queue`、`/healthz`） | 30 s | 都是即时返回，短一点能早发现节点没反应 |
| **提交 `/prompt`** | **`COMFY_TIMEOUT_SECS` 的一半，夹在 [60 s, 600 s]** | 代理会替我们排队，等的是「有节点空出来」，不是「节点没反应」 |
| 大块传输（`/upload/image`、`/view`） | 300 s | SPEC 之外，已在上一轮改动里分开 |

取 `COMFY_TIMEOUT_SECS` 的一半：那个值本来就是「这一镜我愿意等多久」，
排队占掉一半还没轮上，等下去也没意义了。夹住上下限是为了不让一个手滑写小的
配置把提交变成必失败。

提交本身再加**指数退避重试**：3 次，间隔 2/4/8 秒。这是给「刚好撞上一次
调度抖动」留的余地，真正的长时间排队由上面那个超时覆盖。

### 3.3 `health()` 带上集群视图

```rust
pub struct NodeHealth {
    url: String,
    reachable: bool,
    queue_depth: usize,
    detail: Option<String>,
    /// 代理报的失联节点（`X-Comfy-Unreachable-Nodes`）。空表示视图完整。
    unreachable_nodes: Vec<String>,
}
```

有失联节点时 `reachable` 仍是 `true`（至少一个节点在，能干活），
但 `detail` 要说清楚「队列深度是部分视图」。

### 3.4 `doctor` 报集群构成

`/system_stats` 的结果加进体检输出：几个节点、每个的显卡型号与显存、
几个失联。**探到什么就报什么**，这是 CLAUDE.md 那条要求的直接落实。

拿不到 `/system_stats` 不算 Fail——它只是描述性信息，`/queue` 能通就说明
能干活。

## 4. 不做的事

- **不改接入方式。** 仍然是单个 `COMFY_NODE`，`studio-comfy` 不感知节点数。
  代理存在的意义就是让我们不必知道，把节点列表读进来做调度是往回走。
- **不动 `COMFY_CONCURRENCY` 的默认值。** 默认 16 对 8 台是队列深度 2，
  合适。压不满是代理那侧的调度问题，不是客户端该猜的。
- **不用 `/healthz` 替换 `/queue` 做探活。** `/healthz` 更便宜，但拿不到
  队列深度，而 `doctor` 要报深度。`/queue` 全部节点失联时同样返回 503，
  语义够用。
- **不加 `/interrupt`、`/free` 的调用。** 现在没在用，新协议要求 `prompt_id`
  必填这件事记在文档里就够了，等真要用时再说。
- **不碰 `/view?type=temp`。** `collect_files` 只认 `type == "output"`，
  本来就不会去请求 temp。加一条测试锁住这个前提。

## 5. 怎么验

| 层次 | 怎么验 |
|---|---|
| 单元 | 假节点分别回 400 / 503(`no healthy node`) / 503(其他) / 200，断言分类与重试次数正确 |
| 单元 | 假节点回 200 且带 `X-Comfy-Unreachable-Nodes`，断言 `health()` 读出来了 |
| 真机（只读，不烧 GPU） | `doctor` 报出 8 个节点、A800 80GB；`/view?type=temp` 仍是 400 |
| 真机（烧 GPU） | 一次并发压满的渲染跑通，排队没有被报成失败 |

**最后一项要等 GPU 时间。** 前三项不需要，先做。
