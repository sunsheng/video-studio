# video-studio 开发契约

本仓库用 Claude Code 开发，产物在 Codex 上运行。**开发环境和运行环境彻底分开**：
生产机器上只有一个二进制加若干 Markdown/配置，永远没有源码。

## 分层（由 crate 依赖强制）

```
studio-core     领域层。阶段图、typestate 状态机、错误枚举、契约类型、schema 校验。
                ★ 零 I/O 依赖：不得依赖 rusqlite / ureq / std::fs 之外的任何 I/O。
                  状态机逻辑必须能在没有 GPU、没有数据库的机器上纯单元测试跑完。
studio-store    SQLite 持久化。只被 engine 依赖。
studio-engine   阶段循环、确认门、恢复、产物登记。依赖 core + store。
studio-comfy    ComfyUI HTTP 客户端。★ 本机不需要 GPU，一切经 HTTP。
studio-media    ffmpeg / ffprobe 外部进程编排。
studio-mcp      MCP 协议层：工具注册表、schema、决策信封。依赖 engine。
studiod         唯一二进制：init / serve / doctor / emit-assets / pack / unpack。
```

反向依赖一律禁止。`studio-core` 新增依赖需要在 PR 描述里说明理由。

## 硬规则

1. **二进制不提供任何变更型子命令。** 只有 `init`、`serve`、`doctor`、`emit-assets`、
   `pack`、`unpack`。绝不允许出现 `studiod submit-stage` 这类东西——
   状态变更只有 MCP 一个入口，绕过就不存在实现。
2. **Markdown 不手写。** `assets/AGENTS.md` 与各 `SKILL.md` 中涉及工具名、阶段名、
   确认门、错误码的段落由 `studiod emit-assets` 生成。CI 跑 `emit-assets --check`。
3. **每个错误都必须有 remedy。** `StudioError::remedy()` 是穷尽 match，不允许 `_ =>`。
   没有 remedy 的错误视为实现缺陷。
4. **bundle 内一律相对路径。** 数据库、`project.toml`、stages JSON 里不得出现绝对路径。
5. **不引入第二种运行时语言。** 没有 Python、没有 Node。`scripts/` 里的 shell 只做引导。

## 工具链

`rust-toolchain.toml` 指定具体版本（不是 `stable`）。升级时改该文件，然后运行：
```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## 测试边界

### 本机必能通过（无环境依赖）

- `cargo test` —— 单元测试、状态机、schema 校验、MCP 一致性

### 本机有条件通过（需要外部依赖）

- **ffmpeg / ffprobe：** 如果本机装有，可运行 `studio-media` 的集成测试
- **ComfyUI + GPU：** 如果本机装有，可通过 HTTP 运行 `studio-comfy` 的集成测试
- **Codex 环境：** 如果本机满足 Codex 部署条件，可跑完整端到端测试

### 环境检测

- `studiod doctor` 检查本机环境（Codex、ComfyUI、ffmpeg、ffprobe 是否可用）
- 根据检测结果，选择性运行相应的集成测试
- **不得声称集成通过而不说明环境前置条件**

### 真实验收

- 生产环境集成验收在宿主机跑 `scripts/smoke.sh`
- 不得用 mock 通过来宣称链路跑通

## 提交

按实现节点提交，直接进 `main`，不走 PR。消息用中文，首行 `<类型>: <做了什么>`。
