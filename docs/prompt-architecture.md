# 提示词架构重构说明书

> 在线版（含排版）：https://claude.ai/code/artifact/f33fc1fd-1738-4ea3-89ec-bd66f7e8ec1d
>
> 本文是仓库内的权威版本。与在线版有出入时以本文为准。
>
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
> §6 单开一章处理它：ComfyUI + Z-Image 做角色卡与场景卡（文生图出主视图、
> 参考图生图出其余视图），多视图规格，以及一条不可降级的一致性硬要求。
> OpenAI 那条路评估后不采用，理由与实测记在 §6.2.2。

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
| [OpenAI gpt-image-2 文档](https://developers.openai.com/api/docs/models/gpt-image-2)、[images.edit 参考](https://developers.openai.com/api/reference/resources/images/methods/edit)、[图像生成指南](https://developers.openai.com/api/docs/guides/image-generation) | 编辑接口最多接受 **16 张参考图**且自动高保真保留细节；尺寸 1024×1024 / 1024×1536 / 1536×1024 / auto；quality low/med/high；n ≤ 10；png/webp/jpeg；支持透明背景；**没有 negative_prompt**。⚠ 这些是 **Platform API** 的能力；本项目实际用的 sub2api 链路只吃其中一部分，以 §6.2.1 的实测为准 |
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

**已实现。** 提交 `prompt_pack` 时，控制面按每个 shot 的 `workflow` 拿到基线的
`_studio.bindings`，做**双向对账**：

- 写了基线不支持的参数（如 `minimax_h3` 的 `negative`）→ `schema_violation`，
  错误消息直接给替代写法（「把约束改写成正向提示词里的完整句子」）；
- 基线要求但没写的参数（如 `ltx2_5` 的 `duration_seconds`）→ 同样报错，
  说明「不写就用基线默认值，结果不受你控制」；
- 用了未核验的基线 → 当场挡下，并带上未核验的具体原因和可用的替代。

`studio.schema("prompt_pack")` 的返回也**按这台机器动态收窄**：`workflow`
字段的取值只列已核验的基线。与其让 Agent 写完一整包提示词、提交时才被告知
基线没核验，不如在它看 schema 的那一刻就只给能用的那几条。

分层上，判断逻辑在 `studio-core`（纯数据，零 I/O，可在没有 GPU 的机器上完整
单测），基线读取在 `studio-pipeline`，两者通过 `StageExecutor::capabilities()`
连起来。核心层因此不必知道基线文件长什么样。

#### 收敛点：`references` 不按「写了会被丢弃」处理

实现时发现的一处冲突：黄金样例里五镜都写了 `references`，而当前没有任何基线
绑定它——按上面的规则，样例自己就不合规了。

停下来想清楚之后的判断是：**这两类参数的性质不同**。

- 写 `negative` 是**想控制渲染参数**，被丢弃等于控制失效 → 必须当场报错。
- 写 `references` 是**声明「这一镜用到哪些资产」**，即使当前进不了渲染请求，
  这个声明本身可审计，而且基线补上图片输入绑定之后会自动生效。

所以 `references` 的规则是**允许提前写，但基线一旦支持就必须写**。
后半句不需要额外代码——「少写」那个方向遍历的是基线实际绑定的键，
基线一旦绑了 `references`，没写就会报错。

这个区分不是为了让样例过关而开的口子：判据是「写它的意图是控制参数，
还是声明关联」，其它参数照此归类即可。

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
2. **控制面生成**：主视图先出，其余视图以主视图为参考图生成，落盘并登记产物。
3. **用户看图确认**：`visual_assets.approval` 这道门从「看计划」变成「**看图**」——
   门后面挂的是实际生成的卡片，不是一段 JSON。

第 3 点是这道门存在的意义。现在用户在这道门上确认的是一份自己无法判断真伪的计划，
确认了等于没确认。

而贯穿三步的第一原则是 **§6.2.1 的一致性硬要求：达不到就阻塞，不降级、不妥协。**

### 6.2 后端：ComfyUI 是唯一路径

图像后端只有一条：**ComfyUI + [Z-Image](https://docs.comfy.org/tutorials/image/z-image/z-image-turbo)**
（通义实验室 6B、Apache-2.0），同时提供两种生成方式，两种都必须支持：

| 方式 | 用途 | 实现 |
|---|---|---|
| 文生图 | 出主视图（角色 `front_full`、场景 `establishing`） | Z-Image Turbo，8 NFE、亚秒级 |
| 参考图生图 | 出其余所有视图，以主视图为锚 | Z-Image-Edit，指令编辑 |

走的是与视频渲染同一条 ComfyUI HTTP 通道，尺寸自由、有负向提示词、本机免费。

OpenAI 那条路评估过，**不采用**，两条独立的理由记在 §6.2.2。

#### 6.2.1 一致性是硬要求

**这是这个阶段的第一原则，不可降级、不可妥协。**

- 同一个角色在所有视图、所有镜头里必须是**同一个人**——脸、体型、服装细节，
  不是「看起来像」。场景卡同理：同一个空间的不同机位必须是同一个空间。
- 达不到就**结构化阻塞**，不允许退而求其次出一批「差不多」的卡片。
  一批锁不住身份的卡片比没有卡片更糟——它会让下游以为一致性已经解决了，
  而问题会推迟到渲染完成、花完 GPU 时间之后才暴露。
- 手段是**参考图锚定**：主视图先出，其余视图一律以主视图为参考图生成
  （§6.3）。**纯文字锚定不算达标**，§6.2.2 有实测证据。
- 这条原则约束的是控制面，不是 Agent：Agent 写不出合格的身份锁是内容问题，
  走 `studio.revise`；控制面拿不到参考图通道是能力问题，走结构化阻塞。

#### 6.2.2 为什么不用 OpenAI 那条路

**理由一：链路能力面残缺。**
用的 key 不是 Platform API key（直连官方端点报 `invalid_api_key`），
而是 **sub2api** 转出来的订阅制凭证。请求被包装成 Responses API 的
`image_generation` 工具调用——`n` 参数报的 `Unknown parameter: 'tools[0].n'`
是直接证据——并被静默改路由到 `gpt-image-2-codex`。这与
[openai/codex#28723](https://github.com/openai/codex/issues/28723) 记录的现象一致。

实测矩阵（全部走文生图）：

| 参数 | 行为 |
|---|---|
| `model` | 只有 `gpt-image-2` 是入口；写 `gpt-image-2-codex` 或 `gpt-image-1` 都 404。响应回报的 `model` 恒为 `gpt-image-2-codex` |
| `size` | **完全失效**。三个官方档、`auto`、省略、越界值、非法比例，连 `"banana"` 都返回 200 且尺寸相同——字段在到达模型前就被丢弃，不做任何校验 |
| `quality` | **完全失效**，恒为 `auto` |
| `output_format` | 有效：png / jpeg / webp |
| `background` | 被校验：`transparent` → 400 `Transparent background is not supported for this model.` |
| `n` | **不支持**：400 `Unknown parameter: 'tools[0].n'` |

其中最要命的一条不在表里：**参考图传不上去**。这条网络路径上，超过约
5–14 KB 的 multipart 上传会被站点的 Cloudflare 拦成 403 HTML，而任何真实
参考图都远超这个体量。没有参考图通道，就没有锚定手段。

（画幅倒是可控——`size` 无效，但 auto 模式会按提示词里的构图描述定画幅，
写明比例就很准：9:16 → 941×1672，16:9 → 1672×941，1:1 → 1254×1254。
这条经验在 Z-Image 上用不着，但记在这里，将来若接入 Platform API 会用到。）

**理由二：纯文字锚定实测锁不住脸。**
用与黄金样例同等密度的身份锁，四个视图**逐字复用、一个字不改**：

> 20岁东亚女性，长黑发及胸、中分、发梢微卷，圆脸，浅褐色眼睛，左耳一枚细银色小圆环耳钉，白色无袖连衣裙，低帮白色板鞋，奶油色小斜挎包

结果分层：

| 层次 | 结果 |
|---|---|
| 服装大类、发型、配饰 | ✅ 锁住了 |
| 服装细节 | ⚠️ 漂移：腰线拼接 A 字裙 → 抽褶腰；方形翻盖包 → 半圆马鞍包 |
| **脸** | ❌ **锁不住**：正面、四分之三、面部特写是三张不同的脸 |

原因是结构性的，不是提示词写得不够好：**文字能穷尽的是可枚举的外部特征
（穿什么、什么发型、戴什么），穷尽不了脸。**「圆脸、浅褐色眼睛」的解空间太大。
把身份锁写得再长也改变不了这一点——能靠加字解决的，早就被加字解决了。

**当前定位：不可用的备选，设计保留。**

- **不可用**：在拿到能传参考图的图像通道之前，这条路做不了多视图卡片，
  不作为路径、不实现、不在 `backend` 的合法取值里。
- **设计保留**：`ImageBackend` 抽象保留，两个方法（文生图、参考图生图）的
  签名按能接入它的样子设计，不为了「现在只有一个实现」而把接口拍平成具体类型。
- **后续高优先级补回**：网关的图片通道问题解决后（换 Platform API key，
  或换一条不被 Cloudflare 拦 multipart 的通道），这块升为高优先级——
  它带来的是「不需要 GPU 也能做视觉资产」，这个价值在 §6.2.3 里丢掉了。
  重新接入时先重跑 §6.2.2 的两项验证：能力面矩阵 + 四视图一致性实测，
  两项都过才允许作为路径。

真要接入时，密钥安全那套要求照旧：key 只在内存里传给 HTTP 客户端，
不进任何产物、日志、trace、debug 请求体，对外只记后端标识，
并加一条「塞假 key 跑一遍，输出与落盘文件里都搜不到它」的测试。

#### 6.2.3 代价：视觉资产绑定 GPU

OpenAI 那条路唯一的好处是不需要 GPU。放弃它，等于把视觉资产阶段
**完全绑定到装有 ComfyUI 的机器**：

- 开发环境（无 GPU、无 ComfyUI）做不完这个阶段，只能做到 Agent 提交
  `asset_plan` 为止；卡片本身的验收只能在生产机。
- 没有健康的 ComfyUI 节点时，这个阶段以 `comfy_unavailable` 结构化阻塞，
  **不降级、不跳过、不用纯文字凑合出图**——这与 §6.2.1 是同一条原则。

这是明知的取舍：**一致性优先于「到处都能跑」**。

#### 与「不自动降级」硬规则的关系

现有规矩「核心系列不可用就结构化阻塞，不自动换系列」原样适用，而且现在更强了：
图像后端只有一条路，没有可换的对象。`fallback_policy` 字段保留，取值固定为
「阻塞，不降级」——它存在的意义从「声明选哪个兜底」变成「声明这里没有兜底」。

#### 环境变量

图像后端沿用视频那批 ComfyUI 节点，不引入新的连接配置：

```
COMFY_NODES            图像与视频共用；没有健康节点时视觉资产阶段直接阻塞
Z_IMAGE_WORKFLOW       可选，覆盖默认的 z_image 基线名
```

`OPENAI_*` 一族**当前用不到**。将来接入 Platform API 时才需要，
届时记得 `is_studio_key()` 的白名单要放行 `OPENAI_` 前缀——
现在它只放行 `FFMPEG_PATH` / `FFPROBE_PATH` / `COMFY_*` / `CORE_MODEL_FAMILY`，
进程环境里的 `OPENAI_API_KEY` 会被直接丢弃。

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

#### 身份锁与生成顺序

一致性不靠「每次都描述得很详细」，靠**同一个字符串被逐字复用**：

- 每张卡有一个 `identity_prompt`：一次写定的外观锁（发型、脸型、肤色、瞳色、
  服装、体型、年龄段、标志性特征），此后所有视图、所有镜头提示词**逐字复制**，
  不复述、不改写、不「优化措辞」。
- 生成顺序按后端分两种：

  1. **先出主视图**，单独出：角色是 `front_full`，场景是 `establishing`。
  2. **其余视图一律以主视图为参考图生成**，走 Z-Image-Edit：
     提示词 = 逐字相同的 `identity_prompt` + 该视图特有的机位/表情描述，
     参考图 = 主视图。文字负责说清「要什么角度」，参考图负责保证「是同一个人」。
  3. 任一视图与主视图明显不符 → 重生成该视图，而不是接受漂移。
     重生成到顶仍不达标 → 整阶段阻塞（§6.2.1），不放行。

- 主视图**先出、单独出**是硬要求：并行生成八个视图，出来的是八个长得像
  但不是同一个人的角色。这一条与后端无关，是生成模型的固有性质。
- **文字锚定不能替代参考图锚定**。身份锁仍然要逐字复用——它管住服装、
  发型、配饰这些可枚举特征，也是提示词进视频渲染时唯一的抓手——
  但它管不住脸（§6.2.2 有实测），所以卡片这一环必须有图。

#### 画幅：卡片不是成片帧

Z-Image 走 ComfyUI，画幅按参数设，要多少是多少。**卡片和首帧图的画幅要求
本来就不同**，分开处理：

| 用途 | 画幅 | 说明 |
|---|---|---|
| 角色卡 | 竖构图，如 768×1344 | 全身入画不裁切 |
| 场景卡 | 横构图或与成片同画幅 | |
| 道具卡 | 方构图 | 带比例参照 |
| 每镜首帧图（i2v） | **必须等于成片画幅** | 9:16 就出 1080×1920，不靠拉伸 |

**画幅是「一张卡一套」，不是「一个视图一套」**：同一张卡的所有视图用同一个
`aspect`，混用会被 schema 挡下——一张竖一张方，看的人会以为是不同批次
生成的。所以角色卡的 `face_close` 也走这张卡的竖构图，不单独换成方的。

（实现时收敛的一处：初稿把角色卡按「全身竖、面部方」分成两行，与
「同一张卡一套规格」自相矛盾。取后者，因为它可机械校验，而且
面部特写在竖画幅里完全成立。）

### 6.4 分层

**不新增 crate。** 图像后端走的是已有的 `studio-comfy` 通道，
`studio-openai` 当前不建——它只在 §6.2.2 那条路被启用时才需要。

```
studio-pipeline  新增 visual_assets 执行器与后端抽象：

                 trait ImageBackend {
                     fn generate(&self, req) -> Result<Image>;      // 文生图
                     fn edit(&self, req, refs: &[Image]) -> Result<Image>;  // 参考图生图
                 }
                 └── ZImageBackend  (studio-comfy + z_image 基线)

                 依赖 core + engine + comfy + media。
```

抽象保留两个方法，是为了 §6.2.2 那条路将来能接进来——**不要因为现在只有
一个实现就把接口拍平成具体类型**。但也不要为不存在的第二个实现做过度设计：
选择逻辑、重试、阻塞策略都在 `studio-pipeline` 里，后端只负责「把提示词
（可选加参考图）变成一张图」。

依赖方向不变（仍然只向下），`studio-core` 不受影响，**不引入第二种运行时语言**。

### 6.5 schema 改动：`asset_plan` 重做

现在的 `asset_plan.requests[]` 是一维的（一个 asset = 一个 prompt = 一张图），
装不下多视图。改成两级：

```jsonc
{
  "asset_plan": {
    "backend_policy": "block_no_fallback",  // 没有兜底：拿不到一致性就阻塞
    "backend": "comfyui:z_image",           // 控制面回填：当前唯一合法取值
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
            "aspect": "9:16",              // 目标比例，必须显式写进 prompt
            "prompt": "……身份锁逐字 + 本视图机位 + 画幅比例……",
            "derived_from": null,          // 非主视图必须指向锚点视图，
                                           // 控制面照它取参考图（§6.3）
            "status": "ready",             // planned/generating/ready/failed
            "path": "media/assets/C01/front_full.png",   // 控制面回填，相对路径
            "provenance": {                // 控制面回填，可审计
              "backend": "comfyui:z_image",
              "workflow": "z_image/edit",  // 实际用的基线
              "size": "768x1344",
              "seed": 40201,               // 固定并记录，卡片也要可复现
              "references": ["media/assets/C01/front_full.png"]  // 锚点，非主视图必有
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
- `aspect` 必填：同一张卡的所有视图按同一套尺寸规格出，不能一张竖一张方。
- `derived_from` 是**一致性的执行依据，不只是元数据**：非锚点视图必须指向一个
  锚点视图，控制面照着它去取参考图喂给 Z-Image-Edit。没写就没有锚点可用，
  是 `schema_violation`。
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
| `visual` skill | 从「写一份计划」改为「定身份锁 + 视图清单 + 逐视图提示词」；补多视图规格、主视图先行的生成顺序、参考图锚定；说明门后面是**图**不是 JSON |
| 新 doctrine `consistency/character-sheet.md` | 多视图规格全表、中性光/中性底的理由、`identity_prompt` 的写法与反例，以及**为什么文字锁不住脸**（§6.2.2 的实测，写给 Agent 看，免得它以为把身份锁写长就够了） |
| 新能力卡 `models/z_image.md` | 尺寸自由、Turbo 8 步做文生图、Edit 变体做参考图生图、有 negative、本地免费；两种模式各自的写法 |
| `prompt` skill | 增加「优先 r2v、其次 i2v」的选型规则与 asset_id 引用方式 |
| doctor | 增加视觉资产体检：z_image 的两份基线在不在、核验没核验、ComfyUI 节点是否健康 |
| `.env.example` | 增加 `Z_IMAGE_WORKFLOW`；不加 `OPENAI_*`（§6.2.2 当前不启用） |
| `config/models.toml` | 增加 `[z_image]` 契约段（模型文件名、禁用变体） |
| 新基线 | `assets/workflows/z_image/t2i.json`、`z_image/edit.json`，初始 `bindings_verified: false` |

### 6.8 错误与阻塞

图像后端只有一条路，所以错误处理比原方案简单，但**阻塞条件更严**——
一致性达不到就不放行（§6.2.1）。

| 情形 | 行为 |
|---|---|
| 没有健康的 ComfyUI 节点 | `comfy_unavailable` 结构化阻塞。**不降级、不跳过、不用纯文字凑合出图** |
| `z_image` 基线缺失或未核验 | `model_contract_violation`，remedy 指向补齐并真机核验基线 |
| 单张视图生成失败 | 重试（沿用渲染的重试策略）；到顶后该 view 标 `failed`，整阶段阻塞 |
| 部分视图 ready、部分 failed | **不放行**。缺视图的卡片锁不住身份，等于没有 |
| 非主视图缺 `derived_from` | `schema_violation`——没有锚点就没法做参考图锚定 |
| 生成出来与主视图明显不符 | 重生成该视图；到顶仍不符则整阶段阻塞，交给人判断 |

**不新增错误码。** 原方案里的 `image_backend_unavailable` 是为「两个后端都不可用」
设计的；现在只有一条路，`comfy_unavailable` 与 `model_contract_violation` 已经够用，
语义也更准。将来 §6.2.2 那条路启用时再看要不要加。

---

## 7. 落地路线

分三批，每批自成闭环、可独立验收。

### 批次 1：提示词层（不动状态机，风险最低）

**已完成。**

1. 建 `crates/studio-cli/assets/doctrine/**`，写方法层文档与黄金样例。
2. `emit-assets` 增加 doctrine 与模型能力卡的生成；`--check` 覆盖新文件。
3. SKILL.md 模板重写：路由 + 硬规则 + 自检清单 + doctrine 索引。
4. schema 加枚举、`three_facts`、`shot_function`、`audio` 等字段。
5. 泄漏测试扩展到全部随包文档。

**验收**：同一个 brief 跑两遍（重构前后），比较分镜里可执行物理事实的条数、
运镜是否落在词表内、禁用词密度。这一批 Codex 本机就能验（render 之前的六个阶段）。

### 批次 2：能力面对齐（消灭静默丢弃）

6. `prompt_pack` 的双向能力面校验 + 动态 schema。**已完成**——见 §5.1，
   判断逻辑在 `studio-core`（零 I/O、可单测），基线读取在 `studio-pipeline`。
7. 给 `minimax_h3/i2v`、`r2v` 补图片输入绑定，并真机核验
   （`bindings_verified` 只能在真机跑通后置 true——这是现有规矩，不放宽）。
   **这一项是批次 3 的硬前置**：卡片做出来进不了渲染就还是纸面计划。
8. `wan2_2/i2v`、`flf2v` 的未核验绑定要么补齐要么从可选列表里摘掉。
   **「摘掉」这半边已完成**：未核验的基线不出现在 `studio.schema` 的
   可选 workflow 里，写了也会在提交时被挡下。补齐绑定那半边需要真机核验。

**验收**：单元测试覆盖「写了不支持的参数会报错」；真机 smoke 验证参考图确实进图。
**注意**：第 7、8 项必须在装有 ComfyUI 的生产机上验，开发环境验不了——
这条不能含糊，`scripts/smoke.sh` 是唯一真信号。

### 批次 3：视觉资产生成（§6，画面质量的主要抓手）

**前置**：第 7 项（给视频基线补图片输入绑定）必须先做完。卡片做出来却进不了
渲染，就还是纸面计划——§2.4 断层 3 的另一半。原方案把它排在批次 2，
现在它是批次 3 的硬前置。

9. `studio-pipeline` 里 `trait ImageBackend`（文生图 + 参考图生图两个方法）
   与唯一实现 `ZImageBackend`。**不建 `studio-openai`**（§6.2.2 当前不启用）。
10. `z_image` 基线（`t2i` / `edit`）+ `config/models.toml` 的 `[z_image]` 段 +
    doctor 的视觉资产体检 + `.env.example` 的 `Z_IMAGE_WORKFLOW`。
11. `asset_plan` schema 重做成两级（identity + views + provenance），
    视图枚举必填校验，非主视图必须有 `derived_from`。**已完成**——
    视图词表、必需视图齐全、主视图唯一、`derived_from` 指向锚点、
    同卡画幅一致、角色卡身份锁逐字包含，六条都在 `schema::validate` 里挡下。
    配套的 `consistency/character-sheet.md`、`visual` skill、doctor 的
    卡片基线体检、`config/models.toml` 的 `[z_image]` 段也都在了。
    **`z_image` 的两条基线本身没有**：开发环境没有 GPU 也没有 ComfyUI，
    出不了真机导出，`assets/workflows/z_image/README.md` 写的是导出要求
    而不是节点图——一份看起来像模像样的节点图能通过所有静态检查，
    然后在生产机上安静地画错东西。
12. `visual_assets` 执行器：主视图先行、其余视图参考图锚定、逐视图重试、
    落盘 `media/assets/`、产物登记、门改为**看图确认**。
13. 参考图上传到 ComfyUI 节点（`/upload/image`），按节点缓存。
14. 一致性不达标时的阻塞路径：视图缺失、`derived_from` 缺失、重生成到顶
    仍不符——三种都不放行（§6.8）。

**验收**：schema 校验、生成顺序、视图完整性、阻塞条件都能在开发环境单测。
**但卡片本身的效果只能在生产机验**——ComfyUI 需要 GPU，开发环境做不完这个阶段
（§6.2.3）。这一批不存在「在开发机上真跑一遍」的选项，别把单测通过当成效果验证。

### 批次 4：流程改动（价值高，改动最大）

15. `idea` 多候选 + `selection` 真取舍。**已完成**——`concepts[]`（≥2 且互斥）、
    `candidates[]` 逐个评估、选题门变成真三选一，用户选中的选项记进
    `_gate_choice` 带给下游。
16. 首帧图控制点（依赖批次 2 第 7 项与批次 3）。**未做，需要 GPU。**
17. 决定档案（revise 记忆）。**已完成**——见
    [ADR-0003](decisions/ADR-0003-decision-archive.md)：revise 的原话与门上
    的选择进 `decisions` 表，`next_action.decisions` 回最近 20 条，追加即历史。
18. `review` 拆技术验收 / 内容验收。**已完成**——`checks[].kind` 分
    technical / content，新工具 `studio.self_review` 收固定五维 rubric 的
    自评，每条要带可指认的时间点；不改 `passed`，但不交就不算收尾。

第 17 项动了状态存储，有独立 ADR；第 15、18 项没动状态机，
只加了信封字段和一个工具，不需要 ADR。

### 批次之外：质量闸

说明书写作时没有单列，实现时补的一环（§2.3 说「没有质量闸，只有格式闸」，
但落地路线里没有对应条目）。

`studio-core::quality` 七条机械可判的规则：禁用词（面向模型的文本挡提交、
上游只提醒）、物理事实太短、身份锁没逐字出现、身份锁跨阶段漂移、
提示词过短、没译出运镜指令。加 `studio-cli quality` 对整部作品复查，
有挡提交的条目就非零退出，可以进 CI。

提交闸只在提交那一刻跑，已经躺在库里的产物没人回头看——命令补的是那一半。

### 批次之间的依赖

```
批次 1（提示词层）        独立，可单独上线
批次 2（能力面对齐）      独立，可单独上线
                          └─ 第 7 项（图片输入绑定）是批次 3 的硬前置
批次 3（视觉资产生成）    依赖批次 2 第 7 项；全程需要 GPU
批次 4（流程改动）        依赖批次 2 + 3
```

**画面质量的完整链条是「批次 2 第 7 项 → 批次 3」**：先让基线吃得下图片，
再把卡片做出来。少了前一半，卡片进不了渲染；少了后一半，绑定了也没图可喂。
批次 1 已经上线，它让 Agent 知道什么算好，但治不了每镜换脸——
那是图片通道的事，不是提示词的事（§6.2.2 的实测说明了这一点）。

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
| 非主视图带 `derived_from` 的比例 | 无此概念 | 100%（没有锚点就没法参考图锚定） |
| **跨视图是不是同一个人** | **不是**（§6.2.2 实测：三张不同的脸） | **是**——这条不可降级（§6.2.1） |
| 卡片真正进入渲染请求的比例 | 0 | 100%（可在 debug 请求体里验证） |

前四项与后三项都可以做成 `studio-cli` 的本地检查进 CI；
「参数被静默丢弃」一项由能力面校验保证。

其中「跨视图是不是同一个人」目前**没有自动判据**——字符串比对管不了脸。
它靠两道人工关口：`visual_assets.approval` 那道门（用户看图确认）和
生产机上的 smoke。在有可靠的自动判据之前，这条指标由人来判，
但它的地位不因此降低——它是 §6.2.1 那条硬要求的唯一验收方式。

**必须说明的边界**：以上其余指标衡量的是**提示词的质量**，不是**画面的质量**。
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
| **视觉资产从此绑定 GPU**：放弃了唯一一条不需要 GPU 的图像路径 | 明知的取舍，一致性优先于「到处都能跑」（§6.2.3）。开发环境只能做到提交 asset_plan 为止，卡片验收只能在生产机 |
| **一致性硬要求会让阶段更容易阻塞**：达不到就不放行 | 这是有意的。一批锁不住身份的卡片比没有卡片更糟——它让下游以为一致性已解决，问题推迟到烧完 GPU 之后才暴露 |
| **多节点上传成本**：渲染是多节点并发的，卡片要在每个节点上都存在 | 上传按节点做、可缓存（同一文件同一节点只传一次）。这是实现细节，但会影响首镜延迟。生成卡片时同样要上传参考图，这条路现在用得更频繁了 |
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
