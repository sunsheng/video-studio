# video-studio

文档式短视频生产工坊。**一个文件夹就是一部作品**——像一份 `.docx`，只有新建、继续、修订三个动作。

- **运行时** Codex（通过 MCP 驱动）
- **实现** Rust 两个二进制（`studiod` 服务 + `studio-cli` 工具），生产环境没有源码、没有解释器
- **推理** ComfyUI HTTP API —— 运行本程序的机器**不需要 GPU**

## 安装

**Linux**

```bash
# 一键安装（默认 /opt/video-studio，不自动 init）
curl -fsSL https://raw.githubusercontent.com/sunsheng/video-studio/main/scripts/install.sh | bash

# 指定目录 + 顺带初始化一部作品
curl -fsSL .../install.sh | bash -s -- --prefix ~/studio --init ~/videos/我的第一部.studio
```

**Windows**（MSVC 构建）

```powershell
irm https://raw.githubusercontent.com/sunsheng/video-studio/main/scripts/install.ps1 | iex

# 指定目录 + 顺带初始化
.\install.ps1 -Prefix D:\video-studio -Init D:\videos\我的第一部.studio
```

也可以直接下载 [Releases](https://github.com/sunsheng/video-studio/releases) 里的
`video-studio-<版本>-linux-x86_64.zip` 或 `-windows-x86_64.zip`，
解压到任意目录即可，程序不依赖安装位置。

## 快速开始

```bash
studio-cli doctor                       # 体检：ffmpeg / ComfyUI 可达性
studio-cli init ~/videos/千岛湖.studio   # 新建一部作品
cd ~/videos/千岛湖.studio && codex       # 打开它（studiod 被自动拉起）
studio-cli list ~/videos                # 看看都有哪些作品
```

之后就是对话：说创意 → 逐阶段确认 → 出片。想改就说「不要固定 2 秒」，Agent 会调 `studio.revise`。

## 一部作品长什么样

```
千岛湖.studio/
├── AGENTS.md                 # 运行时契约（init 时物化）
├── .agents/skills/           # 10 个阶段 Skill
├── .codex/config.toml        # 指向 studiod
├── project.toml              # 标题、版本、核心模型
├── .studio/                  # 服务端私有：状态库、日志、锁
├── stages/*.json             # 阶段产物，人可读、可进 Git
├── media/                    # 中间媒体
└── output/                   # 交付物
```

搬走、改名、复制都不影响——bundle 内部只用相对路径。

## 文档

- **[完整设计](docs/design.md)** —— 背景、核心决策、验收标准
  （[在线排版版](https://claude.ai/code/artifact/8a3fe961-bcec-4f92-9c38-183d2d4a4ade)）
- [架构](docs/architecture.md) · [状态机](docs/state-machine.md) · [工具面](docs/tool-surface.md) · [部署](docs/deployment.md)
- [观测与验收](docs/e2e.md) —— 两份报告（Agent 侧 / 执行侧）怎么出、怎么读
- [架构决策记录](docs/decisions/) —— 为什么是 bundle 模型、为什么拆成两个二进制
- [待办](docs/TODO.md) —— 还没做完的事，包括四份待核验的 workflow 基线

## License

MIT
