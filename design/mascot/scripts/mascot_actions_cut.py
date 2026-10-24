"""《Astra吉祥物动作总表》透明背景切图 v5（修正合并逻辑）。

流程：
1. alpha>=32 连通域
2. 全局合并断裂部件：y 相邻(重叠或间隙<=8) 且 (x 重叠 或 x 间隙<=12 且 组件小 n<2000)
   —— 脚部光带 / 侧边道具并入本体；相邻猫（都很大）不会误并
3. 丢弃小残留（n<600，标签残字等）
4. 按 cy 聚行（行间距 ~200 >> 行内变化 ~30）
5. 行内按 x 排序 → 语义命名切图
"""
from PIL import Image
from pathlib import Path
from collections import deque
import json

# 本文件位于 design/mascot/scripts/，向上三级即仓库根
REPO_ROOT = Path(__file__).resolve().parents[3]
SOURCE_DIR = REPO_ROOT / "design" / "mascot" / "source"

SRC = SOURCE_DIR / "Astra吉祥物动作总表.png"
OUT = REPO_ROOT / "src" / "public" / "mascot" / "actions"
OUT.mkdir(parents=True, exist_ok=True)

ROWS = [
    ("basic", [("sit", "坐着"), ("stand", "站立"), ("walk", "走路"), ("run", "奔跑"),
               ("stretch", "伸展"), ("lie-down", "趴下"), ("tail-swish", "甩尾巴")]),
    ("expr", [("happy", "开心"), ("wink", "眨眼"), ("focused", "专注"), ("surprised", "惊讶"),
              ("question", "疑问"), ("angry", "生气"), ("wronged", "委屈"), ("sleeping", "睡觉"),
              ("thinking", "思考")]),
    ("inter", [("wave", "打招呼"), ("beckon", "招手"), ("nod", "点头"), ("shake", "摇头"),
               ("tilt", "歪头"), ("crouch", "蹲下"), ("lie-flat", "趴着"), ("jump", "跳跃"),
               ("play", "玩耍")]),
    ("work", [("laptop", "看电脑"), ("typing", "打字"), ("reading", "看书"), ("tools", "使用工具"),
              ("done", "完成任务"), ("backpack", "背包"), ("search", "搜索"), ("idea", "灵感")]),
    ("state", [("loading", "加载中"), ("waiting", "等待"), ("network-error", "网络异常"),
               ("crash", "崩溃"), ("test", "试飞"), ("notice", "提示"), ("leave", "离开"),
               ("fade", "透明"), ("vanish", "消失")]),
]

im = Image.open(SRC).convert("RGBA")
W, H = im.size
ap = im.split()[3].load()

TH = 32
PAD = 4
X_GAP_MAX = 12
Y_GAP_MAX = 8
SMALL = 2000   # n < 2000 的组件才允许跨间隙合并（道具/脚带）
MIN_KEEP = 600  # 合并后 < 600 的丢弃（标签残字）

# ---------- 连通域 ----------
mask = bytearray(W * H)
for y in range(H):
    for x in range(W):
        if ap[x, y] >= TH:
            mask[y * W + x] = 1

comps = []
seen = bytearray(W * H)
for sy in range(H):
    for sx in range(W):
        i = sy * W + sx
        if not mask[i] or seen[i]:
            continue
        q = deque([(sx, sy)])
        seen[i] = 1
        n = 0
        x0 = y0 = 1 << 30
        x1 = y1 = -1
        while q:
            x, y = q.popleft()
            n += 1
            x0, x1 = min(x0, x), max(x1, x)
            y0, y1 = min(y0, y), max(y1, y)
            for nx, ny in ((x+1,y),(x-1,y),(x,y+1),(x,y-1)):
                if 0 <= nx < W and 0 <= ny < H and mask[ny*W+nx] and not seen[ny*W+nx]:
                    seen[ny*W+nx] = 1
                    q.append((nx, ny))
        if n >= 300:
            comps.append([n, x0, y0, x1, y1])

# ---------- 全局合并（断裂部件并入本体） ----------
comps.sort(key=lambda c: (c[2], c[1]))  # 按 y 排
merged = []
for c in comps:
    hit = None
    for m in merged:
        # m 与 c 是否同一猫（区间真正相交）
        y_overlap = c[2] <= m[4] and m[2] <= c[4]
        y_gap = c[2] - m[4] if c[2] > m[4] else m[2] - c[4]
        x_overlap = c[1] <= m[3] and m[1] <= c[3]
        x_gap = c[1] - m[3] if c[1] > m[3] else m[1] - c[3]
        same_cat = False
        if y_overlap:
            same_cat = x_overlap or (0 < x_gap <= X_GAP_MAX and min(c[0], m[0]) < SMALL)
        else:
            # 上下相邻（脚部光带）：x 需重叠或贴近，且其中一方较小
            if 0 < y_gap <= Y_GAP_MAX and (x_overlap or 0 < x_gap <= X_GAP_MAX):
                if min(c[0], m[0]) < SMALL:
                    same_cat = True
        if same_cat:
            hit = m
            break
    if hit:
        hit[0] += c[0]
        hit[1] = min(hit[1], c[1]); hit[2] = min(hit[2], c[2])
        hit[3] = max(hit[3], c[3]); hit[4] = max(hit[4], c[4])
    else:
        merged.append(c)

# ---------- 丢弃小残留 ----------
cats = [c for c in merged if c[0] >= MIN_KEEP]

# ---------- 聚行（行间距 >> 行内变化） ----------
cats.sort(key=lambda c: (c[2] + c[4]) / 2)
rows = []
cur = []
last_cy = None
for c in cats:
    cy = (c[2] + c[4]) / 2
    if last_cy is not None and cy - last_cy > 60:
        rows.append(cur)
        cur = []
    cur.append(c)
    last_cy = cy
if cur:
    rows.append(cur)

print(f"连通域 {len(comps)} → 合并后 {len(merged)} → 猫 {len(cats)}，聚成 {len(rows)} 行")

# ---------- 等距验证 ----------
print("\n=== 等距验证 ===")
if len(rows) != len(ROWS):
    print(f"⚠️ 检测到 {len(rows)} 行，预期 {len(ROWS)} 行")
for r, row in enumerate(rows):
    if r >= len(ROWS):
        break
    prefix, names = ROWS[r]
    row.sort(key=lambda c: c[1])
    cx = [(c[1] + c[3]) / 2 for c in row]
    gaps = [round(cx[i+1] - cx[i]) for i in range(len(cx) - 1)]
    ok = len(row) == len(names)
    print(f"[row{r+1}] {prefix}: {len(row)}只 中心={['%d' % c for c in cx]} 间距={gaps} {'✓' if ok else '✗ 列数不符!'}")

# ---------- 切图 ----------
print("\n=== 切图 ===")
index = {}
ok_all = True
for r, row in enumerate(rows):
    if r >= len(ROWS):
        break
    prefix, names = ROWS[r]
    row.sort(key=lambda c: c[1])
    if len(row) != len(names):
        print(f"[row{r+1}] {prefix}: 检测 {len(row)} 只 ≠ 命名表 {len(names)} 只，跳过")
        ok_all = False
        continue
    for i, (n, x0, y0, x1, y1) in enumerate(row):
        en, zh = names[i]
        bbox = (max(0, x0 - PAD), max(0, y0 - PAD), min(W, x1 + PAD), min(H, y1 + PAD))
        cell = im.crop(bbox)
        fname = f"{prefix}-{en}.png"
        cell.save(OUT / fname)
        index[f"{prefix}-{en}"] = {"zh": zh, "row": r + 1, "col": i + 1,
                                   "bbox": [x0, y0, x1, y1], "size": list(cell.size)}
        print(f"  {fname} ({zh}) {cell.size}")

json.dump(index, open(OUT / "index.json", "w"), ensure_ascii=False, indent=2)
print(f"\n映射表 -> {OUT/'index.json'}")
print("全部列数匹配 ✓" if ok_all else "\n⚠️ 部分行列数不匹配，需检查")
