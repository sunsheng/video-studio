//! 阶段产物契约。
//!
//! 这是「Agent 跑去读别的项目的 output.json 抄字段」的根治办法：
//! 前身项目的 `submit_stage` 参数声明是 `outputs: {[key]: unknown}`，
//! 完全不约束，Agent 唯一能确认「什么形状会被接受」的办法就是找一份被接受过的。
//!
//! 这里用一个够用的 JSON Schema 子集：既能校验，也能 `emit-assets` 成
//! `assets/schema/*.json` 随包分发，Agent 用 `studio.schema(stage)` 直接取回。

use crate::error::{Result, StudioError, Violation};
use crate::lexicon;
use crate::stage::StageId;
use crate::Outputs;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Schema {
    Object {
        desc: &'static str,
        props: Vec<(&'static str, Schema)>,
        required: Vec<&'static str>,
    },
    Array {
        desc: &'static str,
        items: Box<Schema>,
        min_items: usize,
    },
    Str {
        desc: &'static str,
        allowed: Vec<&'static str>,
    },
    Num {
        desc: &'static str,
        min: Option<f64>,
        max: Option<f64>,
    },
    Int {
        desc: &'static str,
    },
    Bool {
        desc: &'static str,
    },
    Any {
        desc: &'static str,
    },
}

pub fn obj(
    desc: &'static str,
    props: Vec<(&'static str, Schema)>,
    required: Vec<&'static str>,
) -> Schema {
    Schema::Object {
        desc,
        props,
        required,
    }
}
pub fn arr(desc: &'static str, items: Schema, min_items: usize) -> Schema {
    Schema::Array {
        desc,
        items: Box::new(items),
        min_items,
    }
}
pub fn text(desc: &'static str) -> Schema {
    Schema::Str {
        desc,
        allowed: vec![],
    }
}
pub fn one_of(desc: &'static str, allowed: Vec<&'static str>) -> Schema {
    Schema::Str { desc, allowed }
}
pub fn num(desc: &'static str) -> Schema {
    Schema::Num {
        desc,
        min: None,
        max: None,
    }
}
pub fn num_min(desc: &'static str, min: f64) -> Schema {
    Schema::Num {
        desc,
        min: Some(min),
        max: None,
    }
}
pub fn int(desc: &'static str) -> Schema {
    Schema::Int { desc }
}
pub fn any(desc: &'static str) -> Schema {
    Schema::Any { desc }
}

impl Schema {
    pub fn to_json(&self) -> Value {
        match self {
            Schema::Object {
                desc,
                props,
                required,
            } => {
                let mut p = serde_json::Map::new();
                for (k, v) in props {
                    p.insert((*k).to_string(), v.to_json());
                }
                json!({
                    "type": "object",
                    "description": desc,
                    "properties": Value::Object(p),
                    "required": required,
                    "additionalProperties": true
                })
            }
            Schema::Array {
                desc,
                items,
                min_items,
            } => json!({
                "type": "array", "description": desc,
                "items": items.to_json(), "minItems": min_items
            }),
            Schema::Str { desc, allowed } => {
                if allowed.is_empty() {
                    json!({ "type": "string", "description": desc })
                } else {
                    json!({ "type": "string", "description": desc, "enum": allowed })
                }
            }
            Schema::Num { desc, min, max } => {
                let mut m = serde_json::Map::new();
                m.insert("type".into(), json!("number"));
                m.insert("description".into(), json!(desc));
                if let Some(v) = min {
                    m.insert("minimum".into(), json!(v));
                }
                if let Some(v) = max {
                    m.insert("maximum".into(), json!(v));
                }
                Value::Object(m)
            }
            Schema::Int { desc } => json!({ "type": "integer", "description": desc }),
            Schema::Bool { desc } => json!({ "type": "boolean", "description": desc }),
            Schema::Any { desc } => json!({ "description": desc }),
        }
    }

    fn check(&self, v: &Value, path: &str, out: &mut Vec<Violation>) {
        match self {
            Schema::Object {
                props, required, ..
            } => {
                let Some(map) = v.as_object() else {
                    out.push(Violation::new(
                        path,
                        format!("应当是对象，实际是 {}", kind_of(v)),
                    ));
                    return;
                };
                for r in required {
                    if !map.contains_key(*r) {
                        out.push(Violation::new(join(path, r), "必填字段缺失"));
                    }
                }
                for (k, sub) in props {
                    if let Some(child) = map.get(*k) {
                        sub.check(child, &join(path, k), out);
                    }
                }
            }
            Schema::Array {
                items, min_items, ..
            } => {
                let Some(a) = v.as_array() else {
                    out.push(Violation::new(
                        path,
                        format!("应当是数组，实际是 {}", kind_of(v)),
                    ));
                    return;
                };
                if a.len() < *min_items {
                    out.push(Violation::new(
                        path,
                        format!("至少需要 {} 项，实际 {}", min_items, a.len()),
                    ));
                }
                for (i, child) in a.iter().enumerate() {
                    items.check(child, &format!("{path}[{i}]"), out);
                }
            }
            Schema::Str { allowed, .. } => match v.as_str() {
                None => out.push(Violation::new(
                    path,
                    format!("应当是字符串，实际是 {}", kind_of(v)),
                )),
                Some(s) => {
                    if !allowed.is_empty() && !allowed.contains(&s) {
                        out.push(Violation::new(
                            path,
                            format!("只能是 {} 之一，实际是 {s:?}", allowed.join(" / ")),
                        ));
                    }
                }
            },
            Schema::Num { min, max, .. } => match v.as_f64() {
                None => out.push(Violation::new(
                    path,
                    format!("应当是数字，实际是 {}", kind_of(v)),
                )),
                Some(n) => {
                    if let Some(m) = min {
                        if n < *m {
                            out.push(Violation::new(path, format!("不能小于 {m}，实际 {n}")));
                        }
                    }
                    if let Some(m) = max {
                        if n > *m {
                            out.push(Violation::new(path, format!("不能大于 {m}，实际 {n}")));
                        }
                    }
                }
            },
            Schema::Int { .. } => {
                if !v.is_i64() && !v.is_u64() {
                    out.push(Violation::new(
                        path,
                        format!("应当是整数，实际是 {}", kind_of(v)),
                    ));
                }
            }
            Schema::Bool { .. } => {
                if !v.is_boolean() {
                    out.push(Violation::new(
                        path,
                        format!("应当是布尔值，实际是 {}", kind_of(v)),
                    ));
                }
            }
            Schema::Any { .. } => {}
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

/// 校验一个阶段的产物。违规会带上完整字段路径，例如
/// `script.story_arc[2].duration_seconds`。
pub fn validate(stage: StageId, outputs: &Outputs) -> Result<()> {
    let key = stage.output_key();
    let mut violations = Vec::new();
    match outputs.get(key) {
        None => violations.push(Violation::new(
            key,
            format!("阶段 {stage} 的产物必须放在顶层键 {key} 下"),
        )),
        Some(v) => stage_schema(stage).check(v, key, &mut violations),
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(StudioError::SchemaViolation { stage, violations })
    }
}

/// 阶段产物在 outputs 里那一层的 schema（不含顶层键）。
pub fn stage_schema(stage: StageId) -> Schema {
    match stage {
        StageId::Idea => obj(
            "把用户创意整理成可执行 brief",
            vec![
                ("title", text("作品标题")),
                ("logline", text("一句话概括")),
                ("platform", text("发布平台，例如「抖音竖屏短视频」")),
                ("audience", text("目标受众")),
                ("theme", text("主题")),
                ("tone", text("情绪基调")),
                ("duration_seconds", num_min("总时长（秒）", 1.0)),
                ("shot_count", int("镜头数")),
                (
                    "aspect_ratio",
                    one_of("画幅", vec!["9:16", "16:9", "4:3", "1:1"]),
                ),
                (
                    "delivery_spec",
                    text("交付规格，例如 1080x1920, 30fps, H.264/AAC"),
                ),
                ("hook_0_3s", text("前三秒钩子")),
                (
                    "story_beats",
                    arr("故事节拍，逐镜头一条", text("一个节拍"), 1),
                ),
                (
                    "success_metrics",
                    arr("可验收的成功标准", text("一条标准"), 1),
                ),
                (
                    "rights_and_safety_risks",
                    arr(
                        "版权与安全风险",
                        obj(
                            "一条风险",
                            vec![
                                ("risk", text("风险")),
                                ("level", text("等级：可规避 / 需用户决定 / 不可接受")),
                                ("mitigation", text("规避方式")),
                            ],
                            vec!["risk", "level"],
                        ),
                        0,
                    ),
                ),
                (
                    "assumptions",
                    arr("对模糊输入所做的假设，必须写清", text("一条假设"), 0),
                ),
                ("explicit_exclusions", arr("明确不做的事", text("一条"), 0)),
                ("protagonist", any("主角设定，用于跨镜头一致性")),
            ],
            vec![
                "title",
                "logline",
                "platform",
                "audience",
                "duration_seconds",
                "shot_count",
                "aspect_ratio",
                "story_beats",
                "success_metrics",
            ],
        ),

        StageId::Selection => obj(
            "从可行性、受众匹配和发布风险筛选方案",
            vec![
                ("recommendation", text("推荐方案的标识")),
                (
                    "feasibility",
                    obj(
                        "可行性",
                        vec![
                            ("score", text("high / medium / low")),
                            ("rationale", text("判断依据")),
                            ("model_control", text("模型可控性说明")),
                            ("production_cost", text("制作成本")),
                        ],
                        vec!["score", "rationale"],
                    ),
                ),
                (
                    "audience_fit",
                    obj(
                        "受众匹配",
                        vec![
                            ("hook_strength", text("钩子强度")),
                            ("benefit", text("观看收益")),
                            ("retention_plan", text("留存设计")),
                        ],
                        vec!["hook_strength"],
                    ),
                ),
                (
                    "publishing_risks",
                    obj(
                        "发布风险分级",
                        vec![
                            ("avoidable", arr("可规避", text("一条"), 0)),
                            ("unacceptable", arr("不可接受", text("一条"), 0)),
                            ("user_decision", arr("需用户决定", text("一条"), 0)),
                        ],
                        vec![],
                    ),
                ),
                ("tradeoffs", text("取舍说明")),
                ("acceptance_metrics", arr("验收标准", text("一条"), 1)),
            ],
            vec![
                "recommendation",
                "feasibility",
                "audience_fit",
                "tradeoffs",
                "acceptance_metrics",
            ],
        ),

        StageId::Script => obj(
            "故事结构、节奏与声音时间线",
            vec![
                ("title", text("标题")),
                (
                    "total_duration_seconds",
                    num_min("总时长，必须与各段之和一致", 1.0),
                ),
                ("shot_count", int("镜头数")),
                (
                    "timing_rule",
                    text("时长分配规则。按内容智能分配时在这里说明依据"),
                ),
                (
                    "hook_at_seconds",
                    num_min(
                        "钩子在第几秒成立。短视频里这个数越小越好，\
                         超过 3 秒基本等于没有钩子",
                        0.0,
                    ),
                ),
                ("language", text("口播语言；无口播填 none")),
                (
                    "story_arc",
                    arr(
                        "逐拍节奏",
                        obj(
                            "一拍",
                            vec![
                                ("beat_id", text("稳定标识，例如 beat_01")),
                                (
                                    "beat_type",
                                    one_of(
                                        "这一拍在结构里的位置。短视频的骨架是\
                                         hook（勾住）→ setup（交代）→ develop（推进）→ \
                                         turn（转折）→ payoff（兑现）→ resolve（收束），\
                                         不必每种都有，但每一拍都要说清自己是哪一种",
                                        lexicon::BEAT_TYPES.to_vec(),
                                    ),
                                ),
                                ("start", num_min("起点（秒）", 0.0)),
                                ("end", num_min("终点（秒）", 0.0)),
                                ("duration_seconds", num_min("时长（秒）", 0.1)),
                                ("purpose", text("这一拍要达成什么")),
                                ("visual", text("画面")),
                                ("audio", text("声音")),
                            ],
                            vec![
                                "beat_id",
                                "beat_type",
                                "start",
                                "end",
                                "duration_seconds",
                                "purpose",
                                "visual",
                                "audio",
                            ],
                        ),
                        1,
                    ),
                ),
                (
                    "segments",
                    arr(
                        "声音/字幕时间线",
                        obj(
                            "一段",
                            vec![
                                ("segment_id", text("标识")),
                                ("start", num_min("起点（秒）", 0.0)),
                                ("end", num_min("终点（秒）", 0.0)),
                                ("speaker", text("说话人；环境声填 ambient")),
                                ("text", text("台词/旁白；无则空串")),
                                ("subtitle_text", text("字幕；无则空串")),
                                ("source", text("声音来源")),
                            ],
                            vec!["segment_id", "start", "end", "speaker"],
                        ),
                        1,
                    ),
                ),
                ("subtitle_policy", any("字幕策略")),
                (
                    "audio_policy",
                    any("音频策略：原生音频优先、外部音乐是否禁用、降级条件"),
                ),
                ("safety_notes", arr("安全注意", text("一条"), 0)),
            ],
            vec![
                "title",
                "total_duration_seconds",
                "shot_count",
                "timing_rule",
                "story_arc",
                "segments",
            ],
        ),

        StageId::Storyboard => obj(
            "逐镜头分镜：摄影机、灯光、构图与时长",
            vec![
                ("title", text("标题")),
                (
                    "aspect_ratio",
                    one_of("画幅", vec!["9:16", "16:9", "4:3", "1:1"]),
                ),
                ("total_duration_seconds", num_min("总时长", 1.0)),
                ("shot_count", int("镜头数")),
                ("timing_basis", text("时长依据。不平均切分时说明为什么")),
                (
                    "character_lock",
                    any("角色连续性锁定：外观、服装、机位签名、安全约束"),
                ),
                (
                    "shots",
                    arr(
                        "镜头表",
                        obj(
                            "一个镜头",
                            vec![
                                ("shot_id", text("稳定标识，例如 sh01")),
                                ("start", num_min("起点（秒）", 0.0)),
                                ("end", num_min("终点（秒）", 0.0)),
                                ("duration_seconds", num_min("时长（秒）", 0.1)),
                                ("purpose", text("这个镜头的作用")),
                                (
                                    "shot_function",
                                    one_of(
                                        "这一镜的职能。三者都不占的镜头不该存在，删掉它",
                                        lexicon::SHOT_FUNCTIONS.to_vec(),
                                    ),
                                ),
                                (
                                    "three_facts",
                                    arr(
                                        "三个物理事实，各一条且必须可拍：\
                                         环境压力（风、雨、人流、温度、光线变化）、\
                                         身体微动作（手指、呼吸、重心、视线）、\
                                         声音锚点（这一镜能听见的具体声源）。\
                                         写「氛围感」「很美」这类不可拍的词等于没写",
                                        text("一条可拍的物理事实"),
                                        3,
                                    ),
                                ),
                                ("shot_size", one_of("景别", lexicon::SHOT_SIZES.to_vec())),
                                ("angle", one_of("机位角度", lexicon::ANGLES.to_vec())),
                                (
                                    "camera_motion",
                                    one_of(
                                        "镜头运动。**每镜只能有一个**——两个以上的运动会让生成结果失控。\
                                         这些取值与视频模型的运镜指令一一对应，提示词阶段直接翻译，\
                                         所以写在表里的词才有效，自由发挥的描述没有效果",
                                        lexicon::CAMERA_MOTIONS.to_vec(),
                                    ),
                                ),
                                (
                                    "lighting_source",
                                    one_of("光源：光从哪来", lexicon::LIGHTING_SOURCES.to_vec()),
                                ),
                                (
                                    "lighting_key",
                                    one_of(
                                        "光型：光怎么打在主体上",
                                        lexicon::LIGHTING_KEYS.to_vec(),
                                    ),
                                ),
                                (
                                    "color_tone",
                                    text(
                                        "色调。用可复现的说法：冷白、暖金、青橙、低饱和、\
                                         漂白旁路。「高级感」这类词对模型无效",
                                    ),
                                ),
                                ("subject", text("主体")),
                                ("foreground", text("前景")),
                                ("midground", text("中景")),
                                ("background", text("背景")),
                                (
                                    "action_chain",
                                    text("动作链：起 -> 承 -> 收。写可见的身体动作，不写内心状态"),
                                ),
                                ("first_frame", text("首帧")),
                                ("last_frame", text("尾帧")),
                                (
                                    "audio",
                                    obj(
                                        "这一镜的声音。核心系列多为音视频联合生成，\
                                         这里写的内容会进提示词，留空等于放弃原生音频",
                                        vec![
                                            ("ambient", text("环境声：这个空间本来就有的声音")),
                                            ("foley", text("拟音：这一镜的动作发出的声音")),
                                            (
                                                "dialogue",
                                                obj(
                                                    "对白。没有就整个省略，不要填空串",
                                                    vec![
                                                        ("speaker", text("说话人")),
                                                        ("text", text("台词原文")),
                                                        (
                                                            "language",
                                                            text("语言与口音，例如「普通话」"),
                                                        ),
                                                    ],
                                                    vec!["speaker", "text"],
                                                ),
                                            ),
                                            ("music", text("音乐；本镜无音乐填 none")),
                                        ],
                                        vec![],
                                    ),
                                ),
                                ("sound", text("声音的一句话概述（保留字段，细节写在 audio 里）")),
                                ("transition_to_next", text("转场方式")),
                            ],
                            vec![
                                "shot_id",
                                "start",
                                "end",
                                "duration_seconds",
                                "purpose",
                                "shot_function",
                                "three_facts",
                                "shot_size",
                                "camera_motion",
                                "subject",
                                "action_chain",
                            ],
                        ),
                        1,
                    ),
                ),
            ],
            vec![
                "title",
                "aspect_ratio",
                "total_duration_seconds",
                "shot_count",
                "shots",
            ],
        ),

        StageId::VisualAssets => obj(
            "角色卡、场景卡与参考资产计划",
            vec![
                ("backend", text("生成后端，通常是 comfyui")),
                ("core_model_family", text("核心模型系列，例如 minimax_h3")),
                ("strategy", text("生成策略，例如先出开发片段再抽帧")),
                (
                    "fallback_policy",
                    text("降级策略。默认结构化阻塞，不自动换系列"),
                ),
                (
                    "consistency_lock",
                    any("一致性锁定：角色、机位、环境、安全、排版"),
                ),
                (
                    "requests",
                    arr(
                        "资产请求",
                        obj(
                            "一项资产",
                            vec![
                                ("asset_id", text("稳定标识，例如 C01 / SC01")),
                                (
                                    "asset_kind",
                                    one_of(
                                        "资产类型",
                                        vec![
                                            "character_card",
                                            "scene_card",
                                            "prop_card",
                                            "safety_reference",
                                            "style_reference",
                                        ],
                                    ),
                                ),
                                ("prompt", text("生成提示词")),
                                ("width", int("宽")),
                                ("height", int("高")),
                                ("applies_to", arr("作用于哪些镜头", text("shot_id"), 0)),
                                ("references", arr("依赖的其它资产", text("asset_id"), 0)),
                                (
                                    "status",
                                    one_of(
                                        "状态",
                                        vec!["planned", "generating", "ready", "failed"],
                                    ),
                                ),
                            ],
                            vec!["asset_id", "asset_kind", "prompt", "status"],
                        ),
                        1,
                    ),
                ),
            ],
            vec![
                "backend",
                "core_model_family",
                "consistency_lock",
                "requests",
            ],
        ),

        StageId::PromptPack => obj(
            "逐镜头 prompt 与 ComfyUI workflow 参数",
            vec![
                ("core_model_family", text("核心模型系列")),
                (
                    "shots",
                    arr(
                        "逐镜头参数",
                        obj(
                            "一个镜头的 prompt",
                            vec![
                                ("shot_id", text("对应分镜的 shot_id")),
                                (
                                    "workflow",
                                    text(
                                        "使用的已验证 workflow 名，例如 minimax_h3/t2v。\
                                         每条基线吃的参数不同，写之前先看这个系列的能力卡——\
                                         提交时会按这台机器上基线的能力面逐项对账，\
                                         多写、少写、用未核验的基线都会被挡下",
                                    ),
                                ),
                                (
                                    "positive",
                                    text(
                                        "正向提示词。一条提示词只描述**一个可读镜头**：\
                                         一个主体、一个可见动作、一个受控环境、一个运镜。\
                                         句首放最重要的视觉元素。\
                                         禁止 cinematic / 电影感 / 史诗般 / 大片质感 / 4K / \
                                         高质量这类对模型无效的词",
                                    ),
                                ),
                                (
                                    "negative",
                                    text(
                                        "负向提示词。**不是每条基线都支持**——不支持的基线上\
                                         写了会被**挡下**（提交时按能力面对账，报 schema_violation），\
                                         该把约束改写成正向的连续性约束\
                                         （「一镜到底」「主体全程居中」「不切场景」）。\
                                         先看能力卡",
                                    ),
                                ),
                                (
                                    "audio",
                                    text(
                                        "这一镜要出的声音：环境声、拟音、对白（放引号并注明语言）、\
                                         音乐。核心系列多为音视频联合生成，留空等于放弃原生音频。\
                                         内容照抄分镜的 audio，不要在这里另编",
                                    ),
                                ),
                                ("width", int("宽")),
                                ("height", int("高")),
                                (
                                    "length_frames",
                                    int("帧数。有的基线按秒收时长，用 duration_seconds"),
                                ),
                                (
                                    "duration_seconds",
                                    num_min("时长（秒）。按帧数收时长的基线用 length_frames", 0.1),
                                ),
                                ("fps", int("帧率")),
                                ("seed", int("随机种子。固定以便复现")),
                                (
                                    "references",
                                    arr(
                                        "引用的视觉资产。可以照常写——它声明这一镜用到哪些资产，\
                                         基线补上图片输入绑定后会自动生效。但**当前所有基线都还没绑\
                                         图片输入**，写在这里的 asset_id 暂时进不了渲染请求，\
                                         一致性目前只能靠 positive 里逐字复用同一段外观描述",
                                        text("asset_id"),
                                        0,
                                    ),
                                ),
                            ],
                            vec![
                                "shot_id",
                                "workflow",
                                "positive",
                                "width",
                                "height",
                                "length_frames",
                                "fps",
                            ],
                        ),
                        1,
                    ),
                ),
            ],
            vec!["core_model_family", "shots"],
        ),

        StageId::Preview => obj(
            "480p 预览结果登记（由控制面产出）。花贵的正式渲染之前，先出便宜的\
             低分辨率预览让人工确认构图与内容——帧数/时长不变，只降分辨率。",
            vec![(
                "shots",
                arr(
                    "每镜头一条",
                    obj(
                        "一镜预览结果",
                        vec![
                            ("shot_id", text("镜头标识")),
                            ("node", text("承载的 ComfyUI 节点")),
                            ("prompt_id", text("ComfyUI 的 prompt_id，用于追溯")),
                            (
                                "path",
                                text("预览文件的 bundle 内相对路径，media/preview/ 下"),
                            ),
                            ("width", int("预览宽度（480 短边缩放后）")),
                            ("height", int("预览高度（480 短边缩放后）")),
                            ("duration_seconds", num("实际时长")),
                        ],
                        vec!["shot_id", "node", "prompt_id", "path"],
                    ),
                    1,
                ),
            )],
            vec!["shots"],
        ),

        StageId::Render => obj(
            "渲染结果登记（由控制面产出）",
            vec![(
                "shots",
                arr(
                    "每镜头一条",
                    obj(
                        "一镜结果",
                        vec![
                            ("shot_id", text("镜头标识")),
                            ("node", text("承载的 ComfyUI 节点")),
                            ("prompt_id", text("ComfyUI 的 prompt_id，用于追溯")),
                            ("path", text("产出文件的 bundle 内相对路径")),
                            ("duration_seconds", num("实际时长")),
                        ],
                        vec!["shot_id", "node", "prompt_id", "path"],
                    ),
                    1,
                ),
            )],
            vec!["shots"],
        ),

        StageId::Post => obj(
            "后期结果（由控制面产出）",
            vec![
                ("video", text("成片的 bundle 内相对路径")),
                ("cover", text("封面相对路径")),
                ("subtitles", text("字幕相对路径")),
                ("duration_seconds", num("成片实际时长")),
                ("aspect_ratio", text("成片实际画幅")),
            ],
            vec!["video", "duration_seconds", "aspect_ratio"],
        ),

        StageId::Review => obj(
            "验收报告（由控制面产出）",
            vec![
                (
                    "passed",
                    Schema::Bool {
                        desc: "是否通过"
                    },
                ),
                (
                    "checks",
                    arr(
                        "逐项检查",
                        obj(
                            "一项",
                            vec![
                                ("name", text("检查项")),
                                ("passed", Schema::Bool { desc: "结果" }),
                                ("detail", text("依据。必须来自实际媒体元数据，不能是推断")),
                            ],
                            vec!["name", "passed", "detail"],
                        ),
                        1,
                    ),
                ),
            ],
            vec!["passed", "checks"],
        ),
    }
}

/// 完整的 JSON Schema 文档，`emit-assets` 写进 `assets/schema/<stage>.json`。
pub fn stage_schema_document(stage: StageId) -> Value {
    let key = stage.output_key();
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": format!("https://github.com/sunsheng/video-studio/schema/{}.json", stage),
        "title": format!("{stage} 阶段产物"),
        "description": format!("studio.submit_stage 在阶段 {stage} 接受的 outputs 形状。顶层键固定为 {key}。"),
        "type": "object",
        "properties": { key: stage_schema(stage).to_json() },
        "required": [key],
        "additionalProperties": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wrap(stage: StageId, v: Value) -> Outputs {
        let mut m = Outputs::new();
        m.insert(stage.output_key().to_string(), v);
        m
    }

    #[test]
    fn every_stage_has_a_schema_document() {
        for s in StageId::all() {
            let doc = stage_schema_document(s);
            assert_eq!(doc["required"][0], json!(s.output_key()));
            assert!(doc["properties"][s.output_key()].is_object());
        }
    }

    #[test]
    fn missing_top_level_key_is_reported() {
        let e = validate(StageId::Script, &Outputs::new()).unwrap_err();
        match e {
            StudioError::SchemaViolation { violations, .. } => {
                assert_eq!(violations[0].path, "script");
            }
            other => panic!("应当是 schema_violation，实际 {other}"),
        }
    }

    #[test]
    fn violation_paths_point_at_the_exact_field() {
        let bad = wrap(
            StageId::Script,
            json!({
                "title": "千岛湖，把快乐装进十秒",
                "total_duration_seconds": 10,
                "shot_count": 5,
                "timing_rule": "按动作复杂度分配",
                "story_arc": [
                    { "beat_id": "beat_01", "start": 0, "end": 1.4, "duration_seconds": 1.4,
                      "purpose": "地点钩子", "visual": "船头掠过湖面", "audio": "湖水轻拍" },
                    { "beat_id": "beat_02", "start": 1.4, "end": 3.4, "duration_seconds": "两秒",
                      "purpose": "人物动作", "visual": "小跑", "audio": "脚步" }
                ],
                "segments": [
                    { "segment_id": "s01", "start": 0, "end": 1.4, "speaker": "ambient" }
                ]
            }),
        );
        let e = validate(StageId::Script, &bad).unwrap_err();
        match e {
            StudioError::SchemaViolation { violations, .. } => {
                assert!(
                    violations
                        .iter()
                        .any(|v| v.path == "script.story_arc[1].duration_seconds"),
                    "没有精确定位到出错字段：{violations:?}"
                );
            }
            other => panic!("应当是 schema_violation，实际 {other}"),
        }
    }

    #[test]
    fn a_well_formed_script_passes() {
        let good = wrap(
            StageId::Script,
            json!({
                "title": "千岛湖，把快乐装进十秒",
                "total_duration_seconds": 10,
                "shot_count": 5,
                "timing_rule": "按动作复杂度和信息量分配，合计 10 秒",
                "language": "none",
                "story_arc": [
                    { "beat_id": "beat_01", "beat_type": "hook", "start": 0,   "end": 1.4,  "duration_seconds": 1.4,
                      "purpose": "地点钩子", "visual": "船头掠过清透湖面", "audio": "湖水轻拍船身" },
                    { "beat_id": "beat_02", "beat_type": "setup", "start": 1.4, "end": 3.4,  "duration_seconds": 2.0,
                      "purpose": "人物动作", "visual": "沿步道轻快小跑", "audio": "轻快脚步与风声" },
                    { "beat_id": "beat_03", "beat_type": "develop", "start": 3.4, "end": 5.8,  "duration_seconds": 2.4,
                      "purpose": "景色展开", "visual": "观景台举起手机取景", "audio": "快门声" },
                    { "beat_id": "beat_04", "beat_type": "payoff", "start": 5.8, "end": 7.8,  "duration_seconds": 2.0,
                      "purpose": "互动快乐", "visual": "举起冷饮轻碰杯", "audio": "碰杯声" },
                    { "beat_id": "beat_05", "beat_type": "resolve", "start": 7.8, "end": 10.0, "duration_seconds": 2.2,
                      "purpose": "情绪收束", "visual": "回头挥手", "audio": "环境声收尾" }
                ],
                "segments": [
                    { "segment_id": "s01", "start": 0, "end": 10, "speaker": "ambient",
                      "text": "", "subtitle_text": "", "source": "核心模型原生环境声" }
                ]
            }),
        );
        validate(StageId::Script, &good).expect("这份剧本应当通过校验");
    }

    #[test]
    fn enum_values_are_enforced() {
        let bad = wrap(
            StageId::Storyboard,
            json!({
                "title": "t", "aspect_ratio": "竖屏", "total_duration_seconds": 10, "shot_count": 1,
                "shots": [{ "shot_id": "sh01", "start": 0, "end": 10, "duration_seconds": 10,
                            "purpose": "p", "shot_size": "近景", "camera_motion": "推",
                            "subject": "人", "action_chain": "起 -> 收" }]
            }),
        );
        let e = validate(StageId::Storyboard, &bad).unwrap_err();
        match e {
            StudioError::SchemaViolation { violations, .. } => assert!(violations
                .iter()
                .any(|v| v.path == "storyboard.aspect_ratio")),
            other => panic!("实际 {other}"),
        }
    }

    #[test]
    fn empty_required_array_is_reported() {
        let bad = wrap(
            StageId::PromptPack,
            json!({ "core_model_family": "minimax_h3", "shots": [] }),
        );
        let e = validate(StageId::PromptPack, &bad).unwrap_err();
        match e {
            StudioError::SchemaViolation { violations, .. } => {
                assert!(violations.iter().any(|v| v.path == "prompt_pack.shots"))
            }
            other => panic!("实际 {other}"),
        }
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::fixtures;

    /// 样例产物必须真的能通过校验，否则测试与端到端用例都是自欺欺人。
    #[test]
    fn every_fixture_passes_its_own_schema() {
        for stage in StageId::all() {
            let outputs = fixtures::outputs(stage);
            validate(stage, &outputs).unwrap_or_else(|e| panic!("阶段 {stage} 的样例不合规：{e}"));
        }
    }

    /// 剧本各拍时长必须真的合计 10 秒——样例本身就该是一份说得通的作品。
    #[test]
    fn script_fixture_durations_add_up() {
        let o = fixtures::outputs(StageId::Script);
        let arc = o["script"]["story_arc"].as_array().unwrap();
        let sum: f64 = arc
            .iter()
            .map(|b| b["duration_seconds"].as_f64().unwrap())
            .sum();
        assert!((sum - 10.0).abs() < 1e-9, "五拍合计应为 10 秒，实际 {sum}");
        assert_eq!(arc.len(), 5);
    }

    #[test]
    fn gated_stages_have_fixture_confirmations() {
        for stage in StageId::all() {
            assert_eq!(
                fixtures::confirmation(stage).is_some(),
                stage.gate().is_some(),
                "阶段 {stage} 的样例确认门与阶段图不一致"
            );
        }
    }
}
