"""诊断：逐连通域 bbox + 每行 alpha 列剖面，搞清真实布局"""
from PIL import Image
from pathlib import Path
from collections import deque

REPO_ROOT = Path(__file__).resolve().parents[3]
SRC = REPO_ROOT / "design" / "mascot" / "source" / "Astra吉祥物动作总表.png"
im = Image.open(SRC).convert("RGBA")
W, H = im.size
ap = im.split()[3].load()

def comps_at(TH):
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
                comps.append((n, x0, y0, x1, y1))
    return comps

for TH in (16, 32, 64, 96):
    comps = comps_at(TH)
    print(f"\n===== TH={TH}: {len(comps)} 个组件 =====")
    comps.sort(key=lambda c: (c[2], c[1]))
    for n, x0, y0, x1, y1 in comps:
        print(f"  n={n:6d} bbox=({x0},{y0})-({x1},{y1})  cx={(x0+x1)//2} cy={(y0+y1)//2}")

# 列 alpha 剖面（每行带）
print("\n===== 列 alpha 剖面（每 8px 一个值，>=32 计数） =====")
band = H / 5
for r in range(5):
    y0, y1 = int(r*band), int((r+1)*band)
    prof = []
    for x in range(0, W, 8):
        c = 0
        for y in range(y0, y1, 2):
            if ap[x, y] >= 32:
                c += 1
        prof.append(c)
    m = max(prof)
    print(f"row{r+1} max={m}:")
    line = "".join("#" if v > m*0.35 else ("+" if v > m*0.12 else ('.' if v > 0 else ' ')) for v in prof)
    print("  " + line)
