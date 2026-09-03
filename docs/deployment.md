# 部署

## 产物

GitHub Actions 在 tag / 手动触发时构建两个平台的压缩包：

- `video-studio-<版本>-linux-x86_64.zip`（gnu 目标）
- `video-studio-<版本>-windows-x86_64.zip`（MSVC 目标）

两边内容一致，只是二进制与安装脚本不同（`install.sh` / `install.ps1`）：

```
video-studio/
├── studiod / studiod.exe
├── assets/{AGENTS.md, skills/, schema/, codex/}
├── config.toml             # 出厂默认
├── .env.example
├── VERSION
└── install.sh
```

解压到任意目录即可运行，程序不依赖安装位置（所有内部路径都相对二进制解析）。

## 安装脚本

```bash
install.sh [--prefix DIR] [--version TAG] [--init BUNDLE] [--force]     # Linux
install.ps1 [-Prefix DIR] [-Version TAG] [-Init BUNDLE] [-Force]        # Windows
```

- `--prefix` 默认 `/opt/video-studio`（Windows 为 `%LOCALAPPDATA%\video-studio`）；
  无写权限时提示用 sudo 或换目录
- **默认不执行 init**；`--init <路径>` 才会顺带初始化一部作品
- `--version` 默认取最新 release
- 脚本与二进制同目录时就地安装，不联网

### Windows 上的一个坑

作品里的 `.codex/config.toml` 要写程序的绝对路径。`C:\opt\studiod.exe` 放进 TOML
双引号串里，`\o` 是非法转义，Codex 读配置会直接报错。所以这里写的是 TOML
字面量字符串（单引号），不处理转义。有测试守着这条往返。

## 升级

覆盖安装目录即可。已有 bundle 保留建时物化的 AGENTS.md 与 skills，
行为不变；`studiod doctor --upgrade-assets` 由用户显式决定是否刷新。

## 运行前提

- **不需要 GPU**：推理全部经 ComfyUI HTTP API
- 需要 ffmpeg / ffprobe，可不在 PATH，见 `.env` 的 `FFMPEG_PATH` / `FFPROBE_PATH`
- 需要至少一个可达的 ComfyUI 节点，见 `.env` 的 `COMFY_NODES`

`studiod doctor` 会逐项检查并给出修复指引。
