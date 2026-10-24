# Astra 吉祥物（黑猫）素材与切图流水线

> 运行时组件：`src/src/components/agent/Mascot.tsx` + `Mascot.scss`
> 运行时素材：`src/public/mascot/actions/*.png`（打包进前端）
> 本目录：**只放源图与一次性处理脚本**，不参与构建。

---

## 目录说明

```
design/mascot/
├── source/                     # 生图产物（源图，体积大，不进前端包）
│   ├── Astra吉祥物动作总表.png      # 5 行 x 9 格动作总表，切图流水线的输入
│   ├── Astria黑猫品牌设计展示板.png # 品牌设计展示板（角色锚定参考）
│   ├── 黑猫吉祥物动作表情参考图.png  # 与动作总表同文件（md5 一致，可删）
│   ├── mascot_preview.png          # 近期拼图预览
│   └── crawl-preview.png           # 爬行动作预览（由 mascot_crawl.py 生成）
└── scripts/                    # 一次性 Python 处理脚本（依赖 Pillow）
    ├── mascot_actions_cut.py   # 【主脚本】动作总表 → 42 张透明 PNG + index.json
    ├── mascot_prompts.md       # 生图提示词包（角色锚定 / 表情 / 动作 / 多格模板）
    ├── mascot_split.py         # 从品牌展示板裁切单只猫并去背
    ├── mascot_crawl.py         # 2x2 爬行动作图 → 4 帧
    ├── mascot_batch.py         # Downloads 里 2x2 生图批量处理成动画帧
    ├── mascot_more.py          # 由静图程序化生成 bounce / jump / walk 帧
    ├── mascot_grid.py          # 多图拼成 contact sheet（通用水印版）
    ├── mascot_check_assets.py  # 校验 Mascot.tsx 引用路径是否都有对应文件（缺图时 exit 1）
    └── mascot_debug*.py / mascot_inspect.py  # 连通域/布局诊断，排查切图问题时用
```

所有脚本都以自身位置推算仓库根（`parents[3]`），可直接在任意目录下用
`python3 design/mascot/scripts/<script>.py` 运行，路径不再硬编码。

---

## 素材清单

`src/public/mascot/actions/` 共 42 张，命名分五类（前缀即用途）：

| 前缀 | 含义 | 数量 |
| --- | --- | --- |
| `basic-` | 基础姿态：坐/站/走/跑/伸展/趴下/甩尾 | 7 |
| `expr-` | 表情：开心/眨眼/专注/惊讶/疑问/生气/委屈/睡觉/思考 | 9 |
| `inter-` | 互动：打招呼/招手/点头/摇头/歪头/蹲下/趴着/跳跃/玩耍 | 9 |
| `work-` | 工作：看电脑/打字/看书/用工具/完成任务/背包/搜索/灵感 | 8 |
| `state-` | 状态：加载/等待/网络异常/崩溃/试飞/提示/离开/透明/消失 | 9 |

`actions/index.json` 记录每张图的来源行列、bbox 与尺寸，改切图逻辑时对应用。

当前前端用了其中 31 张，未接入的 11 张（`basic-stand` / `basic-walk` / `basic-run` /
`basic-stretch` / `basic-lie-down` / `basic-tail-swish` / `inter-nod` / `inter-crouch` /
`inter-lie-flat` / `state-fade` / `state-vanish`）作为储备素材保留在库里。

---

## 重新生成素材

```bash
# 依赖
python3 -m pip install pillow

# 动作总表 → 42 张切图（覆盖 src/public/mascot/actions/）
python3 design/mascot/scripts/mascot_actions_cut.py

# 校验前端引用的图片是否都存在（改过姿态 / 切图后跑）
python3 design/mascot/scripts/mascot_check_assets.py
```

切图逻辑要点（改之前先看 `mascot_actions_cut.py` 顶部注释）：
alpha 连通域 → 合并被割裂的部件（脚部光带、侧边道具）→ 丢弃小残留（标签残字）
→ 按 cy 聚行 → 行内按 x 排序 → 按语义表命名。

---

## 前端状态映射

姿态由 `Mascot.tsx` 推导，优先级（高 → 低）：

1. `error` → `state-network-error`
2. 双击跳跃 / 摸头动画 → `inter-jump`
3. `running` 时按最新条目：等确认 → `expr-question`；
   工具类按 `TOOL_POSE` 映射（写文件 → 打字，搜索 → 搜索，读文件 → 看书，
   命令 → 用工具，记忆 → 背包…），未命中走关键词兜底，再兜底 `focused`
4. 刚完成 4 秒 → 按回复关键词选图（完成/搜索/提问/思考/出错/睡觉/再见）
5. 悬停 → 开心；空闲 >90s → 打瞌睡；默认 → 坐着

活跃期还会在语义相近的图之间轮换（`VARIATIONS`，每 2.5s），避免长时间卡在同一张图。

---

## 新增姿态的流程

1. 写提示词（参考 `scripts/mascot_prompts.md` 的「角色锚定」段，图生图模式效果最好）
2. 生图 → 放进 `source/`
3. 若生图是 2x2 多格，用 `mascot_crawl.py` / `mascot_batch.py` 拆帧
4. 补进 `mascot_actions_cut.py` 的 `ROWS` 语义表（重跑切图），或直接把单图放进 `actions/`
5. 在 `Mascot.tsx` 的 `Pose` 类型、`IMG`、`TIP`（必要时加 `VARIATIONS` / `TOOL_POSE`）里登记
6. 跑 `pnpm -C src typecheck` 确认无类型遗漏（`Record<Pose, ...>` 会强制补全）
