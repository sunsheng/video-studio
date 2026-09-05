# 架构

## 三个平面

| 平面 | 是谁 | 能力 | 对外协议 |
|---|---|---|---|
| Agent 面 | Codex 会话 | 自然语言理解、创作判断、向用户提问 | MCP (stdio) |
| 控制面 | `studiod`（没有子命令，被拉起就是 serve） | 阶段循环、确认门、检查点、产物登记、恢复 | HTTP → ComfyUI；exec → ffmpeg |
| 推理面 | ComfyUI 容器 | GPU 推理、模型权重、custom node | `/prompt`、`/history` |

Agent 面永远不直接接触控制面的状态存储，也不直接接触推理面。

## 运行本程序的机器不需要 GPU

控制面与推理面之间只有 HTTP。`studio-comfy` 负责：探活、上传输入、提交
workflow、轮询 `/history/{prompt_id}`、下载输出。模型权重、custom node、
CUDA 全部在 ComfyUI 那一侧，本机一概不需要。

因此控制面可以跑在一台没有显卡的小机器上，甚至和 ComfyUI 不在同一台主机——
地址在 `config.toml` / `.env` 里配置即可。

**入口只有一个 URL。** 早期版本让控制面维护一个节点列表、按队列深度挑最短
的那个；现在那一侧通常是负载均衡代理，分发与故障转移都归它管，控制面既看
不见后端有几个节点，也不该管。于是：

- `COMFY_NODE` 是单个地址（`COMFY_NODES` 作为旧名仍然认，只取第一个，
  多余的值由 `doctor` 报出来而不是静默丢弃）
- 并发度显式配（`COMFY_CONCURRENCY`，默认 16），不再由「健康节点数」推导
- 需要鉴权的代理配 `COMFY_TOKEN`，客户端给每个请求贴 `Authorization: Bearer`
- 「排除某个坏节点」这个动作没有了——排除唯一的入口等于关掉渲染

## 外部程序依赖

只有 ffmpeg / ffprobe（媒体拼接、转码、字幕、封面、元数据）。
它们**不要求在 PATH 中**：`studio-media` 的查找顺序是

1. bundle 的 `.env` → 程序目录的 `.env` → 进程环境变量：`FFMPEG_PATH` / `FFPROBE_PATH`
2. `config.toml` 的 `[media]` 段
3. PATH

缺失时 `studio-cli doctor` 报告缺什么、去哪配，相关阶段返回 `tool_unavailable` 并附 remedy。

## 确定性阶段的执行

`preview` / `render` / `post` / `review` 由控制面在后台线程里执行，用自己的
SQLite 连接写状态（WAL + busy_timeout）。门一通过就开始，Agent 只需要
`studio.status`。`preview` 执行完不会直接放行——480p 预览生成后走跟
Agent 提交带门阶段一样的 `AwaitingConfirmation` 挂起，等确认才轮到
花钱的正式 `render`；它自己不产出独立内容，门上选「有问题」统一
重定向退回 `prompt_pack`。

`preview` 和 `render` 共享同一套生成逻辑，仅目标分辨率不同——`preview`
按短边 480 等比缩放，帧数/时长照抄提示词包不变。两者都按当下健康的
节点数并发：每个健康节点固定绑一个 worker 线程认领队列里的镜头，
提交 `/prompt` → 轮询 `/history/{prompt_id}` → 下载到 `media/`（`preview`
落在 `media/preview/`）。轮询容忍孤立的连接层抖动——只在连续失败超过
阈值或总耗时超过 `timeout` 才真正判失败，ComfyUI 自己报的结构化错误
不受此宽限。单镜提交-等待-下载失败后最多重试三次，每次重试重新选节点。
怀疑某个节点本身有问题，可以用 `studio.comfy.exclude_node` 把它从这次
会话的候选里临时摘掉；执行失败但内容没问题，用 `studio.retry_stage`
干净重试——它会先停掉可能还在跑的 worker 再重来，不会像 `studio.revise`
那样让旧线程跑完之后拿旧状态覆盖新决定。
基线格式见 [assets/workflows/README.md](../assets/workflows/README.md)。

`review` 的每一条检查都基于 `ffprobe` 的实测值，不靠推断。

执行的每一步都计时落进 `.studio/exec.jsonl`：选节点、加载基线、提交、
排队渲染、下载、拼接、抽帧、字幕、探测。这是一条独立于 MCP 留痕的记录线，
因为这些动作根本不经过 MCP。两份留痕对应两份报告，见 [e2e.md](e2e.md)。

## bundle 即文档

一个文件夹就是一部作品，没有 `run_id`、没有 run 注册表。
`list` / `branch` / `archive` / `cancel` 由文件系统提供：`ls`、`cp -r`、`mv`、`rm -rf`。
