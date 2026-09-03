# ADR-0001：一个文件夹就是一部作品

## 背景

前身 douyin-video-studio（Python，7250 行）在 2026-09-03 的一次会话中暴露结构性问题：
用户五句话，Agent 41 次调用，其中 10 分钟 18 次调用花在一次「把每镜头 2 秒改成智能分配」的修订上。

```
03:21:18  studio_revise_stage -> {"status": "ready_for_redo"}
03:22:00  studio_submit_stage -> -32602: task already claimed: stage.script.v1
03:25:14  python -c 'state_store.cancel_pending_questions(...)'
03:29:21  python -c 'con.execute("UPDATE questions SET status=\"pending\" ...")'
```

根因是工具面缺口、错误信息是死路、文档指路去读源码——不是模型不听话。

## 决策

**一个文件夹 = 一部作品 = 一份文档。**

## 这消掉了什么

| 旧机制 | 现在 |
|---|---|
| `run_id` 生成、隔离、传参 | 没有了，文件夹就是 run |
| `list` / `detail` / `audit` / `orphaned` | `ls` |
| `branch` | `cp -r` |
| `archive` | 挪走或 `pack` |
| `cancel` | `rm -rf` |
| `pause` / `resume` | 关掉 / 打开 Codex |
| 跨会话任务锁、锁过期、残留 lock 清理 | 一进程一项目，flock 随进程释放 |
| 多 run 共享 SQLite 的并发协调 | 每个项目自己的库 |

旧项目 `run_management.py` 那 829 行，相当大一部分是在用代码实现文件系统本来就免费提供的东西。

「跑去读别的 run 的 output.json 抄 schema」这个行为在新模型下**物理上不可能**——
这个文件夹里没有别的 run。同时补上 `studio.schema` 让它根本不需要猜。

## 代价

- bundle 里躺着 AGENTS.md 和 `.agents/skills/`，严格说是程序资源不是文档内容。
  这是 Codex 工作区根自动发现机制逼的。类比：docx 里也嵌着样式定义。
  好处是老项目用老契约打开时行为不变。
- 媒体在 bundle 内，`cp -r` 会很重。用 `pack --no-media` 做轻量分叉。
