"""诊断：看单猫小图的边缘值分布 + 整行活动度曲线形态"""
from PIL import Image, ImageFilter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SRC = REPO_ROOT / "design" / "mascot" / "source" / "Astra吉祥物动作总表.png"
im = Image.open(SRC).convert("RGB")
W, H = im.size
gray = im.convert("L")
edges = gray.filter(ImageFilter.GaussianBlur(1.2)).filter(ImageFilter.FIND_EDGES)
ep = edges.load()

# 1) 一个典型背景块的边缘值分布（row1 第1格左上角区域，应该是纯背景）
h1 = [0] * 16
for x in range(10, 60):
    for y in range(int(H/5*0)+10, int(H/5*0)+60):
        v = ep[x, y]
        h1[min(15, v // 8)] += 1
print("背景块(左上角)边缘值直方图(每桶8):", h1)

# 2) 整张图边缘值直方图
h2 = [0] * 16
epx = ep
for y in range(0, H, 3):
    for x in range(0, W, 3):
        v = epx[x, y]
        h2[min(15, v // 8)] += 1
print("整图抽样边缘值直方图(每桶8):", h2)

# 3) row1 活动度曲线采样打印（每 16px 一个值），观察峰在哪
y0, y1 = 0, int(H / 5)
act = []
for x in range(W):
    s = 0
    for y in range(y0 + 2, int(y0 + (y1 - y0) * 0.82), 2):
        s += ep[x, y]
    act.append(s)
print("row1 活动度 max=", max(act), "mean=", sum(act)/len(act))
print("曲线(每16px):")
print(" ".join(str(act[x]) for x in range(0, W, 16)))
