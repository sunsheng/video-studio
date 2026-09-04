//! 随二进制分发的 Markdown 与配置，**全部由代码生成**。
//!
//! 这是让整套约束闭环的关键：涉及工具名、阶段名、确认门和错误码的段落
//! 都来自 [`studio_core`] 的类型定义与 [`studio_mcp::TOOLS`] 注册表，
//! 所以文档不可能引用到不存在的工具，Agent 也永远没有理由去翻源码。
//!
//! 前身项目的 AGENTS.md 里写着「工具清单的唯一事实源是 mcp_server.py 的
//! TOOLS」「确认门的事实源是 stage_graph.py」——那两句话把 Agent 直接
//! 指向了源码，而它照做了。这里反过来：文档就是代码的投影。

use studio_core::error::ERROR_CODES;
use studio_core::{schema, StageId, StageKind};
use studio_mcp::TOOLS;

/// 一个 Skill 的散文部分。可生成的段落不写在这里。
struct SkillDoc {
    name: &'static str,
    description: &'static str,
    stage: Option<StageId>,
    trigger: &'static str,
    not_trigger: &'static str,
    duties: &'static [&'static str],
    notes: &'static [&'static str],
}

const SKILLS: [SkillDoc; 10] = [
    SkillDoc {
        name: "idea",
        description: "把用户创意整理成可执行 brief，明确受众、平台、时长与发布风险。",
        stage: Some(StageId::Idea),
        trigger: "用户描述了一个想拍的东西，或者说「从头开始」「做一个新的」。",
        not_trigger: "已经有 brief 之后的任何阶段。",
        duties: &[
            "把口语化的创意转成结构化 brief：标题、logline、平台、受众、时长、镜头数、画幅。",
            "对模糊输入做出判断并**写进 assumptions**，不要私下假设也不要反复追问。",
            "识别发布风险并分级：可规避 / 需用户决定 / 不可接受。",
            "定义可验收的成功标准——后面的 review 阶段会照着它逐条核对。",
        ],
        notes: &[
            "这一阶段没有确认门，提交即通过。真正的第一道门在选题阶段。",
            "用户说「20色女性」这类明显笔误，按最合理的理解处理并在 assumptions 里写明，不要卡住不动。",
        ],
    },
    SkillDoc {
        name: "selection",
        description: "从可行性、受众匹配和发布风险筛选 brief，给出推荐方案与取舍。",
        stage: Some(StageId::Selection),
        trigger: "brief 已通过，需要决定往哪个方向做。",
        not_trigger: "创意本身还没成形；那是 idea 的事。",
        duties: &[
            "评估可行性：模型可控性、制作成本、需要牺牲什么。",
            "评估受众匹配：钩子强度、观看收益、留存设计。",
            "把发布风险分成可规避、不可接受、需用户决定三类。",
            "给出一个明确推荐，并说清楚推荐它牺牲了什么。",
        ],
        notes: &["确认门问的是方向，不是细节。细节留到剧本阶段再改。"],
    },
    SkillDoc {
        name: "script",
        description: "创建短视频的故事结构、节奏与声音时间线。",
        stage: Some(StageId::Script),
        trigger: "方向已确认，需要把它变成逐拍的内容。",
        not_trigger: "镜头语言、景别、机位——那是 director 的事。",
        duties: &[
            "按**内容**分配时长，不要平均切分。动作复杂、信息量大的拍给更多时间。",
            "各拍时长必须精确合计到 brief 规定的总时长。",
            "同时给出声音时间线：有口播就写台词，没有就写环境声与拟音的来源。",
            "字幕策略要明确。没有字幕也要写清楚是「本版无字幕」。",
        ],
        notes: &[
            "「不要固定 2 秒」这类反馈直接调 studio.revise，然后重新提交。不需要先解除任何占用。",
            "总时长对不上是最常见的退回原因，提交前自己加一遍。",
        ],
    },
    SkillDoc {
        name: "director",
        description: "把已确认剧本转成逐镜头分镜，定义景别、构图、机位、灯光与时长。",
        stage: Some(StageId::Storyboard),
        trigger: "剧本已确认，需要把每一拍变成可拍的镜头。",
        not_trigger: "还在讨论故事讲什么；那是 script 的事。",
        duties: &[
            "每个镜头一个主动作、一个主镜头运动。两个以上的运动会让生成结果失控。",
            "写清动作链（起 → 承 → 收）、首帧与尾帧，转场要能被审计。",
            "锁定角色连续性：外观、服装、机位签名，逐镜保持一致。",
            "安全约束写进分镜本身，而不是留给后面的阶段补救。",
        ],
        notes: &["镜头时长必须与剧本各拍对齐；改了时长就要回到剧本阶段改。"],
    },
    SkillDoc {
        name: "visual",
        description: "规划并生成一致的角色卡、场景卡与参考资产。",
        stage: Some(StageId::VisualAssets),
        trigger: "分镜已确认，需要先把跨镜头复用的视觉资产定下来。",
        not_trigger: "逐镜头的提示词；那是 prompt 的事。",
        duties: &[
            "为跨镜头复用的角色、场景、道具各建一张卡，给稳定的 asset_id。",
            "写明一致性锁定：角色外观、机位签名、环境、排版禁止项。",
            "核心系列没有独立静态图 workflow 时，先生成开发片段再抽帧，并保留抽帧参数。",
            "降级策略写死：核心系列不可用就结构化阻塞，不自动换系列。",
        ],
        notes: &["这是 hybrid 阶段：你定资产计划，确认之后由控制面执行生成。"],
    },
    SkillDoc {
        name: "prompt",
        description: "把已确认分镜与视觉资产编译成逐镜头 prompt 和 workflow 参数。",
        stage: Some(StageId::PromptPack),
        trigger: "视觉资产已确认，准备进入渲染。",
        not_trigger: "画面内容本身还在改；那是 director 的事。",
        duties: &[
            "逐镜头给出正向、负向提示词，以及尺寸、帧数、帧率、种子。",
            "种子必须固定并记录，否则结果不可复现。",
            "workflow 名必须是已验证基线里的，不要临时编一个。",
            "引用视觉资产用 asset_id，不要重复描述角色外观。",
        ],
        notes: &["这道门是花 GPU 时间之前的最后一关。确认之后就开始烧显卡了，提交前自己再读一遍。"],
    },
    SkillDoc {
        name: "comfyui",
        description: "提交已确认且通过校验的 workflow，选择健康节点、跟踪执行并登记输出。",
        stage: Some(StageId::Render),
        trigger: "提示词包已确认，控制面开始渲染。",
        not_trigger: "任何创作判断。",
        duties: &[
            "这是确定性阶段，由控制面执行，你只需要用 studio.status 观察。",
            "失败时读 studio.timeline 看清是哪一镜、哪个节点、什么原因。",
            "节点不可用或模型契约不满足时会结构化阻塞——不要建议换模型来绕过。",
        ],
        notes: &["运行控制面的机器不需要 GPU，一切经 ComfyUI 的 HTTP API 完成。"],
    },
    SkillDoc {
        name: "post",
        description: "把生成片段拼接为交付视频，处理字幕、音频、封面。",
        stage: Some(StageId::Post),
        trigger: "渲染完成，需要出成片。",
        not_trigger: "镜头本身要重做；那要回到更早的阶段。",
        duties: &[
            "按分镜顺序拼接，转场必须与分镜里写的一致。",
            "字幕只能来自已确认的剧本文本，不要在这一步新编。",
            "封面从成片里抽帧，不要另外生成一张对不上的图。",
        ],
        notes: &["这是确定性阶段，由控制面执行。ffmpeg 不要求在 PATH 中，配置见 .env。"],
    },
    SkillDoc {
        name: "review",
        description: "检查成片的媒体完整性、时长、字幕、编码与发布风险。",
        stage: Some(StageId::Review),
        trigger: "后期完成，需要验收。",
        not_trigger: "创作质量的主观评价。",
        duties: &[
            "每一条检查都必须基于 ffprobe 的**实测**元数据，不能靠推断。",
            "逐条核对 idea 阶段定下的 success_metrics。",
            "任一必需项缺失就判不通过，不要为了让流程走完而放水。",
        ],
        notes: &["验收通过不等于可以发布。对外发布需要另行获得授权。"],
    },
    SkillDoc {
        name: "run-management",
        description: "解释当前作品的状态，走修订与恢复路径。",
        stage: None,
        trigger: "用户问「现在到哪了」「怎么改」「重做某一步」，或者遇到阻塞需要判断下一步。",
        not_trigger: "各阶段的创作执行本身。",
        duties: &[
            "先调 studio.status。信封里的 next_action 和 blocked_by 已经说清了该做什么。",
            "阻塞时照 blocked_by.remedy 做。它一定会指向一个能调的工具。",
            "用户提出修改意见就调 studio.revise——它不会失败，也不需要先解除什么占用。",
            "作品的历史看 studio.timeline，某一步的产物看 studio.stage_output。",
        ],
        notes: &[
            "**没有 run_id**。你打开的这个目录就是当前作品，工具也都不收项目参数。",
            "新建、继续、修订之外没有别的动作。列表用 `ls`，另存为用 `cp -r`，删除用 `rm -rf`。",
        ],
    },
];

fn stage_table() -> String {
    let mut s = String::from("| # | 阶段 | 能力 | 类型 | 确认门 |\n|---|---|---|---|---|\n");
    for (i, stage) in StageId::all().enumerate() {
        let kind = match stage.kind() {
            StageKind::Creative => "creative（你产出全部内容）",
            StageKind::Hybrid => "hybrid（你定内容，控制面执行）",
            StageKind::Deterministic => "deterministic（控制面执行，你只观察）",
        };
        s.push_str(&format!(
            "| {} | `{}` | `{}` | {} | {} |\n",
            i + 1,
            stage,
            stage.capability(),
            kind,
            stage
                .gate()
                .map(|g| format!("`{g}`"))
                .unwrap_or_else(|| "—".into())
        ));
    }
    s
}

fn tool_table() -> String {
    let mut s = String::from("| 工具 | 作用 |\n|---|---|\n");
    for t in TOOLS.iter() {
        let desc = t
            .description
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        s.push_str(&format!("| `{}` | {} |\n", t.name, desc));
    }
    s
}

fn error_table() -> String {
    let mut s = String::from("| 错误码 | 含义 |\n|---|---|\n");
    for (code, desc) in ERROR_CODES {
        s.push_str(&format!("| `{code}` | {desc} |\n"));
    }
    s
}

/// 生成 bundle 根目录的 AGENTS.md。
pub fn agents_md() -> String {
    format!(
        r#"<!-- 本文件由代码生成，请勿手改。 -->

# 这部作品

你现在打开的这个文件夹**就是一部作品**。像一份 .docx：新建、继续、修订，
只有这三个动作。没有项目列表，没有 run id，工具也都不收项目参数。

- 想看另一部作品：退出，`cd` 到那个文件夹再打开。
- 想另存一版：`cp -r 这个目录 另一个名字.studio`。
- 想归档、打包或发给别人：这些超出你的能力范围，提醒用户自己在终端处理，
  不要代劳。

## 你只能通过 Studio MCP 改变状态

创作判断由你来做，状态由控制面持有。**不要**用 shell 去读写这个目录里的
状态。你能看到的只有这份 MCP 工具面——没有能推进阶段的命令行，因为
状态变更只有 MCP 一个入口。

`.studio/` 是控制面私有的，里面是状态库、日志和锁。不要读，不要改。
它有完整性校验，外部改动会在下一次调用时以 `state_drift` 暴露出来。

## 三条工作习惯

1. **不确定就先调 `studio.status`。** 信封里的 `next_action` 说了下一步交什么，
   `pending_question` 说了在等用户答什么，`blocked_by` 说了被什么挡住。
2. **提交前先调 `studio.schema`，不要猜字段。** 也不要参考别处的产物——
   这个目录里没有别的作品，schema 才是唯一事实源。
3. **被挡住时照 `blocked_by.remedy` 做。** 每一条阻塞都带着可执行的下一步。
   如果 remedy 说不通，那是控制面的缺陷，报告出来，不要绕过去。

## 阶段与确认门

{stage_table}

门在阶段**产出之后**暂停。`prompt_pack` 那道门是花 GPU 时间之前的最后一关。

确认门的选项要自己声明 `outcome`：`approve` 通过并进入下一阶段，
`revise` 把本阶段打回草稿。不要靠选项 id 的字面意思去暗示，控制面只认 `outcome`。

## 修订

用户提出修改意见时调 `studio.revise(stage, message)`。它**不会失败**，
也不需要先解除任何占用——提交、修订、再提交是一条顺畅的路径。

修订会让作品的进度整体退回到那个阶段：**它之后的阶段一律变回未执行**。
分镜是照旧剧本做的，剧本一改它就不再成立。旧产物文件留着，你可以用
`studio.stage_output` 读出来参考，重新提交时直接覆盖。

程序不做版本管理。要留版本请让用户 `cp -r`，或提醒他们自己在终端打包。

## 工具

{tool_table}

## 错误码

{error_table}

## 用户在说什么

- 「从头开始」「做一个新的」→ 这是一部新作品，让用户自己在终端新建一个
  目录。不要在当前作品里覆盖着做。
- 「继续」「下一步」「现在到哪了」→ 调 `studio.status`。上下文不在对话里，在这个文件夹里。
- 「改一下 X」→ 调 `studio.revise`，然后重新提交。
"#,
        stage_table = stage_table(),
        tool_table = tool_table(),
        error_table = error_table(),
    )
}

/// 生成一个 Skill 的 SKILL.md。
fn skill_md(doc: &SkillDoc) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "---\nname: {}\ndescription: {}\n---\n\n<!-- 本文件由代码生成，请勿手改。 -->\n\n",
        doc.name, doc.description
    ));
    s.push_str(&format!("# {} Skill\n\n", doc.name));
    s.push_str(&format!("触发：{}\n\n", doc.trigger));
    s.push_str(&format!("不触发：{}\n\n", doc.not_trigger));

    s.push_str("## 职责\n\n");
    for d in doc.duties {
        s.push_str(&format!("- {d}\n"));
    }
    s.push('\n');

    if let Some(stage) = doc.stage {
        s.push_str("## 输入输出\n\n");
        s.push_str(&format!(
            "本阶段的产物放在 `outputs` 的顶层键 `{key}` 下。**提交前先调 `studio.schema(\"{stage}\")`** \
             取回完整契约，不要凭印象填字段。必填项是：\n\n",
            key = stage.output_key(),
        ));
        if let studio_core::schema::Schema::Object { required, .. } = schema::stage_schema(stage) {
            for r in required {
                s.push_str(&format!("- `{}.{}`\n", stage.output_key(), r));
            }
        }
        s.push('\n');
        s.push_str(
            "上游产物由 `studio.status` 的 `next_action.inputs` 给出，不需要你去别处找。\n\n",
        );

        s.push_str("## 确认点\n\n");
        match stage.gate() {
            Some(gate) => s.push_str(&format!(
                "本阶段有确认门 `{gate}`。提交时必须同时给出 `confirmation`：\
                 一句问用户的话，加上至少一个 `outcome: approve` 的选项和一个 `outcome: revise` 的选项。\n\n\
                 用户选了 revise 类选项，控制面会自动把阶段打回草稿；\
                 用户是用自然语言提意见（而不是点选项），就调 `studio.revise`。\n\n"
            )),
            None => s.push_str("本阶段没有确认门，提交即通过。\n\n"),
        }
    }

    s.push_str("## 失败与恢复\n\n");
    s.push_str(
        "任何工具返回的 `blocked_by` 都带着 `remedy`，照它做。\
         schema 不合规时 `message` 会精确指到出错的字段路径，例如 \
         `script.story_arc[1].duration_seconds`。\n\n",
    );

    if !doc.notes.is_empty() {
        s.push_str("## 注意\n\n");
        for n in doc.notes {
            s.push_str(&format!("- {n}\n"));
        }
        s.push('\n');
    }

    s.push_str("## Studio MCP\n\n");
    s.push_str("可用工具（全部不带 run_id，当前目录就是当前作品）：\n\n");
    for t in TOOLS.iter() {
        s.push_str(&format!("- `{}`\n", t.name));
    }
    s
}

/// `.codex/config.toml`：把 MCP server 指向 `studiod` 的位置。
/// `studiod` 没有子命令、不接受参数——唯一行为就是 serve。
pub fn codex_config(studiod_path: &str) -> String {
    format!(
        "# 由 `studio-cli init` 生成。换机器或换安装位置后跑\n\
         # `studio-cli doctor --fix` 修正这里。\n\
         [mcp_servers.video-studio]\n\
         command = {}\n",
        toml_path(studiod_path)
    )
}

/// 把一个路径写成合法的 TOML 字符串。
///
/// Windows 的 `C:\opt\studiod.exe` 放进双引号串里，`\o` 是非法转义，
/// Codex 读配置时会直接报错。TOML 的字面量字符串（单引号）不处理转义，
/// 正好合适；路径里真出现单引号时再退回双引号并转义反斜杠。
fn toml_path(p: &str) -> String {
    if p.contains('\'') {
        format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{p}'")
    }
}

/// `project.toml`：作品自己的元信息。
pub fn project_toml(title: &str, version: &str, core_model: &str) -> String {
    format!(
        "# 这部作品的元信息。内部一律相对路径，整个目录可以随意 mv / cp -r。\n\
         title = \"{title}\"\n\
         created_with = \"{version}\"\n\
         core_model_family = \"{core_model}\"\n"
    )
}

pub fn env_example() -> String {
    r#"# video-studio 运行时配置。
# bundle 里的 .env 优先级最高，其次是程序目录的 .env，然后是进程环境变量。

# ffmpeg / ffprobe 不要求在 PATH 中，可在这里直接指向可执行文件。
# FFMPEG_PATH=/usr/local/bin/ffmpeg
# FFPROBE_PATH=/usr/local/bin/ffprobe

# ComfyUI 节点。运行本程序的机器不需要 GPU，也可以指向另一台主机。
# COMFY_NODES=http://127.0.0.1:9001,http://127.0.0.1:9002
# COMFY_TIMEOUT_SECS=1800
# COMFY_POLL_INTERVAL_SECS=3

# 核心模型系列。
# CORE_MODEL_FAMILY=minimax_h3
"#
    .to_string()
}

/// 全部随包分发的资产：`(相对路径, 内容)`。
pub fn all_assets() -> Vec<(String, String)> {
    let mut out = vec![("AGENTS.md".to_string(), agents_md())];
    for doc in SKILLS.iter() {
        out.push((format!("skills/{}/SKILL.md", doc.name), skill_md(doc)));
    }
    for stage in StageId::all() {
        let json =
            serde_json::to_string_pretty(&schema::stage_schema_document(stage)).unwrap_or_default();
        out.push((format!("schema/{stage}.json"), format!("{json}\n")));
    }
    out.push((".env.example".to_string(), env_example()));
    out
}

/// `studio-cli init` 要在 bundle 里物化的文件。
pub fn bundle_files(
    studiod_path: &str,
    title: &str,
    version: &str,
    core_model: &str,
) -> Vec<(String, String)> {
    let mut out = vec![
        ("AGENTS.md".to_string(), agents_md()),
        (".codex/config.toml".to_string(), codex_config(studiod_path)),
        (
            "project.toml".to_string(),
            project_toml(title, version, core_model),
        ),
        (".env.example".to_string(), env_example()),
    ];
    for doc in SKILLS.iter() {
        out.push((
            format!(".agents/skills/{}/SKILL.md", doc.name),
            skill_md(doc),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_md_never_points_at_source_code() {
        let md = agents_md();
        for leak in ["stage_graph.rs", "mcp_server.rs", "src/", "crates/", ".py"] {
            assert!(
                !md.contains(leak),
                "AGENTS.md 不该把 Agent 指向源码：{leak}"
            );
        }
    }

    /// `studiod`/`studio-cli` 不该出现在 Agent 能读到的文档里——见
    /// docs/decisions/ADR-0002。哪怕是「让用户自己跑 xxx」这种场景，
    /// 只要 Agent 知道命令名和语法，沙箱允许 shell 时它就有能力自己跑，
    /// 绕过 MCP。
    #[test]
    fn agents_md_never_names_the_cli_binaries() {
        let md = agents_md();
        for leak in ["studiod", "studio-cli"] {
            assert!(!md.contains(leak), "AGENTS.md 不该提到二进制名：{leak}");
        }
    }

    #[test]
    fn agents_md_lists_every_tool_and_stage() {
        let md = agents_md();
        for t in TOOLS.iter() {
            assert!(md.contains(t.name), "AGENTS.md 缺少工具 {}", t.name);
        }
        for s in StageId::all() {
            assert!(md.contains(s.as_str()), "AGENTS.md 缺少阶段 {s}");
        }
        for (code, _) in ERROR_CODES {
            assert!(md.contains(code), "AGENTS.md 缺少错误码 {code}");
        }
    }

    #[test]
    fn agents_md_states_the_rewind_rule() {
        let md = agents_md();
        assert!(
            md.contains("一律变回未执行"),
            "必须说清修订会让下游退回未执行"
        );
        assert!(md.contains(".studio/"), "必须说清 .studio/ 不要碰");
    }

    /// 每个 skill 只能引用真实存在的工具名——这是「文档由代码生成」的意义所在。
    #[test]
    fn skills_only_reference_real_tools() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        for doc in SKILLS.iter() {
            let md = skill_md(doc);
            let mut idx = 0;
            while let Some(pos) = md[idx..].find("studio.") {
                let start = idx + pos;
                let rest = &md[start..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
                    .unwrap_or(rest.len());
                let candidate = &rest[..end];
                assert!(
                    names.contains(&candidate),
                    "{} 引用了不存在的工具 {candidate}",
                    doc.name
                );
                idx = start + end;
            }
        }
    }

    #[test]
    fn every_stage_skill_lists_its_required_fields() {
        for doc in SKILLS.iter() {
            let Some(stage) = doc.stage else { continue };
            let md = skill_md(doc);
            assert!(
                md.contains(&format!("studio.schema(\"{stage}\")")),
                "{} 应提示先取 schema",
                doc.name
            );
            if let studio_core::schema::Schema::Object { required, .. } =
                schema::stage_schema(stage)
            {
                for r in required {
                    assert!(md.contains(r), "{} 缺少必填项 {r}", doc.name);
                }
            }
        }
    }

    #[test]
    fn skill_names_match_the_capability_map() {
        let mut names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        names.sort();
        let mut expected: Vec<&str> = studio_core::stage::SKILL_NAMES.to_vec();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn gated_skills_explain_the_confirmation_contract() {
        for doc in SKILLS.iter() {
            let Some(stage) = doc.stage else { continue };
            let md = skill_md(doc);
            if let Some(gate) = stage.gate() {
                assert!(md.contains(gate), "{} 应写明确认门 {gate}", doc.name);
                assert!(
                    md.contains("outcome"),
                    "{} 应说明选项要声明 outcome",
                    doc.name
                );
            } else {
                assert!(md.contains("没有确认门"), "{} 应说明本阶段无门", doc.name);
            }
        }
    }

    #[test]
    fn assets_cover_agents_ten_skills_and_nine_schemas() {
        let a = all_assets();
        assert_eq!(
            a.iter().filter(|(p, _)| p.starts_with("skills/")).count(),
            10
        );
        assert_eq!(
            a.iter().filter(|(p, _)| p.starts_with("schema/")).count(),
            9
        );
        assert!(a.iter().any(|(p, _)| p == "AGENTS.md"));
        assert!(a.iter().all(|(_, c)| !c.is_empty()));
    }

    /// Windows 路径不能写进 TOML 的双引号串——`C:\opt` 里的 `\o` 是非法转义。
    #[test]
    fn windows_paths_survive_the_toml_round_trip() {
        for path in [
            r"C:\opt\video-studio\studiod.exe",
            r"C:\Users\孙\video-studio\studiod.exe",
            "/opt/video-studio/studiod",
        ] {
            let cfg = codex_config(path);
            let parsed: toml::Value = toml::from_str(&cfg)
                .unwrap_or_else(|e| panic!("{path} 生成的配置解析不了：{e}\n{cfg}"));
            assert_eq!(
                parsed["mcp_servers"]["video-studio"]["command"]
                    .as_str()
                    .unwrap(),
                path,
                "路径经过 TOML 往返之后必须一模一样"
            );
        }
    }

    #[test]
    fn bundle_files_land_in_the_places_codex_looks() {
        let f = bundle_files("/opt/video-studio/studiod", "千岛湖", "0.1.0", "minimax_h3");
        assert!(f.iter().any(|(p, _)| p == "AGENTS.md"));
        assert!(f.iter().any(|(p, _)| p == ".codex/config.toml"));
        assert_eq!(
            f.iter()
                .filter(|(p, _)| p.starts_with(".agents/skills/"))
                .count(),
            10
        );
        let cfg = &f.iter().find(|(p, _)| p == ".codex/config.toml").unwrap().1;
        assert!(cfg.contains("/opt/video-studio/studiod"));
        assert!(!cfg.contains("args"), "studiod 没有子命令，不需要 args");
    }

    /// 同上，但对全部随包文档（含每份 SKILL.md）一起查，不只是 AGENTS.md。
    #[test]
    fn no_bundle_doc_names_the_cli_binaries() {
        let f = bundle_files("/opt/video-studio/studiod", "千岛湖", "0.1.0", "minimax_h3");
        for (path, content) in &f {
            if path == ".codex/config.toml" {
                continue; // 这个文件本来就要指向 studiod，规则不适用。
            }
            for leak in ["studiod", "studio-cli"] {
                assert!(!content.contains(leak), "{path} 不该提到二进制名：{leak}");
            }
        }
    }
}
