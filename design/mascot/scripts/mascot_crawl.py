"""把 AI 生成的 2x2 爬行动作图切成 4 帧、去背、底部对齐，输出逐帧素材。

输入: Astra爬行动作.png（2x2 网格，浅灰背景带阴影）
输出: src/public/mascot/anim/crawl-{tl,tr,bl,br}.png + crawl-preview.png（白底拼条预览）
"""
from PIL import Image, ImageFilter
from collections import deque
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SOURCE_DIR = REPO_ROOT / "design" / "mascot" / "source"

SRC = SOURCE_DIR / "Astra爬行动作.png"
OUT = REPO_ROOT / "src" / "public" / "mascot" / "anim"
OUT.mkdir(parents=True, exist_ok=True)

img = Image.open(SRC).convert("RGB")
W, H = img.size
print(f"source: {W}x{H}")

# ===== 1) 切四格（2x2，中线切分；浅色分隔线会被去背吸收）=====
quads = {
    "tl": img.crop((0, 0, W // 2, H // 2)),
    "tr": img.crop((W // 2, 0, W, H // 2)),
    "bl": img.crop((0, H // 2, W // 2, H)),
    "br": img.crop((W // 2, H // 2, W, H)),
}


def remove_bg_and_shadow(im: Image.Image) -> Image.Image:
    """边缘泛洪去背：浅灰背景 + 柔和阴影（亮度较高）从边缘清除；
    黑猫本体（很暗）保留，眼白/星星/内耳等被身体包裹的浅色不受影响。"""
    im = im.convert("RGBA")
    px = im.load()
    w, h = im.size
    corners = [px[0, 0][:3], px[w-1, 0][:3], px[0, h-1][:3], px[w-1, h-1][:3]]
    bg = tuple(sorted(c[i] for c in corners)[1] for i in range(3))

    def near_bg(p):
        r, g, b = p[:3]
        # 距背景色近 → 背景；或整体偏亮（柔和阴影渐变，亮度>135）→ 阴影
        dist = abs(r-bg[0]) + abs(g-bg[1]) + abs(b-bg[2])
        bright = (r + g + b) / 3
        return dist < 100 or bright > 135

    seen = [[False]*w for _ in range(h)]
    q = deque()
    for x in range(w):
        q.append((x, 0)); q.append((x, h-1))
    for y in range(h):
        q.append((0, y)); q.append((w-1, y))
    while q:
        x, y = q.popleft()
        if x < 0 or y < 0 or x >= w or y >= h or seen[y][x]:
            continue
        seen[y][x] = True
        if not near_bg(px[x, y]):
            continue
        px[x, y] = (0, 0, 0, 0)
        q.extend(((x+1, y), (x-1, y), (x, y+1), (x, y-1)))

    a = im.getchannel("A").filter(ImageFilter.MinFilter(3)).filter(ImageFilter.GaussianBlur(0.8))
    im.putalpha(a)
    return im


frames = {}
for name, quad in quads.items():
    out = remove_bg_and_shadow(quad)
    bbox = out.getbbox()
    if bbox:
        out = out.crop(bbox)
    frames[name] = out
    print(f"{name}: trimmed {out.size}")

# ===== 2) 统一画布：底部中心对齐（脚踩地面），画布取各帧最大宽高 + 边距 =====
PAD = 12
cw = max(f.width for f in frames.values()) + PAD * 2
ch = max(f.height for f in frames.values()) + PAD * 2
print(f"canvas: {cw}x{ch}")

canvas_frames = {}
for name, f in frames.items():
    c = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
    c.paste(f, ((cw - f.width) // 2, ch - PAD - f.height), f)
    canvas_frames[name] = c
    c.save(OUT / f"crawl-{name}.png")
    print(f"crawl-{name}.png saved")

# ===== 3) 白底预览条（按建议循环顺序：tl → br → bl → tr抬头）=====
order = ["tl", "br", "bl", "tr"]
preview = Image.new("RGBA", (cw * 4 + 40, ch + 20), (255, 255, 255, 255))
for i, name in enumerate(order):
    preview.paste(canvas_frames[name], (10 + i * (cw + 5), 10), canvas_frames[name])
preview.convert("RGB").save(SOURCE_DIR / "crawl-preview.png")
print("preview ->", SOURCE_DIR / "crawl-preview.png")
