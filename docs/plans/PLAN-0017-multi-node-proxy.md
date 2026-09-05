# PLAN-0017 对齐多节点代理

规格见 [SPEC-0017](../specs/SPEC-0017-multi-node-proxy.md)。

分四步，每步做完仓库都是绿的。前三步不需要 GPU，S4 要。

---

## S1 错误分类：把 `503` / `400` 分开，并且带上代理给的原因

**改哪**：`crates/studio-comfy/src/lib.rs`

- 新增 `enum Failure { Retryable(String), Fatal(String) }`，由一个
  `classify(&ureq::Error) -> Failure` 产生。
- `classify` 要**读 body**：代理自己生成的错误一律是 `{"error": "<原因>"}`，
  丢掉它就丢掉了区分两种 503 的唯一依据。节点自己返回的错误原样透传、
  不是这个形状，读不出来就退回 `HTTP {code}`。
- `403` 视作 Fatal（token 不对的另一种表现）；未列出的状态码按 Retryable，
  保守的一侧是多试一次。

**怎么验**：
- 假节点回 `400 {"error":"..."}`，断言 `Fatal` 且消息里有那句原因
- 假节点回 `503 {"error":"no healthy node"}`，断言 `Retryable` 且消息里有它
- 假节点回 `503 {"error":"context deadline exceeded"}`，同上
- 节点自己的 400（body 不是 `{"error":...}`），断言不 panic、退回 `HTTP 400`

**切提交**：S1 单独一个。它不改任何调用点的行为，只是把判断能力做出来。

---

## S2 提交扛住排队

**改哪**：同上。

- `submit()` 用新的 `submit_timeout()`：`comfy_timeout_secs / 2`，
  夹在 `[60, 600]`。
- `submit()` 加指数退避重试：3 次，2/4/8 秒。`Fatal` 不重试，立即返回。
- 重试到顶后：`error` 是 `no healthy node` → `ComfyUnavailable`；
  其余 → `ComfyFailed`。
- `poll()` 的 `Fatal` 不再计入 `MAX_CONSECUTIVE_UNREACHABLE`，直接
  `PollOutcome::Failed`。

**怎么验**：
- 假节点前两次回 503、第三次回 200，断言 `submit` 成功且只提交了一次任务
- 假节点一直回 `503 {"error":"no healthy node"}`，断言最终是 `comfy_unavailable`
- 假节点一直回 `503 {"error":"context deadline exceeded"}`，断言最终是 `comfy_failed`
- 假节点回 400，断言**不重试**（桩记调用次数）
- `submit_timeout` 的夹取：`COMFY_TIMEOUT_SECS=10` → 60；`=3600` → 600

**切提交**：S2 单独一个。这是这份计划的主体。

---

## S3 集群视图：`health()` 读响应头，`doctor` 报构成

**改哪**：`studio-comfy`（`NodeHealth` 加字段）、`studio-cli` 的 `doctor`。

- `NodeHealth` 加 `unreachable_nodes: Vec<String>`，从
  `X-Comfy-Unreachable-Nodes` 解析（逗号分隔）。有失联节点时 `reachable`
  仍为 `true`，但 `detail` 说明队列深度是部分视图。
- 新增 `Comfy::cluster()`：调 `/system_stats`，返回节点地址 → 显卡型号/显存。
  拿不到不算错误，返回空。
- `doctor` 的 ComfyUI 那一项，除了「可达、队列深度」，再报节点数、
  显卡型号与显存、失联节点。拿不到 `/system_stats` 时只是少报几行，不降级。

**怎么验**：
- 假节点回 200 带 `X-Comfy-Unreachable-Nodes: a:1,b:2`，断言解析出两个
- 假节点回 `/system_stats` 的多节点形状，断言 `cluster()` 数得对
- 真机（只读）：`doctor` 报出 8 个节点、A800 80GB

**切提交**：S3 单独一个。

---

## S4 真机验收

**要 GPU，等得到 GPU 时间再做。**

- 一次把队列压满的渲染（镜头数 ≥ 并发度），确认**排队没有被报成失败**——
  这是整份计划的落点。
- `doctor` 在真集群上的输出贴进 PR。
- 顺带重跑 `real_comfy` 全套：这次改动碰了提交与轮询这两条主路径。

**验收结论要写清楚探到了什么**（节点数、型号、显存），按 CLAUDE.md 那条。

---

## 文档

- `docs/e2e.md` 的探针清单加一行 `/system_stats`：能探到集群构成。
- `CLAUDE.md` 的探针表里 ComfyUI 那一行补一句：现在那一侧是多节点代理，
  排队不是故障。
- `README` / `.env` 说明不动——接入方式没变。
