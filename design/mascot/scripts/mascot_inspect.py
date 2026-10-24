"""检查总表图的模式与透明通道结构"""
from PIL import Image
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SRC = REPO_ROOT / "design" / "mascot" / "source" / "Astra吉祥物动作总表.png"
im = Image.open(SRC)
print("mode:", im.mode, "size:", im.size)
im = im.convert("RGBA")
W, H = im.size
a = im.split()[3]
hist = a.histogram()
total = W * H
fully = hist[0]
print(f"完全透明(alpha=0): {fully} ({fully/total:.1%})")
print(f"完全不透明(alpha=255): {hist[255]} ({hist[255]/total:.1%})")
semi = total - fully - hist[255]
print(f"半透明: {semi} ({semi/total:.1%})")
# 半透明的 alpha 分布
import collections
dist = collections.Counter()
ap = a.load()
for y in range(0, H, 2):
    for x in range(0, W, 2):
        v = ap[x, y]
        if 0 < v < 255:
            dist[v // 16] += 1
print("半透明alpha桶分布(每桶16):", sorted(dist.items()))
