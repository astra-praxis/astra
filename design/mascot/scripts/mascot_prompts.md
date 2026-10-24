# Astra 黑猫吉祥物 · 生图提示词包

> 目标：用已有设计图生成更多表情 / 姿势 / 场景图，保持风格统一，用于前端平面交互。
> 用法：**图生图模式** 效果最好（上传现有 PNG 作参考图 + 用下面的提示词）；文生图模式直接贴提示词。

---

## 一、角色锚定提示词（风格锚点，必用）

### 中文（适用于即梦 / 通义万相 / 豆包 / 可灵等）
```
Q版吉祥物形象，一只纯黑短毛猫，圆滚滚的卡通坐姿造型，头大身体小（Q版/二头身）。
毛色为深炭黑（#0a0a0d），材质有柔和的丝绸光泽。
戴着一条深蓝灰色（#10101a）的V形领巾/围脖，领巾垂片盖在胸口。
尾巴从左后方高高举起，尾巴尖端有一颗白色四角闪星；围巾上也有一颗较小的白色四角闪星。
眼睛非常大（竖椭圆），白色眼白 + 大黑色瞳孔 + 两点白色高光，表情可爱有神。
两只耳朵又大又立，耳朵内侧是浅灰粉。
脸上有几根细白胡须。
整体风格：扁平化插画 / 游戏吉祥物，纯色背景，无描边，无文字。
```

### English（适用于国际平台 / SD / 官方 Qwen-Image）
```
A cute Q-version chibi mascot: a round black short-haired cat with a big head and small body, sitting pose.
Fur is deep charcoal black (#0a0a0d) with soft silky sheen.
Wearing a dark slate-blue (#10101a) V-shaped bandana/scarf whose flap covers the chest.
Tail raised high on the left side, tipped with a white 4-point sparkle star; a smaller white 4-point star on the scarf.
Very large tall oval eyes: white sclera + big black pupils + two white highlight dots, adorable expression.
Big upright ears with light gray-pink inner ears. A few thin white whiskers.
Style: flat illustration, game mascot, solid background, no outline, no text, clean vector-like.
```

---

## 二、表情变体（图生图：换脸，保持身体/构图不变）

| 用途 | 中文提示词 |
|---|---|
| 好奇（默认） | 同上锚定，表情：歪头好奇地睁大眼睛，微微张嘴 |
| 开心 | 同上锚定，表情：眯眼笑，嘴巴弯成开心的弧度 |
| 专注（工作中） | 同上锚定，表情：认真专注，眉毛微微收紧，眼神盯着前方 |
| 惊讶（报错） | 同上锚定，表情：双眼圆睁瞪大，瞳孔缩小，嘴巴张成O形 |
| 睡觉（空闲） | 同上锚定，表情：闭着眼睛打瞌睡，头微微低垂，旁边飘着"Zzz" |

English 版：
```
(curious) same base + tilting head, wide curious eyes, slightly open mouth
(happy)   same base + closed happy eyes, smiling curved mouth
(focused) same base + serious focused eyes, slightly furrowed brows
(surprised) same base + huge wide eyes, small pupils, O-shaped mouth
(sleeping) same base + closed eyes dozing, head drooping, "Zzz" floating nearby
```

---

## 三、场景 / 动作变体

| 场景 | 中文提示词 |
|---|---|
| 探索（坐姿+背包） | 同锚定，但：背上背着一个深灰色小背包，坐在地上好奇地望向远方，背景简洁渐变 |
| 执行（走路） | 同锚定，但：侧身行走姿势，一条腿抬起迈步，尾巴翘得更高，动感 |
| 成长（背影） | 同锚定，但：背对镜头看向远方，只看到后脑勺和尾巴，尾巴星朝上，意境感 |
| 待机浮动 | 同锚定，但：身体微微悬浮/跳跃，脚离地一点，轻盈感 |
| 鼓掌/庆祝 | 同锚定，但：开心地举起两只前爪，星星在周围闪烁 |

English 版：
```
(explore) + wearing a small dark-gray backpack, sitting and looking into the distance
(execute) + side walking pose, one leg lifted mid-step, tail raised higher
(grow)    + back view facing away, looking at the horizon, tail star up, atmospheric
(bounce)  + slightly floating / hopping, feet just off the ground
(celebrate) + raising both front paws happily, stars sparkling around
```

---

## 四、多格变体（一张图里放多套）

> 两种方式：
> - **推荐：单张单变体生成 + `mascot_grid.py` 拼图**（可控性最高，避免模型把多格画乱）
> - 或者：直接在提示词里要求网格布局（下面给模板）

### 多格提示词模板（中文）

**表情套（5 格）**
```
一张图分成 2 行 3 列共 6 个格子（最后一格空白）。
每个格子里是同一个吉祥物角色（纯黑短毛猫、Q版大头、深蓝灰 V 领巾、白色四角星），但表情不同：
第一行：好奇（歪头睁大眼）｜开心（眯眼笑）｜专注（认真盯前方）
第二行：惊讶（圆眼张嘴）｜睡觉（闭眼打瞌睡）｜空白
每个格子独立完整构图、背景纯色统一、角色大小一致、无文字、无边框重叠。
```

**场景套（3 格）**
```
一张图分成 1 行 3 列 3 个格子。
每个格子是同一个吉祥物角色（纯黑短毛猫、Q版大头、深蓝灰 V 领巾、白色四角星）：
左格：探索（背深灰背包坐地望远方）｜中格：执行（侧身行走迈步）｜右格：成长（背对镜头望远方）
每个格子独立完整构图、背景纯色统一、角色大小一致、无文字。
```

### 多格生成注意事项

1. 多格图**角色一致性最容易被破坏**：模型可能画出不同大小的猫、格子间串色、表情雷同。生成后务必检查每个格子。
2. 如果平台支持**参考图**，务必上传原始设计图，并强调“所有格子使用同一个角色”。
3. 多格图生成失败时，退回方案 A：单张生成 + `mascot_grid.py` 拼图。

---

## 五、拼图脚本用法

```sh
# 拼多张变体为一张多格图（默认 3 列、白底、每格高 512px）
python3 mascot_grid.py img1.png img2.png img3.png --out grid.png

# 把一个目录下所有 PNG 按文件名顺序拼起来
python3 mascot_grid.py ./new_images --cols 4 --cell-h 512 --bg white --out grid.png

# 透明底输出（适合后续做素材）
python3 mascot_grid.py ./new_images --cols 4 --bg transparent --out grid.png
```

---

## 六、使用建议

1. **图生图优先**：上传现有 PNG（如 `main.png` / `expr-*.png`）做参考图，提示词写"同锚定 + 目标变化"，风格一致性最高。
2. **负面提示词**（如果有这栏）：
   ```
   文字, 水印, 描边, 线条, 多个动物, 写实, 恐怖, 畸形, 多余肢体, 多余的尾巴, 背景杂乱
   ```
3. **尺寸**：生成 1:1 或 3:4；交互用的透明图后续可用 PIL 去背（复用 `mascot_split.py` 的去背逻辑）。
4. **种子固定**：同一表情多生成几张挑最像的，锁定种子后微调提示词，变体更可控。
5. **"Uncensored" 模型注意**：质量/风格可能飘，生成后务必人工筛选，保证与原始设计一致（黑猫+蓝灰领巾+白星三要素不能丢）。
