# 提示词架构重构说明书

> 本文只做设计，不改任何现有文件。目标是回答两个问题：
> **现在的流程哪里可以优化**，以及**为什么产出干巴、该怎么从架构上治**。
>
> 结论先放这里：产出干巴不是「提示词写得不够漂亮」，而是随包分发的提示词
> **只有契约层，没有方法层、词表层和模型适配层**；同时有四处上下文断层，
> 让好不容易写出的内容在进模型之前就被丢掉了。前者让 Agent 不知道什么算好，
> 后者让写得再好也不生效。
>
> 其中最严重的一处断层是**角色卡与场景卡根本没有被生成过**——
> `visual_assets` 标着 Hybrid 却没有执行器，卡片是纸面计划，也没有通道把图喂给视频模型。
> §6 单开一章处理它：双图像后端（OpenAI 优先、Z-Image 兜底）+ 多视图卡片规格。

---

## 0. 调研材料清单

调研分四块：视频模型的提示词语法、影视创作方法、GitHub 上高星的导演/编剧/摄影技能、
长文本（小说）生成的工程化方法。下面只留下**对本项目有直接可移植结论**的部分。

### 0.1 视频模型提示词语法（适配层的证据）

| 来源 | 可移植结论 |
|---|---|
| [MiniMax Hailuo 提示词指南](https://minimax-ai.chat/guide/hailuo-video-prompts/) | 公式 `[运镜指令] 主体 + 可见动作 + 场景 + 光线/质感 + 连续性约束`；**15 个运镜指令是受控词表**（`[Push in]` `[Truck left]` `[Tracking shot]` `[Static shot]` …），一次组合不超过 3 个；**明确不要发明 `negative_prompt` 参数**，约束写进正向提示词（「one continuous shot」「主体保持居中」） |
| [Hailuo 2.3 指南](https://akool.com/blog-posts/minimax-hailuo-video-prompt-guide) / [Segmind](https://blog.segmind.com/hailuo-minimax-ai-video-prompt-guide/) | 一条提示词只描述**一个可读镜头**：一个清晰主体、一个可见动作、一个受控环境、一个有意图的运镜。混多个运镜是最常见的失败原因 |
| [Wan 2.2 提示词指南](https://www.viewcomfy.com/blog/wan2.2_prompt_guide_with_examples) / [instasd](https://www.instasd.com/post/wan2-2-whats-new-and-how-to-write-killer-prompts) | 结构是 `开场画面 → 摄影机运动 → 揭示/落点`，**80–120 词**最稳；美学控制是分类词表：光源、光型、景别、角度、镜头、运镜、色调；负向提示词在 2.2 上**是被真正执行的** |
| [LTX-2 提示词指南](https://www.dreampixelforge.com/blog/ltx-prompts) / [官方仓库](https://github.com/Lightricks/LTX-2) | 音视频同生成模型：**环境声、拟音、音乐、对白要显式写进提示词**，对白放引号里并注明语言/口音；尺寸须被 32 整除，帧数须为 8n+1 |
| [Veo 3 JSON 提示词实践](https://artlist.io/blog/veo-3-json-prompts/) | 结构化提示词字段化是主流做法：`shot{type,framing,camera}` / `subject{description,wardrobe}` / `scene{location,time_of_day}` / `lighting` / `audio{ambient,dialogue}` / `negative_prompt`；**前 10 个 token 权重最高**，最重要的视觉元素必须放句首 |

**对本项目最重要的一条**：不同基线的能力面**不一样**，提示词写法必须按基线分叉。
这不是风格问题，是「写了会不会生效」的问题——见 §2.4。

### 0.2 GitHub 上的导演/编剧/摄影技能（架构范式的证据）

| 仓库 | 架构上值得抄的东西 |
|---|---|
| [wuwangzhang1216/DirectorSKILL](https://github.com/wuwangzhang1216/DirectorSKILL) | **SKILL.md 只做路由**，12 份 `references/` 按需加载（镜头语言、调度、光色、声音、剪辑、类型片打法、提示词词库、失败模式、模型适配器、一致性圣经）；镜头字段是 9 项**决策**而非描述；19 条编码失败模式 F1–F19，每条带「症状 → 按概率排序的成因 → 按成本排序的修法」，配三振规则（同一修法失败 3 次就重新规划这一镜，不再重摇）；四种提示词形状 S1–S4 按模型能力面选择 |
| [smixs/visual-skills](https://github.com/smixs/visual-skills) | **七条不可选法则**：场景公式（欲望+阻碍+几何+视线+节奏）、每镜三个物理事实、Murch 剪辑六法则（情感 51% / 故事 23% / 节奏 10% / 视线 7% / 银幕平面 5% / 三维空间 4%）、三职能筛选（每镜必须改变情绪、推进动作或增加压力，否则删）、调度、节奏（长→短→短→停→击）、14 字段镜头卡；**明令禁用词**：cinematic / epic / stunning / masterpiece / beautiful lighting / dynamic camera / "he is sad"；每模型一份语法文件 |
| [OSideMedia/higgsfield-ai-prompt-skill](https://github.com/OSideMedia/higgsfield-ai-prompt-skill) | 32 个子技能按模型与任务切分，参考素材的**角色（role）要显式声明**，不是笼统「参考图」 |
| [anthropics/skills · frontend-design](https://github.com/anthropics/skills/blob/main/skills/frontend-design) | 反套路的写法范式：先列出 AI 的**默认套路目录**并点名「这些是默认值不是选择」，再要求两遍法——先出方案，再拿方案对照 brief 自查「这段是不是我对任何题目都会写的那套」，是就重写；「把大胆用在一个地方」 |
| [conorbronsdon/avoid-ai-writing](https://github.com/conorbronsdon/avoid-ai-writing/blob/main/SKILL.md) | 反 AI 味的**可执行清单**：分级词表（Tier 1 必换 / Tier 2 同段出现 2 个才报 / Tier 3 高密度才报）、结构红旗（句长段长齐平、冒号接三项排比、模板化开头）、按严重度 P0/P1/P2 排序、按语境放松规则 |
| [Anthropic Agent Skills 最佳实践](https://www.newsletter.swirlai.com/p/agent-skills-progressive-disclosure) | 渐进披露三级：发现（80–100 token 的 name+description）→ 激活（SKILL.md < 5k token）→ 执行（按需读 references）；**不是每次都要用的东西一律下沉到 reference 文件** |

### 0.3 编剧与小说的工程化方法

| 来源 | 可移植结论 |
|---|---|
| [Save the Cat 15 拍](https://www.studiobinder.com/blog/save-the-cat-beat-sheet/) | 结构是**可校验的清单**（开场画面 → … → 终场画面），不是玄学；LLM 用它的前提是把拍点变成结构化字段 |
| [NousResearch/autonovel](https://github.com/NousResearch/autonovel) | 五层协同文件：`voice.md`（怎么写）/ `world.md`（有什么）/ `characters.md`（谁在动）/ `outline.md`（发生什么）/ 正文，外加横切的 `canon.md`（硬事实库）；**双免疫系统**：机械层用正则查禁用词、陈词、告知代替呈现，LLM 评委层查语气一致性与人物辨识度；分数不达标就重写（地基 7.5、章节 6.0），并有停滞检测防止无限打磨 |
| [红果短剧编剧教程·节奏](https://www.juben.pro/a/1-1783.html) | 单集 1–3 分钟的节奏模板：**前 30 秒**交代背景/困境/动机勾住人，**中间 1 分钟**推进主线且「字少、精准、符合人设」，**最后 30 秒**留强悬念；小高潮 3–5 集一次，大高潮 10 集一次 |
| [抖音脚本方法论](https://zhuanlan.zhihu.com/p/321393661) / [小红书钩子](https://zhuanlan.zhihu.com/p/1952311002839384362) | 前 3 秒必须出现冲突、震惊画面、悬念或反差；文案是**念出来**的，要口语、短句 |
| [创意写作 LLM 评分研究](https://www.mdpi.com/2076-3417/15/6/2971) | rubric 要多维度（具体性、原创性、感官细节、连贯性）且**每档配范例**，只给形容词的 rubric 不可靠 |

### 0.4 一致性方法（视觉资产阶段的证据）

来自 [Kling 角色一致性指南](https://kling.ai/blog/ai-character-consistency-guide)、
[MindStudio 分镜与角色卡](https://www.mindstudio.ai/blog/storyboards-character-sheets-ai-video-generation)、
[invideo 参考图用法](https://invideo.io/faq/how-do-you-use-reference-images-in-ai-video-generation/)：

- **先用静图锁住身份，再动**。文字描述锁不住人物，只有图能。
- 角色卡要**转身图**（正 / 侧 / 四分之三 / 背 + 特写），不是一张正面照。
- **首帧图（i2v）不等于参考图（r2v）**：首帧只喂第一帧，片子往后放身份就漂；
  参考图是全程锚点。这条直接决定本项目该用哪个 workflow。
- 跨镜头连续性靠**从已通过的镜头里抽帧**当下一镜的参考。

### 0.5 图像后端（§6 的技术前提）

| 来源 | 关键事实 |
|---|---|
| [OpenAI gpt-image-2 文档](https://developers.openai.com/api/docs/models/gpt-image-2)、[images.edit 参考](https://developers.openai.com/api/reference/resources/images/methods/edit)、[图像生成指南](https://developers.openai.com/api/docs/guides/image-generation) | 编辑接口最多接受 **16 张参考图**且自动高保真保留细节；尺寸只有 1024×1024 / 1024×1536 / 1536×1024 三档；quality low/med/high；n ≤ 10；png/webp/jpeg；支持透明背景；**没有 negative_prompt** |
| [Z-Image Turbo · ComfyUI 官方教程](https://docs.comfy.org/tutorials/image/z-image/z-image-turbo)、[模型说明](https://comfyui.org/en/z-image-turbo-in-comfyui-realism) | 通义实验室 6B、Apache-2.0；Turbo 仅 8 NFE、16GB 显存可跑、4090 上 1024px 约 2–3 秒；有 **Z-Image-Edit** 变体做指令编辑；ComfyUI 有官方工作流，走正负提示词 |

---

## 1. 现状：提示词是怎么生成的

事实源在 `crates/studio-cli/src/assets.rs:26`：一个 `SKILLS: [SkillDoc; 10]` 常量数组，
每个 skill 只有五个散文字段（`trigger` / `not_trigger` / `duties` / `notes` + description），
其余段落由 `skill_md()`（`crates/studio-cli/src/assets.rs:303`）从阶段图、schema、
工具注册表、错误码枚举拼出来。

以 director（分镜）为例，`assets/skills/director/SKILL.md` 全文 62 行，拆开看：

| 段落 | 行数 | 性质 |
|---|---|---|
| 触发 / 不触发 | 4 | 路由 |
| 职责（4 条） | 6 | **创作指导（全部）** |
| 输入输出（必填字段） | 10 | 契约，机器生成 |
| 确认点 | 5 | 契约，机器生成 |
| 失败与恢复 | 3 | 契约，机器生成 |
| 注意（1 条） | 3 | 创作指导 |
| Studio MCP 工具列表 | 15 | 契约，机器生成 |

**一个导演能拿到的全部创作指导是 5 句话、约 200 字。** 其中还有一句是讲时长对齐的流程约束。
prompt（提示词编译）阶段同理：4 条职责 + 1 条注意，没有一个字提到目标模型的语法。

这套结构在它自己的目标上是成功的——`assets.rs:470` 起的一串测试守着「文档不引用不存在的工具」
「文档不指向源码」「不泄漏二进制名」，[design.md](design.md) §1 记录的那次翻车确实被治住了。
问题是它**只解决了「Agent 会不会绕过协议」，没有解决「Agent 产出的内容好不好」**。

---

## 2. 诊断：干巴的四个成因

### 2.1 只有契约层，没有方法层

Agent 拿到的是一张表格和一句「把每一拍变成可拍的镜头」。它没有被告知：

- 一个镜头凭什么存在（visual-skills 的三职能：改变情绪 / 推进动作 / 增加压力，三者都不占就删）
- 一个镜头要写满哪些**物理事实**（环境压力 + 身体微动作 + 声音锚点）
- 什么样的描述对模型有效（可见的动作 vs 内心状态：`he is sad` 是无效描述）
- 节奏怎么排（长→短→短→停→击；前 3 秒必须给冲突）

于是它只能填表。填表的产物就是「主体：一位女性；动作：走路；光线：柔和的光」——
字段齐全、schema 通过、毫无信息量。**这就是干巴的直接来源**：schema 校验的是形状，
形状对了内容可以是空的。

### 2.2 没有词表层，镜头语言无法落到模型指令

`assets/schema/storyboard.json` 里：

```json
"shot_size":     { "description": "景别", "type": "string" },
"camera_motion": { "description": "镜头运动。每镜只保留一个主运动", "type": "string" },
"lighting_color":{ "description": "灯光与色调", "type": "string" }
```

三个自由字符串，没有枚举、没有取值示例、没有和模型词表的对应关系。
而 MiniMax 的运镜是**受控指令**——写 `[Push in]` 生效，写「镜头缓缓推近，充满诗意」不生效。
分镜阶段写的是散文，提示词阶段就得二次翻译，翻译过程没有任何约束，全靠 Agent 自由发挥。

同样的问题在 `lighting_color`：Wan 2.2 的光源/光型是分类词表（日光/月光/实用光/荧光 ×
柔光/硬光/顶光/侧光/逆光/轮廓光/剪影），写在词表内的词能被模型稳定理解，写「温暖的氛围光」
就是掷骰子。

### 2.3 没有质量闸，只有格式闸

现在提交前的全部自动校验是：JSON Schema + 时长求和（`script` 各拍之和 = 总时长）。
没有任何一条检查内容质量。对照 §0.2 的两个仓库，缺的是：

- **提交前自检清单**（visual-skills 的六点戏剧法检查、DirectorSKILL 的 self-check）
- **禁用词表**（cinematic / epic / 史诗般的 / 电影感的 —— 这些词对模型是噪声，对人是废话）
- **失败模式手册**（「幻灯片感」「身份漂移」「多运镜打架」各自的成因与修法）

`review` 阶段倒是有验收，但它只跑 ffprobe 的机械指标（时长、编码、字幕存在性），
不看内容。也就是说**从头到尾没有一处在问「这东西好不好看」**。

### 2.4 四处上下文断层：写了也不生效

这一节是本次调研中最要紧的发现。前三条是提示词层面的「不知道怎么写好」，
这一条是**写好了也进不去模型**。四处断层都有代码证据。

#### 断层 1：`negative` 在 minimax_h3 上被静默丢弃

- 各基线的可注入参数由 `_studio.bindings` 声明（`crates/studio-pipeline/src/workflow.rs:154` 的 `apply()`），
  **基线里没有绑定的参数直接跳过，不报错**。
- 实测各基线的绑定键：

  | 基线 | 已核验 | 可注入参数 |
  |---|---|---|
  | `minimax_h3/{t2v,i2v,r2v}` | 是 | positive, width, height, length_frames, fps, seed |
  | `wan2_2/t2v` | 是 | positive, **negative**, width, height, length_frames, fps, seed |
  | `wan2_2/i2v`、`wan2_2/flf2v` | **否** | 仅 seed |
  | `ltx2_5/{t2v,i2v}` | 是 | positive, negative, width, height, **duration_seconds**, fps, seed |
  | `wan_animate2/i2v` | **否** | 仅 seed |

- 而 prompt skill 的职责第一条写着「逐镜头给出正向、**负向**提示词」，
  `prompt_pack.json` 也有 `negative` 字段。
- 默认核心系列是 `minimax_h3`（`config/models.toml`、`.env.example`）。
  **Agent 认真写的负向提示词，在默认路径上 100% 被丢弃。**
- 而调研结论恰好相反：MiniMax 官方文档明确说不要发明 `negative_prompt`，
  约束要写进正向提示词。也就是说正确写法是「把负向约束改写成正向的连续性约束」——
  但没有任何文档告诉 Agent 这件事。

#### 断层 2：`length_frames` 在 ltx2_5 上被静默丢弃

`ltx2_5` 绑的是 `duration_seconds`，而 `prompt_pack.json` 的必填项是 `length_frames`，
schema 里根本没有 `duration_seconds`。结果：换到 LTX 基线，**镜头时长完全不受提示词包控制**，
用基线默认值渲染，然后 `post` 阶段拼接、`review` 阶段核对总时长——对不上，
但没人知道是为什么。

#### 断层 3：视觉资产根本进不了渲染

`prompt_pack.shots.references` 是 `asset_id` 数组，visual skill 要求「引用视觉资产用
asset_id，不要重复描述角色外观」。但**所有基线的 bindings 里都没有 `references`、
也没有任何图片输入参数**。渲染时 `references` 和 `negative` 走同一条路——被跳过。

叠加两件事：

1. `visual_assets` 阶段在阶段图里标 `Hybrid`（「Agent 定内容，控制面执行」，
   `crates/studio-core/src/stage.rs:91`），但 `crates/studio-engine/src/project.rs:210`
   的分发只对 `Deterministic` 注册执行器，**Hybrid 走的是和 Creative 完全一样的分支**。
   全仓库没有任何 `visual_assets` 的执行器实现。
2. 于是角色卡、场景卡从来没有被真正生成过，asset_plan 是一份纸面计划。

**这意味着当前架构下的角色一致性是零落地**：既没有生成参考图，也没有通道把图喂给模型，
唯一的一致性手段是每镜重复写外观描述——而 prompt skill 恰好禁止这么做
（「不要重复描述角色外观」）。两头堵死。这是画面质量差最硬的一条原因，
它不是提示词问题，是管线缺环。**补齐方案见 §6，那是本次架构调整里最大的一块。**

#### 断层 4：声音设计在 prompt_pack 处蒸发

`script` 阶段产出了完整的声音时间线（`segments`：说话人、台词、字幕、来源）
和 `audio_policy`（原生音频优先、外部音乐是否禁用）。但 `prompt_pack` 的 shot 只有
`positive`/`negative`，**没有任何音频字段**，prompt skill 的职责里也一个字没提音频。

而 `minimax_h3` 的模型契约里明明有 `minimax_h3_audio_vae_fp32.safetensors`——
它是音视频联合生成模型；LTX-2 同样是原生音视频模型，官方指南明确要求把环境声、
拟音、音乐、对白写进提示词，对白放引号里。

**结论：本项目选的两个主力模型都会出声音，而提示词架构里没有给声音留位置。**
原生音频能力被整条丢掉，成片只能靠后期贴字幕。

---

## 3. 流程本身的四个可优化点

上面是提示词与管线的问题。阶段图本身也有四处值得改，按价值排序：

### 3.1 `selection` 没有候选集可选（高价值）

`idea` 产出**一份** brief，`selection` 的职责是「从可行性、受众匹配和发布风险筛选 brief」，
schema 要求 `recommendation`（单数）+ `tradeoffs`。可是只有一个方案，筛选就是自问自答，
「推荐它牺牲了什么」也只能编。用户遇到的第一道确认门（`selection.approval`）
问的是「方向对不对」，但没有另一个方向可以比。

**改法**：`idea` 阶段产出 2–3 个**互斥**的 concept 候选（各自不同的钩子策略/叙事角度），
`selection` 变成真正的取舍，确认门给用户看的是「A / B / C 选一个」。
这同时解决了另一个问题——用户第一次表态被推迟到了第二阶段，而 idea 阶段没有门，
一个走偏的 brief 会带着整条链跑下去。

### 3.2 缺关键帧控制点（高价值）

DirectorSKILL 的核心判断是「keyframe 是控制点，修一张图比重摇一段视频便宜得多」，
一致性调研的共识是「先用静图锁住身份，再动」。当前流程是
`storyboard`（散文分镜）→ `visual_assets`（无人执行的计划）→ `prompt_pack`（直接写视频提示词），
中间**没有任何一帧图被真正生成和确认**。第一次看到画面是 `preview` 的 480p 视频，
那时错的已经是运动、身份、构图三件事叠加，无法归因。

**改法**：把 `visual_assets` 的 Hybrid 语义真正落地——它生成角色卡、场景卡与每镜首帧候选图，
`visual_assets.approval` 变成**看图确认**而不是看计划确认。完整方案见 §6。

前提是**先给基线补上图片输入的绑定**（i2v 的首帧、r2v 的参考图），否则确认了也喂不进去。
`minimax_h3` 的 SOURCE-README 说明 i2v 接受首帧（尾帧可选）、r2v 接受图/视频/音频参考——
能力是有的，只是 `_studio.bindings` 还没绑。

### 3.3 修订意见用完即弃（中价值）

`studio.revise(stage, message)` 的 message 只用于当次重做，没有沉淀。用户说过
「不要固定 2 秒」，下一部作品、甚至本作品的下一次修订，Agent 依然可能犯。
autonovel 的做法是维护一份横切的 `canon.md` 硬事实库，每次生成都注入。

**改法**：作品级累积一份「决定档案」（用户否决过什么、确认过什么口味、
哪些是本片的不可变约束），由控制面维护，在 `next_action` 里回给 Agent。
这是唯一一处需要动状态存储的建议，但它对「越用越准」的价值最大。

### 3.4 `review` 只有机械指标（中价值）

`review` 逐条核对 `idea` 阶段的 `success_metrics`，但检查项全部基于 ffprobe 实测元数据。
`success_metrics` 里但凡写了「钩子在前 3 秒成立」这种内容性指标，就无法验证。

**改法**：把 review 拆成两半——**技术验收**（确定性，保持现状）与
**内容验收**（Agent 侧，按 rubric 逐条自评并给出证据）。后者不必阻塞交付，
但要进 timeline，让人能看到「这片子按自己定的标准打了几分」。

---

## 4. 重构：四层提示词架构

```
L0 契约层  Contract    交什么、怎么交、被挡住怎么办        机器生成，已有，保持
L1 词表层  Lexicon     每个字段允许写什么词                 机器生成 + 可校验    ← 新增
L2 方法层  Doctrine    怎么想、什么算好、怎么自查           人写、代码分发       ← 新增
L3 适配层  Adapter     这条基线吃什么、不吃什么             从 bindings 投影     ← 新增

横切：质量闸 Gate · 范例库 Exemplars · 失败手册 Failure · 作品记忆 Canon
```

四层的分工原则：**L0 保证不出协议错，L1 保证不出词汇错，L2 保证不出判断错，
L3 保证不出「写了没用」的错。** 现在只有 L0。

### 4.1 L1 词表层：让 schema 承担词表契约

schema 已经是 Agent 唯一信任的事实源（AGENTS.md 三条工作习惯的第二条就是
「提交前先调 `studio.schema`，不要猜字段」）。把词表放进 schema，是**成本最低、
命中率最高**的一处改动——Agent 本来就会读。

具体到 `storyboard.shots`：

| 字段 | 现状 | 改成 |
|---|---|---|
| `shot_size` | 自由字符串 | `enum`：extreme_wide / wide / medium_wide / medium / medium_close / close / extreme_close |
| `angle` | 自由字符串 | `enum`：eye_level / low / high / overhead / dutch / over_shoulder / pov |
| `camera_motion` | 自由字符串 | `enum`：static / push_in / pull_out / pan_left / pan_right / tilt_up / tilt_down / truck_left / truck_right / pedestal_up / pedestal_down / zoom_in / zoom_out / tracking / handheld_shake<br>**与 MiniMax 的 15 个运镜指令一一对应**，适配层负责翻译成 `[Push in]` |
| `lighting_key` | 无（现在混在 `lighting_color` 里） | `enum` 光型：soft / hard / top / side / back / rim / silhouette / bottom |
| `lighting_source` | 无 | `enum` 光源：daylight / moonlight / practical / firelight / fluorescent / overcast / mixed / artificial |
| `color_tone` | 无 | 自由，但 description 给出可用词族（teal_orange / bleach_bypass / kodak_portra / 低饱和暖调…） |
| `three_facts` | 无 | **新增必填**，`minItems: 3` 的字符串数组：环境压力、身体微动作、声音锚点各一条。这一条直接对治「干巴」 |
| `shot_function` | 现有 `purpose`（自由） | `enum`：change_emotion / advance_action / raise_pressure（三职能，三者都不占的镜头不允许存在） |
| `audio` | 无 | 新增对象：`ambient` / `foley` / `dialogue{text,language,speaker}` / `music`，与 script 的 segments 对齐 |

同类改动适用于 `script`（拍点类型枚举、钩子位置、台词密度约束）与 `prompt_pack`（见 §4.3）。

**注意**：全部 schema 目前是 `additionalProperties: true`，加枚举不影响向后兼容，
但会让 `schema_violation` 变得更有用——错误消息会精确指到「`storyboard.shots[2].camera_motion`
不在允许值里」，比现在放行一个模型听不懂的词强得多。

### 4.2 L2 方法层：按需加载的 doctrine

新增一批随包分发的方法文档，SKILL.md 退化成**路由 + 触发 + 硬规则 + 自检清单**（目标 ≤ 120 行），
真正的方法论下沉：

```
.agents/
├── skills/<name>/SKILL.md            # 路由；何时读哪份 doctrine
└── doctrine/
    ├── story/structure.md            # 三幕 / Save the Cat 15 拍 / 短剧卡点模板（前30s-中1min-后30s）
    ├── story/hook.md                 # 前 3 秒钩子的五种做法与反例
    ├── story/voice.md                # 口播文案规则（念出来、短句、口语）+ 反 AI 味词表
    ├── camera/grammar.md             # 景别/角度/运动/轴线/30度规则/视线匹配；三职能；每镜一个主运动
    ├── camera/lighting.md            # 光源×光型×色调词表与组合示例
    ├── camera/blocking.md            # 调度先于构图：起位→动作→落位
    ├── consistency/bible.md          # 一致性锁定：角色外观串、机位签名、环境锁、时代锁
    ├── consistency/character-sheet.md # 多视图卡片规格与身份锁写法（见 §6.3）
    ├── audio/design.md               # 声音三层（环境/拟音/人声）；对白写法；音视频同生成模型的注意事项
    ├── quality/checklist.md          # 提交前六点自检（见 §4.4）
    ├── quality/banned.md             # 禁用词表，分级（见 §4.4）
    ├── failure/modes.md              # F1–Fn 失败模式：症状 → 成因排序 → 成本阶梯修法 → 三振规则
    ├── exemplars/                    # 黄金样例：一部完整作品从 brief 到 prompt_pack 的全部产物
    │   ├── good/ …                   # 每份带批注「这里为什么好」
    │   └── bad/  …                   # 对照组：同一 brief 的干巴版本 + 逐条批注
    └── （模型能力卡见下节，放 .agents/models/）
```

**范例是治干巴最有效的单项手段**。现在 Agent 见不到任何一份「好的分镜长什么样」，
schema 又只给字段名，它唯一的参照就是字段名的字面意思。给它一份带批注的好样例 +
一份带批注的坏样例，比再写十条职责有用。

#### 怎么和「Markdown 不手写」的硬规则共存

CLAUDE.md 硬规则 2 要求随包 Markdown 由 `emit-assets` 生成、CI 跑 `--check`。
方法层是大段散文，塞进 Rust 字符串字面量既难写也难 review。建议：

- 散文以**源文件**形式放在 `crates/studio-cli/assets/doctrine/**.md`，
  用 `include_str!` 编进二进制；
- `emit-assets` 负责把它们物化到 `assets/` 与 bundle 的 `.agents/doctrine/`，
  `--check` 照常守着输出一致性；
- 「文档是代码的投影」这条精神不变——事实源仍然唯一，只是从字符串字面量
  变成了参与编译的源文件。涉及工具名/阶段名/错误码的段落**仍然禁止**出现在
  doctrine 里，由生成器插入，`assets.rs` 现有的四个泄漏测试（不提源码路径、
  不提二进制名、只引用真实工具名）要**扩展到 doctrine 全体文件**。

#### Agent 怎么读到它们

AGENTS.md 现在说的是「不要用 shell 去读写这个目录里的状态」，针对的是 `.studio/`。
需要明确区分：**`.studio/` 是禁区；`.agents/doctrine/` 是给你读的**。
在 SKILL.md 的路由段直接给出相对路径，并在 `next_action` 里回一个 `doctrine`
字段列出本阶段该读的文件——不确定就读，别猜。

### 4.3 L3 适配层：模型能力卡

每个模型系列一份能力卡，**由 `_studio.bindings` 投影生成**，加一段人写的语法要点：

```
.agents/models/minimax_h3.md
```

内容分两半：

**机器生成的部分**（事实源是基线文件，改基线自动改文档）：

| 项 | 值 |
|---|---|
| 可注入参数 | positive, width, height, length_frames, fps, seed |
| **不支持** | negative（写了会被丢弃）、references（写了会被丢弃） |
| 已核验模式 | t2v / i2v / r2v |
| 时长参数 | `length_frames` + `fps` |

**人写的部分**（进 doctrine，按系列分文件）：

- 提示词公式：`[运镜指令] 主体 + 可见动作 + 场景 + 光线/质感 + 连续性约束`
- 运镜指令词表（15 个）与「一次不超过 3 个」
- **负向约束的正确写法**：不写 negative，改写成正向连续性约束
  （「one continuous shot」「主体全程居中」「不切场景」）
- 音频写法：这是音视频联合模型，环境声/拟音/对白写进正向提示词，对白放引号并注明语言
- 长度上限：正向提示词 ≤ 2000 字符

对照 `ltx2_5.md` 就完全不同：吃 negative、时长用 `duration_seconds`、
尺寸须被 32 整除、帧数须为 8n+1、对白放引号。
`wan2_2.md` 又不同：80–120 词最稳、negative 真正生效、结构是「开场→运动→落点」。

**这份卡片是「写了没用」问题的正面解药**，也是 §2.4 断层 1、2 的止血方案。

### 4.4 横切机制

#### 质量闸：提交前六点自检

写进每个创作阶段 SKILL.md 的固定段落（由生成器插入，各阶段条目不同），
并在 `next_action.quality_bar` 里同步回传，Agent 提交前逐条过：

分镜阶段示例：

1. 每个镜头都能说出它属于三职能中的哪一个，说不出就删掉这一镜。
2. 每个镜头都有三个物理事实（环境压力 / 身体微动作 / 声音锚点），没有就补。
3. 每个镜头**只有一个**主运镜，且落在允许词表内。
4. 角色外观串逐镜逐字相同（复制粘贴，不要复述）。
5. 没有出现禁用词（见下）。
6. 镜头时长之和 = 剧本各拍之和 = brief 总时长，且**每镜不超过基线的单镜上限**。

#### 禁用词表（分级）

借 avoid-ai-writing 的分级思路，落到视频语境：

- **Tier 1（出现即改）**：cinematic / epic / stunning / masterpiece / 4K / 高质量 /
  电影感的 / 史诗般的 / 唯美的 / 大片质感 —— 对模型是噪声，对人是废话。
- **Tier 1（描述性失效）**：`he is sad` 这类内心状态。改成可见行为：
  「他把下巴埋进围巾，视线避开摄影机」。
- **Tier 2（同一镜出现 2 个才报）**：beautiful / dynamic / dramatic / atmospheric /
  氛围感 / 质感 / 高级感。
- **Tier 3（全片密度超阈值才报）**：形容词堆砌本身——一个镜头里形容词多于名词就是信号。

#### 失败模式手册

至少覆盖这些（每条：症状 → 按概率排的成因 → 按成本排的修法 → 三振后重规划）：

| 编码 | 症状 |
|---|---|
| F1 | 幻灯片感：画面几乎不动 |
| F2 | 身份漂移：同一角色跨镜换脸 |
| F3 | 多运镜打架：画面同时推、摇、跟，结构崩塌 |
| F4 | 时长对不上：成片总时长与剧本不符（先查基线是否吃你写的时长参数） |
| F5 | 提示词写了没生效（查能力卡：这个参数这条基线绑了吗） |
| F6 | 音画不同步 / 该有声音的地方是静音 |
| F7 | 文字水印乱入：模型自己生成了字幕或 logo |
| F8 | 首尾帧突变：转场处不连续 |

现有的 `studio.retry_stage`（执行失败）与 `studio.revise`（内容要改）刚好是
成本阶梯的两级，手册要把「什么症状用哪个」写死——现在 comfyui skill 只说了一句
「内容/提示词本身要改用 revise」，不够细。

#### 作品记忆（Canon）

见 §3.3。最小实现：控制面在 `revise` 时把 message 归档，
`next_action` 里带一个 `decisions` 数组回给 Agent。

---

## 5. 各阶段重构详案

| 阶段 | 注入的方法（L2） | schema 要动的地方（L1） | 自检要点 |
|---|---|---|---|
| `idea` | 钩子五法、受众/平台差异、短剧节奏模板 | **产出 2–3 个互斥 concept**；`success_metrics` 区分技术指标与内容指标 | 每个 concept 的钩子必须落在前 3 秒；假设写进 assumptions |
| `selection` | 取舍框架（可控性/成本/风险三轴打分） | `candidates[]` + `recommendation` 指向其一 + 每个候选的 `tradeoffs` | 推荐必须说明牺牲了什么；风险三分类 |
| `script` | 三幕/15 拍/短剧卡点；台词口语化与短句；声音三层 | 拍点类型枚举；`hook_at_seconds`；`segments` 与拍点用 id 关联 | 时长精确求和；每拍有明确目的；无口播也要写环境声来源 |
| `storyboard` | 镜头语法、调度先于构图、三职能、Murch 六法则、轴线与 30 度 | 见 §4.1 全表（枚举 + `three_facts` + `shot_function` + `audio`） | 六点自检；每镜一个主运镜；**每镜时长 ≤ 基线单镜上限** |
| `visual_assets` | 一致性方法：多视图卡片规格、身份锚点、首帧 vs 参考图的区别；两个图像后端的能力卡 | 重做，见 §6.5：`identity`（身份锁）+ `views[]`（多视图）+ `provenance`（哪个后端出的） | 每个跨镜复用的角色/场景都有卡；视图齐全；外观串一次写定，后续引用不复述 |
| `prompt_pack` | **按基线分叉的能力卡**；正向公式；负向约束的替代写法；音频写法 | 参数按 workflow 能力面校验（见下）；增加 `audio` 字段；`duration_seconds` 与 `length_frames` 二选一由基线决定 | 提交前对照能力卡逐项确认「这个参数这条基线吃不吃」；禁用词扫描；种子固定 |
| `preview` | 关键帧优先的归因方法 | （见 §3.2 的流程改动） | 先判身份，再判运动，最后判构图 |
| `review` | 内容 rubric（钩子成立/信息密度/节奏/一致性）+ 证据要求 | 检查项区分 `technical` 与 `content` | 技术项必须有 ffprobe 实测值；内容项必须有可指认的时间点 |

### 5.1 prompt_pack 的能力面校验（消灭静默丢弃）

这是配套的引擎改动，属于本次架构的一部分：

- 提交 `prompt_pack` 时，控制面按每个 shot 的 `workflow` 加载基线，
  用 `_studio.bindings` 做**双向校验**：
  - 写了基线不支持的参数（如 minimax_h3 的 `negative`）→ `schema_violation`，
    remedy 指向能力卡与替代写法；
  - 基线要求但没写的参数（如 ltx2_5 的 `duration_seconds`）→ 同样报错。
- `studio.schema("prompt_pack")` 的返回**按当前核心系列动态收窄**——
  Agent 看到的 schema 就是这条基线真正吃的东西。
- 未核验的基线（`wan2_2/i2v` 等）在 schema 层就不出现在可选 workflow 里，
  而不是等到渲染时才报 `model_contract_violation`。

「写了会被忽略」是本项目现在最危险的一种失败——它不报错、不留痕，
只是让画面莫名其妙地不对。**把它变成 schema 错误，是这次重构里性价比最高的一处。**

---

## 6. 视觉资产生成架构（角色卡 / 场景卡）

§2.4 断层 3 指出：角色卡和场景卡**从来没有被生成过**，`visual_assets` 标着 Hybrid
却没有执行器。这一章补齐它，并把「一张大头照」升级成**多视图卡片**。

这是本次调整里唯一需要引入新外部依赖的部分，所以先把边界划清楚。

### 6.1 目标

`visual_assets` 成为真正的 Hybrid 阶段，一次走完三步：

1. **Agent 定身份**：谁需要卡、每张卡的身份锁字符串、需要哪些视图、每个视图的提示词。
2. **控制面生成**：按后端选择规则调图像模型，逐视图出图，落盘并登记产物。
3. **用户看图确认**：`visual_assets.approval` 这道门从「看计划」变成「**看图**」——
   门后面挂的是实际生成的卡片，不是一段 JSON。

第 3 点是这道门存在的意义。现在用户在这道门上确认的是一份自己无法判断真伪的计划，
确认了等于没确认。

### 6.2 双后端与选择规则

| | 后端 A（首选） | 后端 B（兜底） |
|---|---|---|
| 模型 | OpenAI [gpt-image-2](https://developers.openai.com/api/docs/models/gpt-image-2) | [Z-Image](https://docs.comfy.org/tutorials/image/z-image/z-image-turbo)（通义 6B，Turbo / Edit） |
| 通道 | HTTPS → OpenAI Images API | HTTP → ComfyUI（与视频同一条通道） |
| 触发条件 | 环境里有 `OPENAI_API_KEY` | 没有 key，或 key 探活失败 |
| 尺寸 | 只有三档：1024×1024、1024×1536、1536×1024 | ComfyUI 自由设定 |
| 参考图 | `images.edit` 最多 **16 张**输入，高保真自动保留细节 | Z-Image-Edit 走指令编辑；Turbo 纯 t2i |
| 负向提示词 | 无此参数，约束写进正向 | 有，走标准 CLIP 负向通道 |
| 其它 | quality low/med/high、n ≤ 10、png/webp/jpeg、透明背景 | 8 NFE 亚秒级、16GB 显存可跑、Apache-2.0 |
| 成本 | 按张计费，需联网 | 本机 GPU，免费 |

#### 选择规则：整片一次性选定，不逐张回退

**这是关键的架构判断**：两个后端画风不同，同一部作品里混用会直接毁掉一致性——
而一致性正是这个阶段存在的全部理由。所以：

- 后端在 `visual_assets` **阶段开始时**决定一次，写进 `asset_plan.backend`，
  该作品后续所有卡片（包括修订后重生成的）都用同一个后端。
- 换后端的唯一方式是 `studio.revise("visual_assets", ...)` 重做整个阶段，
  且必须在 timeline 里留痕。
- 中途某张图失败**不触发换后端**，走重试；重试到顶就结构化阻塞。

#### 与「不自动降级」硬规则的关系

现有规矩是「核心系列不可用就结构化阻塞，不自动换系列」（`visual` skill 的第四条职责，
`fallback_policy` 字段）。这条规矩**继续适用于视频核心系列**，不放宽。

图像后端是另一回事，理由是：视频核心系列决定成片的画面本身，换了就是换了一部片子；
图像后端只产出**参考素材**，两个后端产出的卡片承担同一个功能（锁住身份），
且用户会在门上亲眼看到结果。所以图像后端允许声明式兜底，但有三条约束：

1. 兜底是**声明式**的：`asset_plan.backend_policy` 必须显式写明「OpenAI 优先、Z-Image 兜底」，
   不是隐式行为。
2. 实际用了哪个必须写进 `asset_plan.backend` 与每张图的 `provenance`，且进 timeline。
3. **两个后端都不可用**时结构化阻塞，不降级成「跳过视觉资产」——
   新错误码 `image_backend_unavailable`（见 §6.8）。

#### 环境变量

```
OPENAI_API_KEY          有则用后端 A。缺失即视为未配置，不报错，直接走兜底
OPENAI_BASE_URL         可选，默认官方端点；自建网关/代理填这里
OPENAI_IMAGE_MODEL      可选，默认 gpt-image-2
IMAGE_BACKEND           可选，显式锁定 openai / z_image，跳过自动探测
```

**一处必须改的代码**：`crates/studio-engine/src/config.rs:229` 的 `is_studio_key()`
白名单目前只放行 `FFMPEG_PATH` / `FFPROBE_PATH` / `COMFY_*` / `CORE_MODEL_FAMILY`。
进程环境里的 `OPENAI_API_KEY` **会被直接丢弃**——不加 `OPENAI_` 前缀，
用户在生产机上 `export OPENAI_API_KEY=...` 是不生效的，只有写进 `.env` 才行。
这与用户的预期（「在生产环境看 OpenAI 的环境变量有没有设置」）不符，必须放行。

#### 密钥安全

`Settings.env` 会被 doctor 输出、trace 记录，渲染时还会把请求体落盘成
`media/*/debug/*.request.json` 供 curl 复现。因此：

- API key **只在内存中传给 HTTP 客户端**，不进任何产物、日志、trace、debug 请求体。
- 所有对外可见的地方只记录后端标识（`openai:gpt-image-2` / `z_image/t2i`）。
- doctor 只报告「已配置 / 未配置」与探活结果，**不回显 key 的任何片段**。
- 需要为此加一条测试：把一个可识别的假 key 塞进 settings，跑一遍 doctor 与
  一次资产生成，断言输出与落盘文件里都搜不到它。

### 6.3 多视图卡片规格

用户的要求是「像市面上已有的经验一样，多个方面，而不是单一的大头照或全身照」。
调研（[Kling 角色一致性](https://kling.ai/blog/ai-character-consistency-guide)、
[MindStudio 分镜与角色卡](https://www.mindstudio.ai/blog/storyboards-character-sheets-ai-video-generation)）
给出的标准做法是**转身图（turnaround）**，落到本项目：

#### 角色卡（`character_card`）

| 视图 id | 内容 | 必需 |
|---|---|---|
| `front_full` | 正面全身，自然站姿，中性表情 | ✅ 主视图 |
| `three_quarter` | 四分之三侧身全身 | ✅ |
| `profile` | 正侧面全身 | ✅ |
| `back` | 背面全身（发型、服装背面） | ✅ |
| `face_close` | 面部特写，中性表情 | ✅ |
| `expressions` | 表情组（本片主导情绪 2–3 种） | 有台词/情绪戏时必需 |
| `hands_props` | 手部与关键道具的持握关系 | 有道具交互时必需 |
| `wardrobe_detail` | 服装材质与关键配饰特写 | 服装是识别点时必需 |

统一约束（写进生成提示词，所有视图共享）：**中性灰底、均匀柔光、无阴影投射、
全身入画不裁切、同一套服装、同一发型**。卡片是**测量用的参考素材**，不是好看的剧照——
戏剧性打光留给成片。

#### 场景卡（`scene_card`）

| 视图 id | 内容 | 必需 |
|---|---|---|
| `establishing` | 建立镜头广角，交代空间全貌 | ✅ 主视图 |
| `key_angle` | 主机位角度（分镜里用得最多的那个） | ✅ |
| `reverse_angle` | 反打角度（保证轴线两侧都成立） | ✅ |
| `detail` | 材质/纹理/标志性局部 | ✅ |
| `lighting_variants` | 剧本要求的时间光线变体（日/黄昏/夜） | 跨时间段时必需 |
| `empty_plate` | 空景（无人物），便于人物合成与对位 | 有人物入场时建议 |

场景卡同样要求**同一空间、同一布景陈设**，变的只有机位和光线。

#### 道具卡（`prop_card`）

正面、侧面、使用状态三视图 + 比例参照（与手或人体的相对大小）。

#### 身份锁与锚点扩散

一致性不靠「每次都描述得很详细」，靠**同一个字符串被逐字复用**：

- 每张卡有一个 `identity_prompt`：一次写定的外观锁（发型、脸型、肤色、瞳色、
  服装、体型、年龄段、标志性特征），此后所有视图、所有镜头提示词**逐字复制**，
  不复述、不改写、不「优化措辞」。
- 生成顺序是**锚点扩散**，不是并行独立生成：
  1. 先出主视图（角色 `front_full`、场景 `establishing`）；
  2. 主视图作为**参考图**输入，生成其余视图——
     OpenAI 走 `images.edit`（最多 16 张参考），Z-Image 走 Edit 变体；
  3. 任一视图与主视图明显不符 → 重生成该视图，而不是接受漂移。
- 这条顺序是硬要求：并行生成八个视图，出来的是八个长得像但不是同一个人的角色。

#### 画幅：卡片不是成片帧

OpenAI 只有三档尺寸，**出不了 9:16 的 1080×1920**。这不影响卡片用途——
卡片是参考素材，1024×1536（2:3）完全够用。

但**首帧图**不一样：要喂给 i2v 当第一帧，就必须是成片画幅，否则模型要么拉伸要么裁切。
所以分开处理：

| 用途 | 画幅要求 | 建议来源 |
|---|---|---|
| 角色卡 / 场景卡 / 道具卡 | 自由（参考用途） | 图像后端（A 或 B） |
| 每镜首帧图（i2v） | **必须等于成片画幅** | Z-Image 按成片画幅出图，或用视频系列 t2v 出片抽帧 |
| 参考图（r2v） | 自由 | 直接复用角色卡/场景卡 |

也就是说：即便用了 OpenAI 做卡片，首帧图这条路仍然可能要走 ComfyUI。
这不是缺陷，是两种资产的用途本来就不同。

### 6.4 分层与新 crate

按 CLAUDE.md 的分层契约，新增一个瘦客户端 crate，与 `studio-comfy` 平级：

```
studio-openai    OpenAI Images HTTP 客户端。★ 纯协议，零业务逻辑，
                 与 studio-comfy 同构：生成、编辑、下载、错误映射。
                 不依赖 engine / store。

studio-pipeline  新增 visual_assets 执行器与后端抽象：
                 trait ImageBackend { generate, edit }
                 ├── OpenAiBackend  (studio-openai)
                 └── ZImageBackend  (studio-comfy + z_image 基线)
                 依赖 core + engine + comfy + media + openai。
```

依赖方向不变（仍然只向下），`studio-core` 不受影响，**不引入第二种运行时语言**
（Rust + ureq，与现有 HTTP 调用同一套）。

后端选择、锚点扩散顺序、重试与阻塞策略都在 `studio-pipeline` 里，
两个后端只负责「把提示词和参考图变成一张图」。

### 6.5 schema 改动：`asset_plan` 重做

现在的 `asset_plan.requests[]` 是一维的（一个 asset = 一个 prompt = 一张图），
装不下多视图。改成两级：

```jsonc
{
  "asset_plan": {
    "backend_policy": "openai_preferred_zimage_fallback",  // 声明式，必填
    "backend": "openai:gpt-image-2",        // 控制面回填：实际用的后端
    "core_model_family": "minimax_h3",
    "consistency_lock": { /* 角色/机位/环境/安全/排版 */ },
    "assets": [
      {
        "asset_id": "C01",
        "asset_kind": "character_card",
        "identity_prompt": "……一次写定的外观锁，后续逐字复用……",
        "applies_to": ["sh01", "sh03", "sh05"],
        "views": [
          {
            "view": "front_full",          // enum，见 §6.3
            "is_anchor": true,             // 主视图，先生成
            "prompt": "……",
            "derived_from": null,          // 非主视图必须指向锚点
            "status": "ready",             // planned/generating/ready/failed
            "path": "media/assets/C01/front_full.png",   // 控制面回填，相对路径
            "provenance": {                // 控制面回填，可审计
              "backend": "openai:gpt-image-2",
              "size": "1024x1536",
              "seed": null,                // OpenAI 无种子；Z-Image 有则记
              "references": []
            }
          }
        ]
      }
    ]
  }
}
```

要点：

- `views[].view` 是**枚举**（§6.3 的两张表），schema 层保证「不许只出一张大头照」——
  角色卡缺 `front_full`/`three_quarter`/`profile`/`back`/`face_close` 任一即 `schema_violation`。
- `identity_prompt` 提到卡片级，不再散落在每个 view 里，从结构上杜绝逐视图改写。
- `derived_from` 强制表达锚点扩散关系：非锚点视图必须指向一个锚点视图。
- `path` / `provenance` / `status` 由**控制面回填**，Agent 提交时不填——
  这是 Hybrid 阶段的分工：Agent 给意图，控制面给事实。
- bundle 内一律相对路径（硬规则 4），资产落
  `media/assets/<asset_id>/<view>.<ext>`。

### 6.6 卡片怎么进入渲染

**不解决这一条，前面全是白做**——§2.4 断层 3 的另一半：所有视频基线都没有图片输入绑定。

1. **上传**：ComfyUI 走 `/upload/image` 把卡片送上节点，拿到服务端文件名；
   多节点池要按节点分别上传（现在渲染是多节点并发的，卡片必须在每个用到的节点上都存在）。
2. **绑定**：给 `minimax_h3/i2v` 补首帧（可选尾帧）绑定、给 `minimax_h3/r2v` 补参考图绑定，
   基线里加 `references` / `first_frame` / `last_frame` 参数。
3. **核验**：按现有规矩，`bindings_verified` 只能在**真机跑通后**置 true。
   开发环境验不了这一步（没有 GPU、没有 ComfyUI），只能在生产机上做。
4. **选型**：优先 r2v（全程锚点）而不是 i2v（只喂第一帧，往后放身份会漂）——
   这是调研里最明确的一条一致性结论，要写进 prompt skill 的 doctrine。

### 6.7 技能与文档要跟着改

| 对象 | 改动 |
|---|---|
| `visual` skill | 从「写一份计划」改为「定身份锁 + 视图清单 + 逐视图提示词」；补多视图规格、锚点扩散顺序、两个后端的写法差异；说明门后面是**图**不是 JSON |
| 新 doctrine `consistency/character-sheet.md` | 多视图规格全表、中性光/中性底的理由、identity_prompt 的写法与反例 |
| 新能力卡 `models/openai_image.md` | 三档尺寸、16 张参考图、无 negative（约束写正向）、内容政策边界（真人肖像/名人）、按张计费 |
| 新能力卡 `models/z_image.md` | 尺寸自由、Turbo 8 步、Edit 变体做参考编辑、有 negative、本地免费 |
| `prompt` skill | 增加「优先 r2v、其次 i2v」的选型规则与 asset_id 引用方式 |
| doctor | 增加图像后端体检：有无 key、探活结果、z_image 基线在不在、将要使用哪个后端。**不回显 key** |
| `.env.example` | 增加 §6.2 的四个变量与说明 |
| `config/models.toml` | 增加 `[z_image]` 契约段（模型文件名、禁用变体） |
| 新基线 | `assets/workflows/z_image/t2i.json`、`z_image/edit.json`，初始 `bindings_verified: false` |

### 6.8 错误与阻塞

新增错误码 `image_backend_unavailable`，remedy 必须给出可执行下一步（硬规则 3）：

| 情形 | 行为 |
|---|---|
| 没有 key，z_image 基线可用 | 静默走兜底（这是**正常路径**，不是错误），backend 字段如实记录 |
| 没有 key，z_image 基线缺失/未核验 | `image_backend_unavailable`，remedy：配置 `OPENAI_API_KEY`，或补齐 z_image 基线 |
| 有 key 但探活失败（401/网络） | 不静默换后端——报错并说明是 key 无效还是网络不通，让用户决定 |
| 单张视图生成失败 | 重试（沿用渲染的重试策略）；到顶后该 view 标 `failed`，整阶段阻塞 |
| 部分视图 ready、部分 failed | **不放行**。缺视图的卡片锁不住身份，等于没有 |

第三行是有意为之：配了 key 说明用户想用 OpenAI，静默降级会让人误以为用的是 OpenAI 的画风。
**兜底是「没配置」的策略，不是「配置错了」的策略。**

---

## 7. 落地路线

分三批，每批自成闭环、可独立验收。

### 批次 1：提示词层（不动状态机，风险最低）

1. 建 `crates/studio-cli/assets/doctrine/**`，写方法层文档与黄金样例。
2. `emit-assets` 增加 doctrine 与模型能力卡的生成；`--check` 覆盖新文件。
3. SKILL.md 模板重写：路由 + 硬规则 + 自检清单 + doctrine 索引。
4. schema 加枚举、`three_facts`、`shot_function`、`audio` 等字段。
5. 泄漏测试扩展到全部随包文档。

**验收**：同一个 brief 跑两遍（重构前后），比较分镜里可执行物理事实的条数、
运镜是否落在词表内、禁用词密度。这一批 Codex 本机就能验（render 之前的六个阶段）。

### 批次 2：能力面对齐（消灭静默丢弃）

6. `prompt_pack` 的双向能力面校验 + 动态 schema。
7. 给 `minimax_h3/i2v`、`r2v` 补图片输入绑定，并真机核验
   （`bindings_verified` 只能在真机跑通后置 true——这是现有规矩，不放宽）。
8. `wan2_2/i2v`、`flf2v` 的未核验绑定要么补齐要么从可选列表里摘掉。

**验收**：单元测试覆盖「写了不支持的参数会报错」；真机 smoke 验证参考图确实进图。
**注意**：第 7、8 项必须在装有 ComfyUI 的生产机上验，开发环境验不了——
这条不能含糊，`scripts/smoke.sh` 是唯一真信号。

### 批次 3：视觉资产生成（§6，画面质量的主要抓手）

9. 新 crate `studio-openai`（Images 生成/编辑/下载），`studio-pipeline` 里
   `trait ImageBackend` + 两个实现 + 后端选择规则。
10. `is_studio_key()` 放行 `OPENAI_` 前缀；补密钥不外泄的测试（doctor 输出、
    trace、debug 请求体、stages 产物里都搜不到 key）。
11. `asset_plan` schema 重做成两级（identity + views + provenance），视图枚举必填校验。
12. `visual_assets` 执行器：锚点扩散顺序、逐视图重试、落盘 `media/assets/`、
    产物登记、门改为**看图确认**。
13. `z_image` 基线（t2i / edit）+ `config/models.toml` 的 `[z_image]` 段 +
    doctor 的图像后端体检 + `.env.example`。
14. 新错误码 `image_backend_unavailable` 与它的 remedy（穷尽 match，硬规则 3）。

**验收**：后端选择、锚点扩散、视图完整性校验、密钥不外泄都能在开发环境单测；
OpenAI 那条路只要有 key 就能在开发机真跑（不需要 GPU），
**Z-Image 那条路需要 ComfyUI，只能在生产机验**。

### 批次 4：流程改动（价值高，改动最大）

15. `idea` 多候选 + `selection` 真取舍。
16. 首帧图控制点（依赖批次 2 第 7 项与批次 3）。
17. 决定档案（revise 记忆）。
18. `review` 拆技术验收 / 内容验收。

第 15、17 项要动状态机与存储，需要各自的 ADR。

### 批次之间的依赖

```
批次 1（提示词层）        独立，可单独上线
批次 2（能力面对齐）      独立，可单独上线
批次 3（视觉资产生成）    第 14 项之外都独立；卡片要真正生效依赖批次 2 第 7 项
批次 4（流程改动）        依赖批次 2 + 3
```

**最短见效路径是批次 1 + 批次 3**：前者让 Agent 知道什么算好，后者让角色不再每镜换脸。
批次 2 是把「写了没用」变成报错，属于止血，但不直接提升画面。

---

## 8. 验收：怎么证明「不干巴了」

主观评价靠不住，定几条可量化的：

| 指标 | 现状（基线待测） | 目标 |
|---|---|---|
| 分镜每镜物理事实条数 | 约 0–1（`subject`+`action_chain` 各一句） | ≥ 3，且可指认 |
| 运镜落在受控词表内的比例 | 无词表，不可测 | 100% |
| 禁用词密度（Tier 1） | 待测 | 0 |
| 角色外观串跨镜一致性 | 靠复述，字符串不同 | 逐字相同（可用字符串比对自动测） |
| 提示词参数被静默丢弃的条数 | 默认路径上 negative + references 全丢 | 0（改为报错） |
| 音频描述覆盖率 | 0 | 每镜都有声音锚点 |
| 角色卡视图完整率 | **0（一张都没生成过）** | 必需视图 100% ready |
| 身份锁跨视图/跨镜逐字一致 | 无此概念 | 100%（字符串比对可自动测） |
| 卡片真正进入渲染请求的比例 | 0 | 100%（可在 debug 请求体里验证） |

前四项与后三项都可以做成 `studio-cli` 的本地检查进 CI；
「参数被静默丢弃」一项由能力面校验保证。

**必须说明的边界**：以上全部指标衡量的是**提示词的质量**，不是**画面的质量**。
开发环境没有 GPU、没有 ComfyUI，Codex 端到端只能验到 `prompt_pack`
（见 CLAUDE.md「Codex 验收的真实覆盖范围」）。画面到底变没变好，
只能在装了 ComfyUI 的生产机上跑 `scripts/smoke.sh` 对比。
本说明书不承诺、也无法在开发环境证明画面质量的改善。

---

## 9. 取舍与风险

| 风险 | 判断 |
|---|---|
| **上下文膨胀**：doctrine 加起来可能上万 token | 用渐进披露解决：SKILL.md ≤ 120 行，doctrine 按需读，模型卡只加载当前系列那一份。前身项目把 30 行 safetensors 文件名塞进每个会话（design.md §11）的教训在这里同样适用——**默认不加载**是原则 |
| **散文进仓库与「Markdown 不手写」冲突** | 见 §4.2：以 `include_str!` 源文件形式存在，生成器仍是唯一出口，`--check` 照常。精神不变 |
| **枚举收紧会让老产物失效** | schema 都是 `additionalProperties: true`，且修订路径本来就允许重提。收紧的是新提交 |
| **doctrine 泄漏二进制名 / 指向源码** | 现有四个泄漏测试扩展到全体随包文档，CI 挡 |
| **能力面校验会让原本"能跑"的提交失败** | 这是有意的：现在的「能跑」是静默丢参数换来的假象 |
| **方法层会不会让 Agent 变啰嗦** | 用禁用词表和自检清单反向压制；rubric 里明确「具体性」而不是「丰富性」 |
| **批次 4 改状态机的成本** | 拆成独立 ADR，不和批次 1、2、3 绑定 |
| **引入 OpenAI 依赖**：联网、计费、内容政策（真人肖像、名人、未成年人题材会被拒） | 这是 §6.2 兜底存在的理由：Z-Image 全本地、Apache-2.0、无政策拒绝。被拒时按 §6.8 报错并说明原因，**不静默换后端**（换了画风就变了） |
| **两个后端画风不一致** | 后端整片一次性选定，不逐张回退（§6.2）。这是硬约束，不是优化项 |
| **API key 泄漏进产物/日志** | §6.2 的密钥安全条款 + 一条专门的搜索测试。debug 请求体落盘这条路尤其要盯 |
| **多节点上传成本**：渲染是多节点并发的，卡片要在每个节点上都存在 | 上传按节点做、可缓存（同一文件同一节点只传一次）。这是实现细节，但会影响首镜延迟 |
| **卡片生成把 `visual_assets` 从秒级变成分钟级** | 这道门本来就该花时间——它现在秒过是因为什么都没做。进度用现有 `note` 机制回报（「C01 3/5 视图」） |

---

## 10. 一句话总结

现在的提示词系统**把 Agent 当成需要防范的执行者**，所以只写了契约；
它需要同时**把 Agent 当成需要培训的创作者**，补上词表、方法、范例和模型适配，
并且把「写了但不生效」的断层堵死。

而在这些之上还有一条更硬的：**角色卡和场景卡从来没有被生成过**——
再好的提示词也救不了每镜换脸的片子。所以这次调整的两个重心是
**给 Agent 方法**（§4、§5）和**给管线补上视觉资产这一环**（§6），
两者缺一，产出都好不了。
