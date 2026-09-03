# 架构

## 三个平面

| 平面 | 是谁 | 能力 | 对外协议 |
|---|---|---|---|
| Agent 面 | Codex 会话 | 自然语言理解、创作判断、向用户提问 | MCP (stdio) |
| 控制面 | `studiod serve` | 阶段循环、确认门、检查点、产物登记、恢复 | HTTP → ComfyUI；exec → ffmpeg |
| 推理面 | ComfyUI 容器 | GPU 推理、模型权重、custom node | `/prompt`、`/history` |

Agent 面永远不直接接触控制面的状态存储，也不直接接触推理面。

## 运行本程序的机器不需要 GPU

控制面与推理面之间只有 HTTP。`studio-comfy` 负责：健康检查、选择队列最短的节点、
上传输入、提交 workflow、轮询 `/history/{prompt_id}`、下载输出。
模型权重、custom node、CUDA 全部在 ComfyUI 那一侧，本机一概不需要。

因此控制面可以跑在一台没有显卡的小机器上，甚至和 ComfyUI 不在同一台主机——
节点地址在 `config.toml` / `.env` 里配置即可。

## 外部程序依赖

只有 ffmpeg / ffprobe（媒体拼接、转码、字幕、封面、元数据）。
它们**不要求在 PATH 中**：`studio-media` 的查找顺序是

1. bundle 的 `.env` → 程序目录的 `.env` → 进程环境变量：`FFMPEG_PATH` / `FFPROBE_PATH`
2. `config.toml` 的 `[media]` 段
3. PATH

缺失时 `studiod doctor` 报告缺什么、去哪配，相关阶段返回 `tool_unavailable` 并附 remedy。

## 确定性阶段的执行

`render` / `post` / `review` 由控制面在后台线程里执行，用自己的 SQLite 连接
写状态（WAL + busy_timeout）。门一通过就开始，Agent 只需要 `studio.status`。

`render` 逐镜头走：选队列最短的健康节点 → 加载已验证基线并注入逐镜头参数 →
提交 `/prompt` → 轮询 `/history/{prompt_id}` → 下载到 `media/`。
基线格式见 [assets/workflows/README.md](../assets/workflows/README.md)。

`review` 的每一条检查都基于 `ffprobe` 的实测值，不靠推断。

## bundle 即文档

一个文件夹就是一部作品，没有 `run_id`、没有 run 注册表。
`list` / `branch` / `archive` / `cancel` 由文件系统提供：`ls`、`cp -r`、`mv`、`rm -rf`。
