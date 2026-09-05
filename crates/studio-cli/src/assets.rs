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
use studio_core::{fixtures, lexicon, schema, StageId, StageKind};
use studio_mcp::TOOLS;

/// 方法层：**怎么想**，区别于 SKILL.md 管的**交什么**。
///
/// 散文是源文件（跟着代码走、参与 review），词表和样例由代码插进去——
/// 手写的词表迟早会和 schema 的 `enum` 对不上，手抄的样例迟早会和契约漂移。
/// 占位符形如 `<!-- 词表:camera_motion -->` 与 `<!-- 样例:storyboard -->`。
///
/// 全部按需加载：SKILL.md 只给索引，Agent 用到哪份读哪份。默认一份都不进
/// 上下文——前身项目把 30 行模型文件名塞进每个会话的教训在这里同样适用。
const DOCTRINE: [(&str, &str); 12] = [
    ("README.md", include_str!("../assets/doctrine/README.md")),
    (
        "story/structure.md",
        include_str!("../assets/doctrine/story/structure.md"),
    ),
    (
        "story/concepts.md",
        include_str!("../assets/doctrine/story/concepts.md"),
    ),
    (
        "story/hook.md",
        include_str!("../assets/doctrine/story/hook.md"),
    ),
    (
        "story/voice.md",
        include_str!("../assets/doctrine/story/voice.md"),
    ),
    (
        "camera/grammar.md",
        include_str!("../assets/doctrine/camera/grammar.md"),
    ),
    (
        "camera/lighting.md",
        include_str!("../assets/doctrine/camera/lighting.md"),
    ),
    (
        "camera/blocking.md",
        include_str!("../assets/doctrine/camera/blocking.md"),
    ),
    (
        "consistency/bible.md",
        include_str!("../assets/doctrine/consistency/bible.md"),
    ),
    (
        "audio/design.md",
        include_str!("../assets/doctrine/audio/design.md"),
    ),
    (
        "failure/modes.md",
        include_str!("../assets/doctrine/failure/modes.md"),
    ),
    (
        "exemplars/script.md",
        include_str!("../assets/doctrine/exemplars/script.md"),
    ),
];

/// 黄金样例：批注是散文，产物本身从 fixtures 来。
const EXEMPLARS: [(&str, StageId, &str); 2] = [
    (
        "exemplars/storyboard.md",
        StageId::Storyboard,
        include_str!("../assets/doctrine/exemplars/storyboard.md"),
    ),
    (
        "exemplars/prompt_pack.md",
        StageId::PromptPack,
        include_str!("../assets/doctrine/exemplars/prompt_pack.md"),
    ),
];

/// 一个模型系列的能力卡。
///
/// 「可注入参数」这一栏是**基线的投影**：写了基线没绑定的参数会被静默跳过，
/// 不报错也不生效。这张表就是为了让 Agent 提前知道哪些字段白写。
/// 一个测试守着它与 `assets/workflows/<family>/<mode>.json` 的
/// `_studio.bindings` 完全一致——基线一改，测试就红。
struct ModelCard {
    family: &'static str,
    title: &'static str,
    /// `(模式, 可注入参数, 是否已核验)`
    modes: &'static [(&'static str, &'static [&'static str], bool)],
    prose: &'static str,
}

const MODEL_CARDS: [ModelCard; 3] = [
    ModelCard {
        family: "minimax_h3",
        title: "MiniMax 系列（默认核心系列）",
        modes: &[
            (
                "t2v",
                &[
                    "positive",
                    "width",
                    "height",
                    "length_frames",
                    "fps",
                    "seed",
                ],
                true,
            ),
            (
                "i2v",
                &[
                    "positive",
                    "width",
                    "height",
                    "length_frames",
                    "fps",
                    "seed",
                ],
                true,
            ),
            (
                "r2v",
                &[
                    "positive",
                    "width",
                    "height",
                    "length_frames",
                    "fps",
                    "seed",
                ],
                true,
            ),
        ],
        prose: include_str!("../assets/models/minimax_h3.md"),
    },
    ModelCard {
        family: "wan2_2",
        title: "Wan 系列",
        modes: &[
            (
                "t2v",
                &[
                    "positive",
                    "negative",
                    "width",
                    "height",
                    "length_frames",
                    "fps",
                    "seed",
                ],
                true,
            ),
            ("i2v", &["seed"], false),
            ("flf2v", &["seed"], false),
        ],
        prose: include_str!("../assets/models/wan2_2.md"),
    },
    ModelCard {
        family: "ltx2_5",
        title: "LTX 系列",
        modes: &[
            (
                "t2v",
                &[
                    "positive",
                    "negative",
                    "width",
                    "height",
                    "duration_seconds",
                    "fps",
                    "seed",
                ],
                true,
            ),
            (
                "i2v",
                &[
                    "positive",
                    "negative",
                    "width",
                    "height",
                    "duration_seconds",
                    "fps",
                    "seed",
                ],
                true,
            ),
        ],
        prose: include_str!("../assets/models/ltx2_5.md"),
    },
];

/// 这个 Skill 该读哪几份方法文档。路径是 bundle 内的相对路径。
///
/// **不是「全部读一遍」**：默认一份都不加载，用到哪份读哪份。
fn doctrine_for(skill: &str) -> &'static [&'static str] {
    match skill {
        "idea" => &[
            ".agents/doctrine/story/concepts.md",
            ".agents/doctrine/story/hook.md",
            ".agents/doctrine/story/structure.md",
        ],
        "selection" => &[
            ".agents/doctrine/story/concepts.md",
            ".agents/doctrine/story/hook.md",
        ],
        "script" => &[
            ".agents/doctrine/story/structure.md",
            ".agents/doctrine/story/voice.md",
            ".agents/doctrine/audio/design.md",
            ".agents/doctrine/exemplars/script.md",
        ],
        "director" => &[
            ".agents/doctrine/camera/grammar.md",
            ".agents/doctrine/camera/blocking.md",
            ".agents/doctrine/camera/lighting.md",
            ".agents/doctrine/consistency/bible.md",
            ".agents/doctrine/audio/design.md",
            ".agents/doctrine/exemplars/storyboard.md",
        ],
        "visual" => &[".agents/doctrine/consistency/bible.md"],
        // 能力卡是这个阶段最要紧的一份：写了基线没绑定的参数会被静默丢弃。
        // 目录会在生成时展开成逐个文件，只读要用的那个系列即可。
        "prompt" => &[
            ".agents/models/",
            ".agents/doctrine/exemplars/prompt_pack.md",
            ".agents/doctrine/consistency/bible.md",
            ".agents/doctrine/quality/banned.md",
        ],
        "comfyui" => &[".agents/doctrine/failure/modes.md"],
        "review" => &[".agents/doctrine/quality/checklist.md"],
        "run-management" => &[".agents/doctrine/failure/modes.md"],
        _ => &[],
    }
}

/// 提交之前逐条过的自检项。阶段名从阶段图来，条目写在这里。
fn checklist(stage: StageId) -> &'static [&'static str] {
    match stage {
        StageId::Idea => &[
            "至少两个方案，且互斥：选了一个，另一个独有的东西就拍不进去了",
            "各方案的 angle、hook_0_3s、story_beats 都不同，不是换了说法的同一拍法",
            "各方案的 tradeoff 各不相同，没有三条都写成需求本身的约束",
            "钩子在前 3 秒内成立，且说得出具体是什么画面",
            "对模糊输入的判断写进了 assumptions，没有私下假设也没有反复追问",
            "success_metrics 每一条都能被验收，不是「效果好」这类说法",
        ],
        StageId::Selection => &[
            "每个方案都单独评过，没有只写推荐那个",
            "recommendation 指向的 concept_id 确实在候选里",
            "推荐说清了牺牲什么，不是只讲优点",
            "风险分成可规避 / 需用户决定 / 不可接受三类",
        ],
        StageId::Script => &[
            "各拍时长之和精确等于总时长（自己加一遍）",
            "时长按内容分配，timing_rule 写的是依据不是结果",
            "每一拍都说得出自己的 beat_type，且不与上一拍重复",
            "无口播也写清了环境声来源与字幕策略",
        ],
        StageId::Storyboard => &[
            "每镜说得出 shot_function，说不出就删掉这一镜",
            "每镜三条物理事实齐全，且都是拍得出来的",
            "每镜只有一个主运镜，且落在受控词表里",
            "镜头时长与剧本各拍对齐，总和一致",
            "角色外观串逐镜逐字相同（复制粘贴，不要复述）",
            "每镜的 audio 都写了，没有留空",
        ],
        StageId::VisualAssets => &[
            "每个跨镜头复用的角色、场景、道具都有卡",
            "一致性锁定写明了外观、机位签名、环境与排版禁止项",
        ],
        StageId::PromptPack => &[
            "逐项对照能力卡：写的每个参数这条基线都吃",
            "不支持负向提示词的系列，约束改写成了正向的完整句子",
            "身份锁在每一镜里逐字出现，没有写成「同一位…」",
            "没有禁用词（cinematic / 电影感 / 唯美这类）",
            "种子固定并记录，尺寸与帧数按各镜时长算准",
            "audio 写了三层，没有放弃原生音频",
        ],
        StageId::Preview | StageId::Render | StageId::Post | StageId::Review => &[],
    }
}

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
            "给出 **2–3 个互斥方案**：平台受众时长这些由需求定死的共用，\
             各方案不同的是切入角度、前三秒钩子和节拍走向。互斥的判据见 concepts.md。",
            "每个方案写清选它要牺牲什么，且各方案牺牲的不是同一件事。",
            "对模糊输入做出判断并**写进 assumptions**，不要私下假设也不要反复追问。",
            "识别发布风险并分级：可规避 / 需用户决定 / 不可接受。",
            "定义可验收的成功标准——后面的 review 阶段会照着它逐条核对。",
        ],
        notes: &[
            "这一阶段没有确认门，提交即通过。真正的第一道门在选题阶段，\
             用户在那里从你给的方案里挑一个——只给一个方案，那道门就退化成「同意 / 重来」。",
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
            "**逐个**评估上一阶段给出的每个方案，不要只写推荐的那个——\
             用户要看的是比较，不是结论。",
            "每个方案都评三样：可行性（模型可控性、制作成本）、\
             受众匹配（钩子强度、观看收益、留存）、风险。",
            "把发布风险分成可规避、不可接受、需用户决定三类。",
            "给出一个明确推荐（`recommendation.concept_id` 指向候选之一），\
             并说清楚推荐它牺牲了什么。",
        ],
        notes: &[
            "确认门在这里把你的候选列表原样摆给用户选。\
             用户可能不选你推荐的那个——这是设计如此，不是异常。",
            "被选中的方案会记进产物的 `_gate_choice`，后面的阶段照它写，不要回头改主意。",
            "确认门问的是方向，不是细节。细节留到剧本阶段再改。",
        ],
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
            "怀疑是某个节点本身有问题（反复失败、迟迟连不上），\
             先调 studio.comfy.exclude_node 把它排除，再重试。",
            "执行失败但内容没问题（节点抖动、连接超时）时调 studio.retry_stage，\
             不要用 studio.revise——那是给内容要改的场景用的。",
        ],
        notes: &[
            "运行控制面的机器不需要 GPU，一切经 ComfyUI 的 HTTP API 完成。",
            "孤立的一次轮询连接超时不代表渲染失败：控制面会自动容错重试，\
             只有连续失败或总耗时超过 timeout 才会真正报错。",
        ],
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

/// 一张词表的 Markdown 表格。
fn vocabulary_table(name: &str) -> String {
    let Some(rows) = lexicon::vocabulary(name) else {
        return String::new();
    };
    let mut s = format!("| `{name}` 取值 | 说明 |\n|---|---|\n");
    for (value, label) in rows {
        s.push_str(&format!("| `{value}` | {label} |\n"));
    }
    s
}

/// 分镜运镜到 MiniMax 运镜指令的对照表。
fn minimax_camera_table() -> String {
    let mut s = String::from("| 分镜的 `camera_motion` | 提示词里写 | 说明 |\n|---|---|---|\n");
    for motion in lexicon::CAMERA_MOTIONS {
        let command = lexicon::minimax_camera_command(motion).unwrap_or("");
        let label = lexicon::CAMERA_MOTION_LABELS
            .iter()
            .find(|(k, _)| *k == motion)
            .map(|(_, v)| *v)
            .unwrap_or("");
        s.push_str(&format!("| `{motion}` | `{command}` | {label} |\n"));
    }
    s
}

/// 把一份散文里的占位符换成生成内容。
///
/// 认两种：`<!-- 词表:<名字> -->` 和 `<!-- 样例:<阶段> -->`。
/// 换不掉的占位符会原样留下，由测试抓出来——静默留下一个空洞比报错更难查。
fn fill_placeholders(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let trimmed = line.trim();
        let filled = match trimmed
            .strip_prefix("<!-- 词表:")
            .and_then(|r| r.strip_suffix(" -->"))
        {
            Some("minimax_camera") => Some(minimax_camera_table()),
            Some(name) if lexicon::vocabulary(name).is_some() => Some(vocabulary_table(name)),
            _ => None,
        };
        let filled = filled.or_else(|| {
            trimmed
                .strip_prefix("<!-- 样例:")
                .and_then(|r| r.strip_suffix(" -->"))
                .and_then(StageId::parse)
                .map(exemplar_json)
        });
        match filled {
            Some(block) => out.push_str(block.trim_end()),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// 某个阶段的黄金样例，取自契约样例本身，所以不会和 schema 漂移。
fn exemplar_json(stage: StageId) -> String {
    let outputs = fixtures::outputs(stage);
    let json = serde_json::to_string_pretty(&outputs).unwrap_or_default();
    format!("```json\n{json}\n```")
}

/// 禁用词表。取自代码里的词表，不手写。
fn banned_md() -> String {
    let mut s = String::from(
        "# 禁用词\n\n\
         这些词的共同点：**它们不描述任何可拍的东西**。\
         对模型是噪声，对人是废话。\n\n\
         ## Tier 1：出现即改\n\n\
         写了就是没写。把它们换成具体的、能被摄影机记录的描述。\n\n",
    );
    for w in lexicon::BANNED_TIER1 {
        s.push_str(&format!("- `{w}`\n"));
    }
    s.push_str(
        "\n## Tier 2：同一段里出现两个以上就是问题\n\n\
         单独用未必错，堆在一起就是形容词汤。\n\n",
    );
    for w in lexicon::BANNED_TIER2 {
        s.push_str(&format!("- `{w}`\n"));
    }
    s.push_str(
        "\n## 还有一类：写了内心状态\n\n\
         `他很难过` `她感到兴奋` `气氛紧张`——模型拍不出情绪，只能拍行为。\n\n\
         | 写了 | 改成 |\n|---|---|\n\
         | 他很难过 | 他把下巴埋进围巾，视线避开镜头 |\n\
         | 她很紧张 | 她反复用拇指刮杯壁的水珠 |\n\
         | 气氛紧张 | 两个人的影子在墙上快要碰到 |\n\n\
         ## 一个判断标准\n\n\
         把这句话换到另一个完全不同的片子里，还成立吗？\
         成立，就说明它什么都没说。\n",
    );
    s
}

/// 逐阶段的提交前自检清单。阶段与确认门从阶段图来。
fn checklist_md() -> String {
    let mut s = String::from(
        "# 提交前自检\n\n\
         逐条过。过不了的不要提交——退回来重做比往下走便宜得多，\
         尤其是提示词那道门之后就开始花 GPU 时间了。\n\n",
    );
    for stage in StageId::all() {
        let items = checklist(stage);
        if items.is_empty() {
            continue;
        }
        s.push_str(&format!("## `{stage}`\n\n"));
        for item in items {
            s.push_str(&format!("- [ ] {item}\n"));
        }
        s.push('\n');
    }
    s.push_str(
        "## 所有创作阶段通用\n\n\
         - [ ] 没有不可拍的描述（情绪、氛围、「很美」）\n\
         - [ ] 没有 Tier 1 禁用词\n\
         - [ ] 换一个题材就不成立——如果换了还成立，说明写得太空\n",
    );
    s
}

/// 一个模型系列的能力卡。表格是基线的投影，散文是人写的语法要点。
fn model_card_md(card: &ModelCard) -> String {
    let mut s = format!(
        "# {}\n\n`{}`\n\n## 这条系列吃什么\n\n",
        card.title, card.family
    );
    s.push_str("| 模式 | 可用 | 可注入参数 |\n|---|---|---|\n");
    for (mode, params, verified) in card.modes {
        let params = if *verified {
            params
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join("、")
        } else {
            "—".to_string()
        };
        s.push_str(&format!(
            "| `{}/{}` | {} | {} |\n",
            card.family,
            mode,
            if *verified { "是" } else { "**否**" },
            params
        ));
    }

    // 没被任何已核验模式绑定的参数 = 写了会被静默丢弃的参数。
    let mut supported: Vec<&str> = Vec::new();
    for (_, params, verified) in card.modes {
        if *verified {
            for p in *params {
                if !supported.contains(p) {
                    supported.push(p);
                }
            }
        }
    }
    // 「写了会被挡下」的那一类：提交时按能力面对账，直接报 schema_violation。
    let rejected: Vec<&str> = studio_core::INJECTABLE_PARAMS
        .iter()
        .copied()
        .filter(|p| !supported.contains(p))
        .collect();
    if !rejected.is_empty() {
        s.push_str(&format!(
            "\n**这条系列不吃、写了会被挡下**：{}。\
             提交提示词包时会按这张表对账，写了它不吃的参数直接报 \
             `schema_violation`，不会等到渲染才发现。\n",
            rejected
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    // `references` 是另一类：允许提前写，但当前进不了渲染请求。
    if !supported.contains(&"references") {
        s.push_str(
            "\n`references` 可以照常写——它声明的是这一镜用到哪些资产，\
             可审计，基线补上图片输入绑定之后会自动生效。\
             但**现在它进不了渲染请求**，所以跨镜一致性目前只能靠在每一镜的\
             正向提示词里逐字复用同一段身份锁。\n",
        );
    }
    if card.modes.iter().any(|(_, _, v)| !v) {
        s.push_str(
            "\n标着「否」的模式尚未在真机上核验绑定，**不要选它们**——\
             绑错节点会静默产出错的画面，比直接报错更难查。\n",
        );
    }
    s.push('\n');
    s.push_str(fill_placeholders(card.prose).trim_end());
    s.push('\n');
    s
}

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

## 方法手册

上面三条讲的是**怎么交**。**怎么写得好**是另一回事，写在
`.agents/doctrine/` 里：镜头语法、光与色、调度、结构与钩子、声音设计、
一致性、失败模式、禁用词，还有一部完整作品的黄金样例。
每个 Skill 会指出自己该读哪几份，索引在 `.agents/doctrine/README.md`。

各个模型系列吃的参数**不一样**——写了某条系列没有绑定的参数会被静默丢弃，
不报错也不生效。写提示词之前先看 `.agents/models/` 里对应系列那一份。

这些文件用你的文件读取工具直接读，**按需读，不要一次全读**。
唯一的禁区还是 `.studio/`。

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

    let doctrine = doctrine_for(doc.name);
    if !doctrine.is_empty() {
        s.push_str("## 方法\n\n");
        s.push_str(
            "职责说的是**交什么**，下面这几份说的是**怎么想**——什么算好、\
             怎么避开已知的坑、写好的长什么样。动手之前读，别凭感觉写。\n\n",
        );
        // 目录要展开成具体文件：给一个目录名，Agent 会照自己的猜测去 cat
        // 一个不存在的路径，然后才想起来列目录。能力卡就是这么被跳过的。
        for path in doctrine {
            match path.strip_suffix('/') {
                Some(dir) => {
                    for (rel, _) in doctrine_files() {
                        let full = format!(".agents/{rel}");
                        if full.starts_with(dir) {
                            s.push_str(&format!("- `{full}`\n"));
                        }
                    }
                }
                None => s.push_str(&format!("- `{path}`\n")),
            }
        }
        s.push_str(
            "\n这些文件就在这部作品的目录里，用你的文件读取工具直接读——\
             路径照抄，不要凭印象猜。（`.studio/` 是控制面私有的，那个不要碰。）\n\n",
        );
    }

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

    if let Some(stage) = doc.stage {
        let items = checklist(stage);
        if !items.is_empty() {
            s.push_str("## 提交前自检\n\n");
            s.push_str("逐条过。过不了就别提交——退回来重做比往下走便宜得多。\n\n");
            for item in items {
                s.push_str(&format!("- [ ] {item}\n"));
            }
            s.push('\n');
        }
    }

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

/// 方法层与模型能力卡：`(相对路径, 内容)`。
///
/// 与 SKILL.md 的分工：那边是契约（交什么、被挡住怎么办），这边是方法
/// （怎么想、什么算好）。按需加载，默认一份都不进上下文。
fn doctrine_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, src) in DOCTRINE.iter() {
        out.push((format!("doctrine/{path}"), fill_placeholders(src)));
    }
    for (path, _, src) in EXEMPLARS.iter() {
        out.push((format!("doctrine/{path}"), fill_placeholders(src)));
    }
    out.push(("doctrine/quality/checklist.md".to_string(), checklist_md()));
    out.push(("doctrine/quality/banned.md".to_string(), banned_md()));
    for card in MODEL_CARDS.iter() {
        out.push((format!("models/{}.md", card.family), model_card_md(card)));
    }
    out
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
    out.extend(doctrine_files());
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
    for (path, content) in doctrine_files() {
        out.push((format!(".agents/{path}"), content));
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

    /// 方法层文档和能力卡也是 Agent 读得到的东西，同一套约束照样适用——
    /// 泄漏一处，「Agent 永远没有理由去翻源码」这条就破了。
    #[test]
    fn no_packaged_doc_points_at_source_code_or_names_the_binaries() {
        for (path, content) in all_assets() {
            if !path.ends_with(".md") {
                continue;
            }
            for leak in ["stage_graph.rs", "mcp_server.rs", "src/", "crates/", ".py"] {
                assert!(
                    !content.contains(leak),
                    "{path} 不该把 Agent 指向源码：{leak}"
                );
            }
            for leak in ["studiod", "studio-cli"] {
                assert!(!content.contains(leak), "{path} 不该提到二进制名：{leak}");
            }
        }
    }

    /// 换不掉的占位符会在文档里留下一个空洞：Agent 看到的是一句注释，
    /// 不是词表，而且没有任何东西会报错。
    #[test]
    fn every_placeholder_gets_filled() {
        for (path, content) in all_assets() {
            assert!(
                !content.contains("<!-- 词表:") && !content.contains("<!-- 样例:"),
                "{path} 里有没被替换的占位符"
            );
        }
    }

    /// 方法层必须真的带着方法：词表进得去，样例进得去，
    /// 禁用词表和自检清单是从代码里长出来的。
    #[test]
    fn doctrine_carries_the_vocabularies_and_the_exemplars() {
        let files: std::collections::HashMap<_, _> = doctrine_files().into_iter().collect();

        let grammar = &files["doctrine/camera/grammar.md"];
        for motion in lexicon::CAMERA_MOTIONS {
            assert!(grammar.contains(motion), "镜头语法里缺运镜 {motion}");
        }
        for size in lexicon::SHOT_SIZES {
            assert!(grammar.contains(size), "镜头语法里缺景别 {size}");
        }

        let lighting = &files["doctrine/camera/lighting.md"];
        for source in lexicon::LIGHTING_SOURCES {
            assert!(lighting.contains(source), "光与色里缺光源 {source}");
        }

        let banned = &files["doctrine/quality/banned.md"];
        for w in lexicon::BANNED_TIER1 {
            assert!(banned.contains(w), "禁用词表里缺 {w}");
        }

        // 样例来自契约样例本身，所以不会和 schema 漂移。
        let sb = &files["doctrine/exemplars/storyboard.md"];
        assert!(sb.contains("three_facts") && sb.contains("push_in"));
        let pack = &files["doctrine/exemplars/prompt_pack.md"];
        assert!(
            pack.contains(studio_core::fixtures::IDENTITY_LOCK),
            "提示词样例里应当看得见身份锁本身"
        );

        let checklist = &files["doctrine/quality/checklist.md"];
        for stage in StageId::all() {
            if !checklist_items_exist(stage) {
                continue;
            }
            assert!(checklist.contains(stage.as_str()), "自检清单缺阶段 {stage}");
        }
    }

    fn checklist_items_exist(stage: StageId) -> bool {
        !checklist(stage).is_empty()
    }

    /// 能力卡的「可注入参数」是基线的投影。两边对不上，Agent 就会照着
    /// 一张过期的表写提示词，而写错的参数是被**静默丢弃**的。
    #[test]
    fn model_cards_match_the_verified_baselines() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/workflows")
            .canonicalize()
            .expect("找不到基线目录");
        for card in MODEL_CARDS.iter() {
            for (mode, params, verified) in card.modes {
                let path = root.join(card.family).join(format!("{mode}.json"));
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("读不到基线 {}：{e}", path.display()));
                let json: serde_json::Value =
                    serde_json::from_str(&text).expect("基线不是合法 JSON");
                let studio = &json["_studio"];

                let disk_verified = studio["bindings_verified"].as_bool().unwrap_or(false);
                assert_eq!(
                    disk_verified, *verified,
                    "{}/{mode} 的核验状态与能力卡不一致",
                    card.family
                );

                let mut disk: Vec<&str> = studio["bindings"]
                    .as_object()
                    .expect("基线缺少 _studio.bindings")
                    .keys()
                    .map(|k| k.as_str())
                    .collect();
                disk.sort_unstable();

                // 未核验的基线不许拿来渲染，能力卡上也就不列参数。
                let mut card_params: Vec<&str> = if *verified {
                    params.to_vec()
                } else {
                    disk.clone()
                };
                card_params.sort_unstable();
                assert_eq!(
                    card_params, disk,
                    "{}/{mode} 的可注入参数与基线的 _studio.bindings 对不上",
                    card.family
                );
            }
        }
    }

    /// 能力卡必须把「写了会被挡下」的参数点出来——这是它存在的主要理由。
    /// 两类要分清：negative 是硬错误，references 是允许提前写但暂不生效。
    #[test]
    fn minimax_card_separates_rejected_params_from_inert_references() {
        let card = MODEL_CARDS
            .iter()
            .find(|c| c.family == "minimax_h3")
            .unwrap();
        let md = model_card_md(card);
        assert!(md.contains("写了会被挡下"), "没有点出会被挡下的参数");
        assert!(md.contains("`negative`"), "没有点名 negative");
        assert!(md.contains("schema_violation"), "没说清会当场报错");
        assert!(
            md.contains("`references` 可以照常写"),
            "references 是另一类，不该和 negative 混为一谈"
        );
        assert!(md.contains("进不了渲染请求"), "要说清它当前不生效");
    }

    /// LTX 用的是按秒计的时长参数，写 length_frames 会被丢弃——
    /// 这是三条系列里最容易踩的一处差异。
    #[test]
    fn ltx_card_warns_about_the_duration_parameter() {
        let card = MODEL_CARDS.iter().find(|c| c.family == "ltx2_5").unwrap();
        let md = model_card_md(card);
        assert!(md.contains("duration_seconds"));
        assert!(md.contains("`length_frames`"), "没有点名会被丢弃的参数");
    }

    /// 每个创作阶段的 Skill 都要指得出方法文档，并带上自检清单——
    /// 只有契约没有方法，产出就是填表。
    #[test]
    fn creative_skills_carry_doctrine_and_a_checklist() {
        for doc in SKILLS.iter() {
            let Some(stage) = doc.stage else { continue };
            if stage.kind() == StageKind::Deterministic {
                continue;
            }
            let md = skill_md(doc);
            assert!(
                !doctrine_for(doc.name).is_empty(),
                "{} 没有方法文档",
                doc.name
            );
            assert!(
                md.contains("## 方法"),
                "{} 的 SKILL.md 缺方法索引",
                doc.name
            );
            assert!(
                md.contains("## 提交前自检"),
                "{} 的 SKILL.md 缺自检清单",
                doc.name
            );
            for path in doctrine_for(doc.name) {
                assert!(md.contains(path), "{} 没列出 {path}", doc.name);
            }
        }
    }

    /// Skill 指向的方法文档必须真的会被物化出来，否则 Agent 会去读一个不存在的文件。
    #[test]
    fn every_referenced_doctrine_file_is_actually_shipped() {
        let shipped: Vec<String> = doctrine_files()
            .into_iter()
            .map(|(p, _)| format!(".agents/{p}"))
            .collect();
        for doc in SKILLS.iter() {
            for path in doctrine_for(doc.name) {
                let ok = if let Some(dir) = path.strip_suffix('/') {
                    shipped.iter().any(|s| s.starts_with(dir))
                } else {
                    shipped.iter().any(|s| s == path)
                };
                assert!(ok, "{} 指向的 {path} 不在随包文件里", doc.name);
            }
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
    fn assets_cover_agents_ten_skills_and_ten_schemas() {
        let a = all_assets();
        assert_eq!(
            a.iter().filter(|(p, _)| p.starts_with("skills/")).count(),
            10
        );
        assert_eq!(
            a.iter().filter(|(p, _)| p.starts_with("schema/")).count(),
            10
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
