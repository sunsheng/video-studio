# 样例：剧本

十秒五拍，无口播。

```json
{
  "script": {
    "audio_policy": {
      "external_music": "disabled",
      "fallback": "原生音频不可用则结构化阻塞",
      "primary": "核心模型原生音频"
    },
    "hook_at_seconds": 0.6,
    "language": "none",
    "safety_notes": [
      "步道动作保持低速，远离护栏和水边边缘"
    ],
    "segments": [
      {
        "end": 1.4,
        "segment_id": "s01",
        "source": "核心模型原生环境声",
        "speaker": "ambient",
        "start": 0,
        "subtitle_text": "",
        "text": ""
      },
      {
        "end": 3.4,
        "segment_id": "s02",
        "source": "核心模型原生环境声",
        "speaker": "ambient",
        "start": 1.4,
        "subtitle_text": "",
        "text": ""
      },
      {
        "end": 5.8,
        "segment_id": "s03",
        "source": "核心模型原生环境声",
        "speaker": "ambient",
        "start": 3.4,
        "subtitle_text": "",
        "text": ""
      },
      {
        "end": 7.8,
        "segment_id": "s04",
        "source": "核心模型原生环境声",
        "speaker": "ambient",
        "start": 5.8,
        "subtitle_text": "",
        "text": ""
      },
      {
        "end": 10.0,
        "segment_id": "s05",
        "source": "核心模型原生环境声",
        "speaker": "ambient",
        "start": 7.8,
        "subtitle_text": "",
        "text": ""
      }
    ],
    "shot_count": 5,
    "story_arc": [
      {
        "audio": "湖水拍打船身的持续哗声",
        "beat_id": "beat_01",
        "beat_type": "hook",
        "duration_seconds": 1.4,
        "end": 1.4,
        "purpose": "0.6 秒内让人认出这是千岛湖，并给出一张会笑的脸",
        "start": 0,
        "visual": "船头切开清透湖面，女孩以约30度侧脸快速入画并转头露出笑容"
      },
      {
        "audio": "板鞋落在木板上的两声闷响与风声",
        "beat_id": "beat_02",
        "beat_type": "setup",
        "duration_seconds": 2.0,
        "end": 3.4,
        "purpose": "交代人物状态：她在这里是自在的",
        "start": 1.4,
        "visual": "女孩沿湖边木质步道轻快小跑两步，白裙被风吹起"
      },
      {
        "audio": "快门声，随后远处游船汽笛",
        "beat_id": "beat_03",
        "beat_type": "develop",
        "duration_seconds": 2.4,
        "end": 5.8,
        "purpose": "把视线从人交给景，这是全片信息量最大的一拍",
        "start": 3.4,
        "visual": "她在观景台举起手机取景，镜头从30度侧脸摇向湖中群岛"
      },
      {
        "audio": "两只玻璃杯轻碰的脆响与短促笑声",
        "beat_id": "beat_04",
        "beat_type": "payoff",
        "duration_seconds": 2.0,
        "end": 7.8,
        "purpose": "把风景兑现成可分享的快乐：有人在跟她一起",
        "start": 5.8,
        "visual": "保持30度侧脸笑着举起冷饮，与画外另一只手轻碰杯"
      },
      {
        "audio": "衣料摩擦声与远处水浪低频，自然尾收",
        "beat_id": "beat_05",
        "beat_type": "resolve",
        "duration_seconds": 2.2,
        "end": 10.0,
        "purpose": "收在一个可以停住的画面上，留出天空的空白",
        "start": 7.8,
        "visual": "夕阳暖光下女孩回头挥手，白裙和长发被风带动"
      }
    ],
    "subtitle_policy": {
      "generated_from": [],
      "policy": "本版无口播、无字幕"
    },
    "timing_rule": "按动作复杂度和信息量分配时长；五段连续片段无重叠，精确合计10秒",
    "title": "千岛湖，把快乐装进十秒",
    "total_duration_seconds": 10
  }
}
```

## 为什么这样写

**`hook_at_seconds` 是 0.6，不是 1.4。**
第一拍长 1.4 秒，但钩子在 0.6 秒就成立了——观众那时已经知道
"这是湖上，有个女孩在笑"。剩下的 0.8 秒是让人确认自己看到了什么。
写成 1.4 就是把镜头长度当成了钩子时间。

**五拍的 `beat_type` 是一条完整的骨架：**
`hook` → `setup` → `develop` → `payoff` → `resolve`。
十秒装不下六种拍点，这五种是最常用的组合。没有 `turn`——
十秒的 vlog 不需要转折，硬塞一个反而会把节奏打散。

**`purpose` 写的是目的，不是内容。**
对比一下：

- 写成内容：「船头掠过湖面，女孩转头笑」——这是 `visual` 字段的事
- 写成目的：「0.6 秒内让人认出这是千岛湖，并给出一张会笑的脸」

后者能被验收：拍完之后可以问"0.6 秒认出来了吗"。
前者只是把画面又说了一遍。

**时长分配的依据写在 `timing_rule` 里。**
第三拍 2.4 秒最长，因为它信息量最大（要把视线从人交给景）；
第一拍 1.4 秒最短，因为只有一个转头。**不平均切**是这份剧本最重要的
一个判断——每镜 2 秒的版本会让观众明显感到幻灯片感。

**`segments` 与 `story_arc` 时间对齐。**
五段声音时间线的起止点与五拍完全一致：0–1.4、1.4–3.4、3.4–5.8、
5.8–7.8、7.8–10.0。总和精确等于 10 秒。**这是最常见的退回原因，
提交前自己加一遍。**

**无口播也要把声音写清楚。**
`language` 是 `none`，每段的 `text` 和 `subtitle_text` 是空串，
但 `source` 明确写了「核心模型原生环境声」，`audio_policy` 里写了
原生音频优先、外部音乐禁用、原生音频不可用时结构化阻塞。

「没有口播」不等于「没有声音」，更不等于这一栏可以不填。

**`subtitle_policy` 明确写了「本版无字幕」。**
留空会让后期不知道该不该生成字幕。没有也要说出来。
