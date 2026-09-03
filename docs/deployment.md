# 部署

## 产物

GitHub Actions 在 tag / 手动触发时构建 `video-studio-<version>-linux-x86_64.zip`：

```
video-studio/
├── studiod                 # 静态链接二进制
├── assets/{AGENTS.md, skills/, schema/, codex/}
├── config.toml             # 出厂默认
├── .env.example
├── VERSION
└── install.sh
```

解压到任意目录即可运行，程序不依赖安装位置（所有内部路径都相对二进制解析）。

## install.sh

```bash
install.sh [--prefix DIR] [--version TAG] [--init BUNDLE_PATH] [--force]
```

- `--prefix` 默认 `/opt/video-studio`；无写权限时自动提示用 sudo 或换目录
- **默认不执行 init**；`--init <path>` 才会顺带初始化一部作品
- `--version` 默认取最新 release

## 升级

覆盖安装目录即可。已有 bundle 保留建时物化的 AGENTS.md 与 skills，
行为不变；`studiod doctor --upgrade-assets` 由用户显式决定是否刷新。

## 运行前提

- **不需要 GPU**：推理全部经 ComfyUI HTTP API
- 需要 ffmpeg / ffprobe，可不在 PATH，见 `.env` 的 `FFMPEG_PATH` / `FFPROBE_PATH`
- 需要至少一个可达的 ComfyUI 节点，见 `.env` 的 `COMFY_NODES`

`studiod doctor` 会逐项检查并给出修复指引。
