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

/// 一个视图最多挂几张参考图——FLUX.2 dev 的上限，见
/// `assets/workflows/flux2_dev/README.md`。
pub const MAX_CARD_REFERENCES: usize = 10;

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
        Some(v) => {
            stage_schema(stage).check(v, key, &mut violations);
            // 形状之外的结构约束：schema 的 enum 表达不了「角色卡必须有
            // 这五个视图」「非主视图必须指向锚点」这类条件依赖。
            structural(stage, v, key, &mut violations);
        }
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
                (
                    "concepts",
                    arr(
                        "**互斥**的创意方案，2–3 个。同一个需求的几种不同拍法：\
                         平台、受众、时长这些由需求定死的东西共用，\
                         各方案不同的是切入角度、钩子和节拍。\
                         只给一个方案，下一阶段的「筛选」就成了自问自答",
                        obj(
                            "一个方案",
                            vec![
                                ("concept_id", text("稳定标识，例如 c1")),
                                ("logline", text("这个方案的一句话概括")),
                                (
                                    "angle",
                                    text("切入角度：同一件事，这个方案从哪儿讲起"),
                                ),
                                (
                                    "hook_0_3s",
                                    text("前三秒钩子。要具体到画面，不能写「用悬念抓住观众」"),
                                ),
                                (
                                    "story_beats",
                                    arr("故事节拍，逐镜头一条", text("一个节拍"), 1),
                                ),
                                (
                                    "tradeoff",
                                    text("选它要牺牲什么。三个方案的牺牲不该是同一件事"),
                                ),
                            ],
                            vec!["concept_id", "logline", "angle", "hook_0_3s", "story_beats"],
                        ),
                        2,
                    ),
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
                "concepts",
                "success_metrics",
            ],
        ),

        StageId::Selection => obj(
            "从可行性、受众匹配和发布风险里挑一个方案，并说清代价",
            vec![
                (
                    "candidates",
                    arr(
                        "逐个评估 idea 阶段给出的方案。**每个都要评**——\
                         只评推荐的那个，等于没有比较",
                        obj(
                            "一个方案的评估",
                            vec![
                                ("concept_id", text("对应 idea 的 concept_id")),
                                (
                                    "feasibility",
                                    obj(
                                        "可行性",
                                        vec![
                                            ("score", one_of("高 / 中 / 低", vec!["high", "medium", "low"])),
                                            ("rationale", text("判断依据：模型可控性、制作成本")),
                                        ],
                                        vec!["score", "rationale"],
                                    ),
                                ),
                                (
                                    "audience_fit",
                                    obj(
                                        "受众匹配",
                                        vec![
                                            ("hook_strength", one_of("钩子强度", vec!["strong", "medium", "weak"])),
                                            ("rationale", text("为什么这个钩子对这批受众有效")),
                                        ],
                                        vec!["hook_strength"],
                                    ),
                                ),
                                ("risks", arr("这个方案特有的风险", text("一条"), 0)),
                                (
                                    "verdict",
                                    one_of(
                                        "结论：推荐 / 可选 / 不建议",
                                        vec!["recommended", "viable", "not_advised"],
                                    ),
                                ),
                            ],
                            vec!["concept_id", "feasibility", "audience_fit", "verdict"],
                        ),
                        2,
                    ),
                ),
                (
                    "recommendation",
                    text("推荐哪个方案，写 concept_id。它必须是 candidates 里 verdict 为 recommended 的那个"),
                ),
                (
                    "tradeoffs",
                    text("推荐它**牺牲了什么**。只讲优点的推荐等于没推荐"),
                ),
                (
                    "publishing_risks",
                    obj(
                        "发布风险分级（针对推荐方案）",
                        vec![
                            ("avoidable", arr("可规避", text("一条"), 0)),
                            ("unacceptable", arr("不可接受", text("一条"), 0)),
                            ("user_decision", arr("需用户决定", text("一条"), 0)),
                        ],
                        vec![],
                    ),
                ),
                ("acceptance_metrics", arr("验收标准", text("一条"), 1)),
            ],
            vec![
                "candidates",
                "recommendation",
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
                    obj(
                        "角色连续性锁定。**这里定的字符串是后面两个阶段的唯一来源**：\
                         视觉资产的 consistency_lock.character 和提示词包的 \
                         identity_lock.character 必须与它逐字相同",
                        vec![
                            (
                                "identity_lock",
                                text(
                                    "身份锁：一次写定、后续逐字复制的那段外观描述。\
                                     年龄、性别、发型发长、上衣、下装、鞋、随身物，\
                                     一句话写完。写「同一位女孩」这类指代等于没锁——\
                                     模型看不到上一镜",
                                ),
                            ),
                            (
                                "camera_signature",
                                text("机位签名：这个角色主要以什么角度出现，全片保持一致"),
                            ),
                            ("safety", text("安全约束：不做什么动作、不靠近什么位置")),
                        ],
                        vec!["identity_lock"],
                    ),
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
                (
                    "strategy",
                    text(
                        "这批卡怎么排：哪些角色/场景/道具要出卡、各要哪些视图、为什么。\
                         用什么模型出图由控制面决定，不用写",
                    ),
                ),
                (
                    "fallback_policy",
                    text("降级策略。默认结构化阻塞，不自动换系列"),
                ),
                (
                    "consistency_lock",
                    obj(
                        "一致性锁定。`character` 从分镜的 character_lock.identity_lock \
                         **原样复制**，不要在这里重写一遍",
                        vec![
                            (
                                "character",
                                text("身份锁，与分镜的 character_lock.identity_lock 逐字相同"),
                            ),
                            ("camera", text("机位签名")),
                            ("environment", text("环境锁定：地点、天气、时段")),
                            (
                                "typography",
                                text(
                                    "排版禁止项：乱码文字、假字幕、水印。\
                                     挡的是模型幻觉出来的假字，不是标识本身——\
                                     要出现的 logo、图标、品牌正常写进提示词即可",
                                ),
                            ),
                        ],
                        vec!["character"],
                    ),
                ),
                (
                    "assets",
                    arr(
                        "资产清单。**每个跨镜头复用的角色、场景、道具都要有一项**，\
                         每一项下面挂多个视图——一张大头照锁不住服装，\
                         一张全身照锁不住脸",
                        obj(
                            "一项资产",
                            vec![
                                ("asset_id", text("稳定标识，例如 C01 / SC01 / P01")),
                                (
                                    "asset_kind",
                                    one_of("资产类型", lexicon::ASSET_KINDS.to_vec()),
                                ),
                                (
                                    "identity_prompt",
                                    text(
                                        "这张卡的外观锁，**一次写定**：\
                                         发型、脸型、肤色、瞳色、服装、体型、年龄段、\
                                         标志性特征。下面每个视图的 prompt 都要逐字带上它，\
                                         不复述、不改写。角色卡的这一段要与 \
                                         consistency_lock.character 一致",
                                    ),
                                ),
                                ("applies_to", arr("作用于哪些镜头", text("shot_id"), 0)),
                                (
                                    "views",
                                    arr(
                                        "视图清单。必需视图缺一个就不放行，\
                                         各类必需哪些见方法文档 consistency/character-sheet.md",
                                        obj(
                                            "一个视图",
                                            vec![
                                                (
                                                    "view",
                                                    one_of(
                                                        "视图 id。取值随 asset_kind 而定：\
                                                         角色卡用 front_full 那一组，\
                                                         场景卡用 establishing 那一组，\
                                                         道具卡用 front 那一组",
                                                        lexicon::ALL_VIEWS.to_vec(),
                                                    ),
                                                ),
                                                (
                                                    "is_anchor",
                                                    Schema::Bool {
                                                        desc: "是不是主视图。每张卡**有且仅有一个**，\
                                                               且必须是该类型的固定主视图。\
                                                               主视图先出、单独出，其余视图都以它为参考图",
                                                    },
                                                ),
                                                (
                                                    "aspect",
                                                    text(
                                                        "目标比例，例如 9:16 / 1:1 / 16:9。\
                                                         同一张卡的所有视图用同一套规格——\
                                                         一张竖一张方会让人误以为是不同批次生成的",
                                                    ),
                                                ),
                                                (
                                                    "prompt",
                                                    text(
                                                        "本视图的提示词：identity_prompt 逐字 + \
                                                         本视图特有的机位/表情描述 + 画幅比例。\
                                                         统一约束：中性灰底、均匀柔光、无阴影投射、\
                                                         不裁切。卡片是**测量用的参考素材**，\
                                                         不是好看的剧照",
                                                    ),
                                                ),
                                                (
                                                    "derived_from",
                                                    arr(
                                                        "本视图挂哪几张已定稿的视图当参考图。\
                                                         **非主视图必填**，第一项是本卡的主视图，\
                                                         后面按顺序补上**这一张之前已经定稿的其余视图**——\
                                                         这叫累积锁定：出第 5 个视图时前 4 个都在场，\
                                                         新视图必须同时与它们自洽，漂移无处可去。\
                                                         主视图自己不填。只能指向同一张卡里排在自己\
                                                         前面的视图，最多 10 张",
                                                        text("同一张卡里的视图 id，例如 front_full"),
                                                        1,
                                                    ),
                                                ),
                                                (
                                                    "status",
                                                    one_of(
                                                        "状态。**提交时一律 planned**，\
                                                         其余取值由控制面回填",
                                                        vec![
                                                            "planned",
                                                            "generating",
                                                            "ready",
                                                            "failed",
                                                        ],
                                                    ),
                                                ),
                                                (
                                                    "path",
                                                    text(
                                                        "落盘位置，控制面回填。\
                                                         bundle 内相对路径，\
                                                         形如 media/assets/<asset_id>/<view>.png",
                                                    ),
                                                ),
                                                (
                                                    "provenance",
                                                    any(
                                                        "哪个后端、哪条基线、什么尺寸、什么种子出的。\
                                                         控制面回填，可审计",
                                                    ),
                                                ),
                                            ],
                                            vec!["view", "is_anchor", "aspect", "prompt", "status"],
                                        ),
                                        1,
                                    ),
                                ),
                            ],
                            vec!["asset_id", "asset_kind", "identity_prompt", "views"],
                        ),
                        1,
                    ),
                ),
            ],
            vec![
                "backend",
                "core_model_family",
                "consistency_lock",
                "assets",
            ],
        ),

        StageId::PromptPack => obj(
            "逐镜头 prompt 与 ComfyUI workflow 参数",
            vec![
                ("core_model_family", text("核心模型系列")),
                (
                    "identity_lock",
                    obj(
                        "身份锁。从视觉资产的 consistency_lock **原样复制**——\
                         提交时会逐字比对，也会逐镜检查每条 positive 是不是真的带上了它",
                        vec![
                            (
                                "character",
                                text(
                                    "与分镜 character_lock.identity_lock、\
                                     视觉资产 consistency_lock.character 逐字相同的那段外观描述。\
                                     每一镜的 positive 里必须原样出现",
                                ),
                            ),
                            ("environment", text("环境锁定，同样逐字复用")),
                            ("typography", text("排版禁止项")),
                        ],
                        vec!["character"],
                    ),
                ),
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
                                        "整图基线名，例如 ltx2_5/t2v。**只有走整图基线的系列用它**\
                                         （ltx2_5 / wan2_2 等）。片段化的系列改用 head + \
                                         references + guides，写 workflow 会被挡下。\
                                         调 studio.schema 看这台机器给的是哪一种",
                                    ),
                                ),
                                (
                                    "head",
                                    text(
                                        "这一镜用哪种生成方式。**片段化的系列用它代替 workflow**：\
                                         reference（挂参考，锁身份/风格，起幅由模型定）、\
                                         image（给首尾帧，锁构图与运动轨迹）。\
                                         取值由 studio.schema 按这台机器的片段库给出",
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
                                        "挂给这一镜的视觉资产。**只有 head: reference 吃它**——\
                                         身份、空间、运动各挂一路，比在 positive 里反复描述外观\
                                         有效得多。上限按介质分：图 9、视频 3、音频 3",
                                        obj(
                                            "一条参考",
                                            vec![
                                                (
                                                    "kind",
                                                    one_of(
                                                        "介质",
                                                        vec!["image", "video", "audio"],
                                                    ),
                                                ),
                                                (
                                                    "asset_id",
                                                    text(
                                                        "visual_assets 登记过的产物 id。\
                                                         写不存在的会被挡下并列出可用的",
                                                    ),
                                                ),
                                                (
                                                    "with_audio",
                                                    Schema::Bool {
                                                        desc: "视频参考是否连音轨一起挂。\
                                                               只有 kind: video 能用",
                                                    },
                                                ),
                                            ],
                                            vec!["kind", "asset_id"],
                                        ),
                                        0,
                                    ),
                                ),
                                (
                                    "guides",
                                    arr(
                                        "把某个素材锚在某一帧上。**镜头之间接不接得住靠它**——\
                                         把上一镜的尾段锚在本镜第 0 帧，模型生成的是两条流的续接，\
                                         比只在提示词里说「接上一镜」有效。\
                                         head: image 只能锚首帧（0）或尾帧（-1）",
                                        obj(
                                            "一个锚点",
                                            vec![
                                                (
                                                    "kind",
                                                    one_of(
                                                        "锚什么。clip 是视频片段，长度吃 17k+5 网格",
                                                        vec!["image", "clip", "audio"],
                                                    ),
                                                ),
                                                (
                                                    "at_frame",
                                                    int(
                                                        "锚在第几帧。负数从末尾倒数，-1 是最后一帧",
                                                    ),
                                                ),
                                                ("asset_id", text("visual_assets 登记过的产物 id")),
                                            ],
                                            vec!["kind", "at_frame", "asset_id"],
                                        ),
                                        0,
                                    ),
                                ),
                                (
                                    "first_frame",
                                    text("首帧的 asset_id。**只有 head: image 用**"),
                                ),
                                (
                                    "last_frame",
                                    text("尾帧的 asset_id。**只有 head: image 用**，可不给"),
                                ),
                            ],
                            vec![
                                "shot_id",
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
            vec!["core_model_family", "identity_lock", "shots"],
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
                (
                    "upscaled",
                    Schema::Bool {
                        desc: "有没有做成片超分。false 表示成片停在模型的原生画布上，\
                               交付分辨率达不到 1080——那是配置里明确关掉的结果，不是失败",
                    },
                ),
                ("delivery", text("成片实际尺寸，例如 1080x1920")),
            ],
            vec!["video", "duration_seconds", "aspect_ratio"],
        ),

        StageId::Review => obj(
            "验收报告。**技术验收**由控制面产出（下面的 checks 与 passed）；\
             **内容验收**由 Agent 事后用 studio.self_review 补上（content_review），\
             它不改变 passed——片子已经出来了，内容评价改变不了它是否完整",
            vec![
                (
                    "passed",
                    Schema::Bool {
                        desc: "技术验收是否通过。只看 kind 为 technical 的检查项"
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
                                (
                                    "kind",
                                    one_of(
                                        "这一项是谁判的：technical 来自 ffprobe 实测，\
                                         content 来自内容自评",
                                        vec!["technical", "content"],
                                    ),
                                ),
                                ("passed", Schema::Bool { desc: "结果" }),
                                ("detail", text("依据。必须来自实际媒体元数据，不能是推断")),
                            ],
                            vec!["name", "kind", "passed", "detail"],
                        ),
                        1,
                    ),
                ),
                (
                    "content_review",
                    obj(
                        "内容自评。由 studio.self_review 写入，Agent 不在这里提交",
                        vec![
                            (
                                "items",
                                arr(
                                    "逐维度一条",
                                    obj(
                                        "一条自评",
                                        vec![
                                            (
                                                "criterion",
                                                one_of(
                                                    "评价维度。固定五条，不可增删",
                                                    crate::rubric::CRITERIA
                                                        .iter()
                                                        .map(|(c, _)| *c)
                                                        .collect(),
                                                ),
                                            ),
                                            (
                                                "verdict",
                                                one_of(
                                                    "结论",
                                                    crate::rubric::VERDICTS.to_vec(),
                                                ),
                                            ),
                                            (
                                                "at_seconds",
                                                num_min("可指认的时间点（秒）", 0.0),
                                            ),
                                            (
                                                "evidence",
                                                text(
                                                    "在那个时间点上看见/听见了什么，\
                                                     以及它为什么支持这个结论",
                                                ),
                                            ),
                                        ],
                                        vec!["criterion", "verdict", "at_seconds", "evidence"],
                                    ),
                                    5,
                                ),
                            ),
                            ("summary", text("一句话：最强的一点和最弱的一点")),
                        ],
                        vec!["items", "summary"],
                    ),
                ),
            ],
            vec!["passed", "checks"],
        ),
    }
}

/// 条件依赖类的结构约束。JSON Schema 的子集表达不了它们，但它们和字段
/// 缺失一样是硬错误——放过去，下游拿到的就是一份看起来完整、实际锁不住
/// 任何东西的资产计划。
fn structural(stage: StageId, v: &Value, key: &str, out: &mut Vec<Violation>) {
    if stage != StageId::VisualAssets {
        return;
    }
    let Some(assets) = v.get("assets").and_then(|a| a.as_array()) else {
        return;
    };
    let lock = v
        .get("consistency_lock")
        .and_then(|c| c.get("character"))
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    for (i, asset) in assets.iter().enumerate() {
        let at = |suffix: &str| format!("{key}.assets[{i}]{suffix}");
        let kind = asset
            .get("asset_kind")
            .and_then(|k| k.as_str())
            .unwrap_or_default();
        let identity = asset
            .get("identity_prompt")
            .and_then(|p| p.as_str())
            .unwrap_or_default();
        // 角色卡的身份锁必须**逐字包含**视频那段身份锁。卡片可以更细
        // （脸型、肤色、瞳色是出图才需要的），但不能是另一段近义改写——
        // 那样卡片上的人和成片里的人就不是同一个人了。
        if kind == "character_card" && !lock.is_empty() && !identity.contains(lock) {
            out.push(Violation::new(
                at(".identity_prompt"),
                format!(
                    "没有逐字包含 consistency_lock.character。\
                     卡片可以写得更细，但那段身份锁要原样在里面：「{lock}」"
                ),
            ));
        }

        let allowed = lexicon::views_for(kind);
        if allowed.is_empty() {
            // 参照类资产（safety_reference / style_reference）不强制多视图。
            continue;
        }
        let views = asset
            .get("views")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let names: Vec<&str> = views
            .iter()
            .filter_map(|x| x.get("view").and_then(|n| n.as_str()))
            .collect();

        for want in lexicon::required_views(kind) {
            if !names.contains(want) {
                out.push(Violation::new(
                    at(".views"),
                    format!(
                        "{kind} 缺必需视图 {want}。多角度不是形容词：\
                         少一个角度就少一处可比对的参照，下游只能靠猜"
                    ),
                ));
            }
        }
        for (j, name) in names.iter().enumerate() {
            if !allowed.contains(name) {
                out.push(Violation::new(
                    at(&format!(".views[{j}].view")),
                    format!(
                        "{name} 不是 {kind} 的视图。合法取值：{}",
                        allowed.join("、")
                    ),
                ));
            }
        }

        let anchor_name = lexicon::anchor_view(kind).unwrap_or_default();
        let anchors: Vec<usize> = views
            .iter()
            .enumerate()
            .filter(|(_, x)| x.get("is_anchor").and_then(|b| b.as_bool()) == Some(true))
            .map(|(j, _)| j)
            .collect();
        match anchors.len() {
            1 => {
                let j = anchors[0];
                let got = views[j].get("view").and_then(|n| n.as_str()).unwrap_or("");
                if got != anchor_name {
                    out.push(Violation::new(
                        at(&format!(".views[{j}].is_anchor")),
                        format!("{kind} 的主视图必须是 {anchor_name}，不是 {got}"),
                    ));
                }
                // V10：主视图要排在第一位。**顺序就是生成顺序**——主视图不先出，
                // 后面的视图没有参考图可挂。
                if j != 0 {
                    out.push(Violation::new(
                        at(&format!(".views[{j}]")),
                        format!(
                            "主视图 {anchor_name} 排在第 {} 位。views 的顺序就是生成顺序，\
                             主视图必须排第一——它先出、单独出，后面每一张都要拿它当参考图",
                            j + 1
                        ),
                    ));
                }
            }
            0 => out.push(Violation::new(
                at(".views"),
                format!(
                    "没有主视图。{kind} 的主视图是 {anchor_name}，\
                     它先出、单独出，其余视图都以它为参考图"
                ),
            )),
            n => out.push(Violation::new(
                at(".views"),
                format!(
                    "有 {n} 个主视图。每张卡有且仅有一个——\
                     并行出多个「主」视图，出来的是几个长得像但不是同一个人的角色"
                ),
            )),
        }

        let mut aspects = std::collections::BTreeSet::new();
        for (j, view) in views.iter().enumerate() {
            let name = view.get("view").and_then(|n| n.as_str()).unwrap_or("");
            let is_anchor = view.get("is_anchor").and_then(|b| b.as_bool()) == Some(true);
            let derived: Vec<&str> = view
                .get("derived_from")
                .and_then(|d| d.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            if let Some(a) = view.get("aspect").and_then(|a| a.as_str()) {
                aspects.insert(a.to_string());
            }
            // V13：视图的提示词必须逐字包含**本卡自己的** identity_prompt。
            //
            // 「一次写定、逐字复用」是这个阶段的一致性手段（SPEC-0016 §6.3），
            // 但在这条校验之前它只是一句约定，没有任何东西机械地查过。
            //
            // 2026-09-05 的十阶段端到端就栽在这里：Agent 把**角色卡的**身份锁
            // 抄进了全部三张卡的全部 15 个视图，于是场景卡和道具卡出来的都是
            // 那个角色。控制面忠实渲染了它被告知的东西，一路绿到人眼看图才发现。
            //
            // 「本卡自己的」是这条校验的全部重点：场景卡的身份锁是那个空间，
            // 道具卡的是那个道具，都不是角色。
            //
            // 判据是**开头**，不是「包含」。`contains` 不够：这次那份计划里，
            // 角色卡的身份锁末尾恰好是「手持无标识透明饮料杯。」，把道具卡的
            // 身份锁整个包了进去——于是道具卡那四个视图用 `contains` 查是「通过」的，
            // 而它们画的全是角色。技能清单里写的本来就是「以 identity_prompt
            // 逐字开头」，按它来。
            //
            // 两边都按空白归一化再比。**误拒比漏判更难查**：这条校验挡下来的
            // 计划 Agent 会重试，而它看不出「我明明抄对了」和「首尾多了个空格」
            // 的区别，于是在一份本来正确的计划上反复撞墙。空白差异不是这条规则
            // 要管的事。
            let prompt = view
                .get("prompt")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .trim_start();
            let identity = identity.trim();
            if !identity.is_empty() && !prompt.starts_with(identity) {
                out.push(Violation::new(
                    at(&format!(".views[{j}].prompt")),
                    format!(
                        "没有以本卡的 identity_prompt 逐字开头。同一张卡的每个视图都要\
                         以它开头，一个字不改——那是这张卡里「是同一个东西」的唯一凭据。\
                         本卡的是：「{identity}」。\
                         （别抄别的卡的：场景卡的身份锁是那个空间，道具卡的是那个道具，\
                         都不是角色。）"
                    ),
                ));
            }
            let at_field = at(&format!(".views[{j}].derived_from"));
            if is_anchor {
                if !derived.is_empty() {
                    out.push(Violation::new(
                        at_field,
                        "主视图自己不参考任何视图，这里不要填",
                    ));
                }
                continue;
            }
            // V11：非主视图必须挂参考，第一项是主视图，其余只能指向排在自己
            // 前面的视图——顺序即生成顺序，指向后面的视图那时还不存在。
            if derived.is_empty() {
                out.push(Violation::new(
                    at_field,
                    format!(
                        "非主视图必须挂参考图，第一项是 {anchor_name}。\
                         没有参考就没有锚点，出来的是长得像但不是同一个人"
                    ),
                ));
                continue;
            }
            if derived[0] != anchor_name {
                out.push(Violation::new(
                    at_field.clone(),
                    format!(
                        "{name} 的 derived_from 第一项必须是主视图 {anchor_name}，写的却是 {}",
                        derived[0]
                    ),
                ));
            }
            let earlier: Vec<&str> = views[..j]
                .iter()
                .filter_map(|v| v.get("view").and_then(|n| n.as_str()))
                .collect();
            for d in &derived {
                if !earlier.contains(d) {
                    out.push(Violation::new(
                        at_field.clone(),
                        format!(
                            "{name} 参考了 {d}，但它不在本卡里、或者排在 {name} 后面还没生成。\
                             只能挂**已经定稿**的视图；本卡在它之前的有：{}",
                            if earlier.is_empty() {
                                "（一个都没有）".to_string()
                            } else {
                                earlier.join("、")
                            }
                        ),
                    ));
                }
            }
            // V12：FLUX.2 一次最多吃 10 张参考，多了图会被拒。
            if derived.len() > MAX_CARD_REFERENCES {
                out.push(Violation::new(
                    at_field,
                    format!(
                        "挂了 {} 张参考，上限是 {MAX_CARD_REFERENCES}。\
                         超出的部分卡片后端不吃，图会被判非法",
                        derived.len()
                    ),
                ));
            }
        }
        if aspects.len() > 1 {
            out.push(Violation::new(
                at(".views"),
                format!(
                    "同一张卡出现了多种画幅（{}）。一张竖一张方会让人\
                     误以为是不同批次生成的",
                    aspects.into_iter().collect::<Vec<_>>().join("、")
                ),
            ));
        }
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

    fn plan() -> Outputs {
        fixtures::outputs(StageId::VisualAssets)
    }

    fn refuses(o: &Outputs, needle: &str) {
        let e = validate(StageId::VisualAssets, o)
            .expect_err(&format!("这份计划应当被挡下（{needle}）"));
        assert!(
            e.message().contains(needle),
            "错误没说到点子上（要找 {needle}）：{}",
            e.message()
        );
    }

    /// 少一个角度就少一处可比对的参照——这是「多角度」这个要求的全部意义。
    #[test]
    fn a_character_card_missing_a_required_view_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v["view"] != "back");
        refuses(&o, "缺必需视图 back");
    }

    /// 并行出多个「主」视图，出来的是几个长得像但不是同一个人的角色。
    #[test]
    fn two_anchors_on_one_card_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"][1]["is_anchor"] = json!(true);
        refuses(&o, "有 2 个主视图");
    }

    #[test]
    fn a_non_anchor_view_without_an_anchor_to_derive_from_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"][2]
            .as_object_mut()
            .unwrap()
            .remove("derived_from");
        refuses(&o, "非主视图必须挂参考图");
    }

    /// V11：第一项必须是主视图。挂了参考但没挂锚点，等于没锚。
    #[test]
    fn derived_from_not_starting_at_the_anchor_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"][2]["derived_from"] = json!(["three_quarter"]);
        refuses(&o, "第一项必须是主视图");
    }

    /// V11：只能挂**已经定稿**的视图——指向排在自己后面的，那时它还不存在。
    #[test]
    fn deriving_from_a_later_view_is_refused() {
        let mut o = plan();
        let last = o["asset_plan"]["assets"][0]["views"]
            .as_array()
            .unwrap()
            .len()
            - 1;
        let last_name = o["asset_plan"]["assets"][0]["views"][last]["view"]
            .as_str()
            .unwrap()
            .to_string();
        o["asset_plan"]["assets"][0]["views"][1]["derived_from"] = json!(["front_full", last_name]);
        refuses(&o, "还没生成");
    }

    /// V10：主视图必须排第一——顺序就是生成顺序。
    #[test]
    fn an_anchor_that_is_not_first_is_refused() {
        let mut o = plan();
        let views = o["asset_plan"]["assets"][0]["views"]
            .as_array_mut()
            .unwrap();
        views.swap(0, 1);
        refuses(&o, "views 的顺序就是生成顺序");
    }

    /// V12：超过卡片后端的参考上限，图会被判非法，要在提交时就挡下。
    #[test]
    fn too_many_references_on_one_view_are_refused() {
        let mut o = plan();
        let mut refs = vec!["front_full".to_string()];
        refs.extend((0..MAX_CARD_REFERENCES).map(|i| format!("x{i}")));
        o["asset_plan"]["assets"][0]["views"][1]["derived_from"] = json!(refs);
        refuses(&o, "上限是 10");
    }

    #[test]
    fn mixing_aspects_within_one_card_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"][4]["aspect"] = json!("1:1");
        refuses(&o, "多种画幅");
    }

    /// 卡片可以写得更细，但不能是另一段近义改写——那样卡上的人和成片里
    /// 的人就不是同一个人了。
    #[test]
    fn a_card_identity_that_paraphrases_the_lock_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["identity_prompt"] =
            json!("20岁女生，黑长发，白裙子，白板鞋，小挎包，鹅蛋脸");
        refuses(&o, "没有逐字包含 consistency_lock.character");
    }

    /// V13：视图的提示词要以**本卡自己的** identity_prompt 逐字开头。
    ///
    /// 2026-09-05 的十阶段端到端栽在这里：Agent 把角色卡的身份锁抄进了三张卡
    /// 全部 15 个视图，于是场景卡和道具卡出来的都是那个角色。控制面忠实渲染了
    /// 它被告知的东西，一路绿到人眼看图才发现。
    #[test]
    fn a_view_prompt_that_does_not_start_with_its_own_card_identity_is_refused() {
        let mut o = plan();
        let other = "完全不相干的一段身份锁";
        o["asset_plan"]["assets"][0]["views"][1]["prompt"] =
            json!(format!("{other}，四分之三侧身全身"));
        refuses(&o, "没有以本卡的 identity_prompt 逐字开头");
    }

    /// **判据是「开头」，不是「包含」。**
    ///
    /// 真实那份计划里，角色卡的身份锁末尾恰好是「手持无标识透明饮料杯。」，
    /// 把道具卡的身份锁整个包了进去——用 `contains` 查，道具卡那四个画着角色的
    /// 视图是「通过」的。这条测试守着这个区别。
    #[test]
    fn a_prompt_that_merely_contains_the_identity_somewhere_is_still_refused() {
        let mut o = plan();
        let identity = o["asset_plan"]["assets"][0]["identity_prompt"]
            .as_str()
            .unwrap()
            .to_string();
        o["asset_plan"]["assets"][0]["views"][1]["prompt"] =
            json!(format!("一段别的开头，中间才提到{identity}"));
        refuses(&o, "没有以本卡的 identity_prompt 逐字开头");
    }

    #[test]
    fn a_scene_view_on_a_character_card_is_refused() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["views"][1]["view"] = json!("establishing");
        refuses(&o, "不是 character_card 的视图");
    }

    /// 参照类资产不强制多视图——它们本来就只有一张图。
    #[test]
    fn a_style_reference_needs_no_views() {
        let mut o = plan();
        o["asset_plan"]["assets"][0]["asset_kind"] = json!("style_reference");
        o["asset_plan"]["assets"][0]["views"] = json!([
            { "view": "front_full", "is_anchor": true, "aspect": "9:16",
              "prompt": "一张色调参照", "status": "planned" }
        ]);
        assert!(validate(StageId::VisualAssets, &o).is_ok());
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
