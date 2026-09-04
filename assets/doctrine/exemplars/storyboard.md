# 样例：分镜

一部十秒五镜的旅行 vlog。下面是完整产物，然后是逐条批注——
**看批注比看 JSON 有用**，因为要学的是判断，不是字段。

```json
{
  "storyboard": {
    "aspect_ratio": "9:16",
    "character_lock": {
      "camera_signature": "主要使用约30度侧脸，面部可读",
      "safety": "不靠近水边危险边缘，不翻越护栏",
      "subject": "20岁女性，长黑发",
      "wardrobe": "白裙、低帮板鞋、奶油色小斜挎包"
    },
    "shot_count": 5,
    "shots": [
      {
        "action_chain": "船头切开水面 -> 她转头 -> 笑容展开",
        "angle": "low",
        "audio": {
          "ambient": "湖水拍打船身的持续哗声，低频风声",
          "foley": "碎发拂过脸颊、衣料轻响",
          "music": "none"
        },
        "background": "层叠群岛与远山",
        "camera_motion": "push_in",
        "color_tone": "上午冷白，低对比",
        "duration_seconds": 1.4,
        "end": 1.4,
        "first_frame": "湖面与船头",
        "foreground": "船头栏杆",
        "last_frame": "侧脸笑容定格",
        "lighting_key": "soft",
        "lighting_source": "daylight",
        "midground": "清透湖面",
        "purpose": "地点钩子：0.6 秒内认出千岛湖，同时给出一张会笑的脸",
        "shot_function": "advance_action",
        "shot_id": "sh01",
        "shot_size": "wide",
        "sound": "湖水拍打船身",
        "start": 0,
        "subject": "女孩以约30度侧脸入画",
        "three_facts": [
          "船行带起的风把碎发吹到她嘴角",
          "她抬手把碎发别到耳后，指尖在耳廓停住",
          "船头切开水面的持续哗声"
        ],
        "transition_to_next": "以水声作 J-cut"
      },
      {
        "action_chain": "起步 -> 小跑两步 -> 裙摆扬到最高点",
        "angle": "eye_level",
        "audio": {
          "ambient": "开阔湖面的风声",
          "foley": "板鞋踩木板的两声闷响、裙摆抖动",
          "music": "none"
        },
        "background": "湖面与远岛",
        "camera_motion": "tracking",
        "color_tone": "顺光明亮，低对比",
        "duration_seconds": 2.0,
        "end": 3.4,
        "first_frame": "脚步落下",
        "foreground": "步道栏杆虚化",
        "last_frame": "裙摆最高点",
        "lighting_key": "soft",
        "lighting_source": "daylight",
        "midground": "女孩全身",
        "purpose": "交代人物状态：她在这里是自在的",
        "shot_function": "advance_action",
        "shot_id": "sh02",
        "shot_size": "medium",
        "sound": "脚步与风声",
        "start": 1.4,
        "subject": "女孩沿木质步道小跑",
        "three_facts": [
          "湖风从左侧推来，裙摆和发梢一起向右扬",
          "落地时脚踝先内扣再蹬直，重心前倾半步",
          "板鞋鞋底拍在木板上的两声闷响"
        ],
        "transition_to_next": "顺动作切"
      },
      {
        "action_chain": "举起手机 -> 镜头随视线摇向群岛 -> 按下快门",
        "angle": "eye_level",
        "audio": {
          "ambient": "观景台上的风与零星人声",
          "foley": "手机快门声",
          "music": "none"
        },
        "background": "湖中群岛",
        "camera_motion": "pan_right",
        "color_tone": "正午偏暖，高对比",
        "duration_seconds": 2.4,
        "end": 5.8,
        "first_frame": "抬手",
        "foreground": "手机边框",
        "last_frame": "群岛全景",
        "lighting_key": "hard",
        "lighting_source": "daylight",
        "midground": "女孩侧脸",
        "purpose": "把视线从人交给景，全片信息量最大的一镜",
        "shot_function": "change_emotion",
        "shot_id": "sh03",
        "shot_size": "medium_close",
        "sound": "快门声",
        "start": 3.4,
        "subject": "女孩举起手机取景",
        "three_facts": [
          "正午的光很硬，手机屏幕反着湖面的白",
          "她眯了一下眼，拇指在快门键上停了半秒",
          "快门声，随后是远处游船的汽笛"
        ],
        "transition_to_next": "快门声作硬切"
      },
      {
        "action_chain": "举杯 -> 轻碰 -> 笑出声",
        "angle": "eye_level",
        "audio": {
          "ambient": "湖风与远处水声",
          "foley": "玻璃杯轻碰的脆响、短促笑声",
          "music": "none"
        },
        "background": "湖岛虚化",
        "camera_motion": "static",
        "color_tone": "下午暖金",
        "duration_seconds": 2.0,
        "end": 7.8,
        "first_frame": "两杯靠近",
        "foreground": "画外伸入的另一只杯子",
        "last_frame": "笑容与杯壁水珠",
        "lighting_key": "side",
        "lighting_source": "daylight",
        "midground": "女孩胸像",
        "purpose": "把风景兑现成可分享的快乐：有人跟她在一起",
        "shot_function": "change_emotion",
        "shot_id": "sh04",
        "shot_size": "close",
        "sound": "碰杯声与笑声",
        "start": 5.8,
        "subject": "女孩举起冷饮与画外的手碰杯",
        "three_facts": [
          "杯壁的冷凝水滑到虎口",
          "指节收紧握住杯身，笑的时候肩膀轻轻一耸",
          "两只玻璃杯轻碰的一声脆响"
        ],
        "transition_to_next": "笑声延续到下一镜"
      },
      {
        "action_chain": "转身 -> 回头 -> 挥手",
        "angle": "eye_level",
        "audio": {
          "ambient": "傍晚的风与远处水浪低频",
          "foley": "衣料摩擦、手臂划过空气",
          "music": "none"
        },
        "background": "湖面与远山",
        "camera_motion": "pedestal_up",
        "color_tone": "夕阳暖金，逆光",
        "duration_seconds": 2.2,
        "end": 10.0,
        "first_frame": "背影",
        "foreground": "草叶",
        "last_frame": "挥手定格，天空留白",
        "lighting_key": "back",
        "lighting_source": "daylight",
        "midground": "女孩全身",
        "purpose": "收在一个能停住的画面上，给天空留白",
        "shot_function": "change_emotion",
        "shot_id": "sh05",
        "shot_size": "medium_wide",
        "sound": "挥手拟音与环境尾音",
        "start": 7.8,
        "subject": "女孩回头挥手",
        "three_facts": [
          "逆光把发丝勾出暖金边，风把裙摆吹成一道弧",
          "她转身时先动眼神再动脖子，抬手挥到肩高",
          "衣料摩擦声与远处水浪的低频"
        ],
        "transition_to_next": "淡出黑场"
      }
    ],
    "timing_basis": "时长由动作完成度和信息密度决定，不平均切分",
    "title": "千岛湖，把快乐装进十秒",
    "total_duration_seconds": 10
  }
}
```

## 为什么这样写

**时长不是平均的：1.4 / 2.0 / 2.4 / 2.0 / 2.2。**
第三镜最长，因为它要完成"抬手 → 摇镜 → 按快门"三个阶段；
第一镜最短，因为它只需要一个转头。`timing_basis` 里写的是依据，
不是把结果重复一遍。

**每一镜的 `shot_function` 都不同。**
前两镜 `advance_action`（把故事往前推），后三镜里有两镜 `change_emotion`
（改变观众的感受）。如果五镜全是同一个职能，说明这五镜在重复做一件事。

**`three_facts` 三条各司其职。** 以第二镜为例：

- 环境压力：「湖风从左侧推来，裙摆和发梢一起向右扬」——
  给了画面动势，也告诉模型什么该动、往哪动
- 身体微动作：「落地时脚踝先内扣再蹬直，重心前倾半步」——
  这是"小跑"这个动作的具体形态，不写就会得到一个飘着走的人
- 声音锚点：「板鞋鞋底拍在木板上的两声闷响」——
  具体到鞋和地面的材质，不是"脚步声"

对照着看第五镜的第一条：「逆光把发丝勾出暖金边，风把裙摆吹成一道弧」。
它同时交代了光和运动——一条物理事实可以承担两件事，但不能一件都不承担。

**运镜每镜只有一个，且都在词表里。**
`push_in` / `tracking` / `pan_right` / `static` / `pedestal_up`。
注意第四镜是 `static`：碰杯这个动作本身有内容，镜头再动就是干扰。
**固定机位是默认，不是保守。**

**光拆成三个字段。** 第三镜是 `daylight` + `hard` + 「正午偏暖，高对比」，
第五镜是 `daylight` + `back` + 「夕阳暖金，逆光」——同样是日光，
光型和色调把两镜的时间感彻底分开了。如果都写成"自然光"，
模型会给你两个一样的中午。

**`audio` 每镜都填了。** 核心系列是音视频联合生成，留空等于放弃。
注意每镜的 `foley` 都是具体声源：碎发拂过脸颊、板鞋踩木板、
手机快门、玻璃杯轻碰、衣料摩擦。

**转场能被审计。** 第一镜「以水声作 J-cut」、第三镜「快门声作硬切」——
后期是照着这些字拼的，写"自然过渡"等于没写。

## 这份样例里没有的东西

没有一个「电影感」「唯美」「氛围感」。也没有一处写「她很开心」——
开心是通过"转头露出笑容""笑时肩膀轻轻一耸"拍出来的。
