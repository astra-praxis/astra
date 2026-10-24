"""批量处理 Downloads 里的“生成动作互动图”：切 2x2 四格 → 去背去阴影 → 底部对齐 → 输出帧 + 预览条。

输出结构:
  src/public/mascot/anim/sheet-{N}/frame-{1..4}.png   处理后的透明帧
  mascot/preview-sheet-{N}.png                        白底预览条（用户目检用）
"""
from PIL import Image, ImageFilter
from collections import deque
from pathlib import Path
import glob
import re

# 本文件位于 design/mascot/scripts/，向上三级即仓库根
REPO_ROOT = Path(__file__).resolve().parents[3]
SOURCE_DIR = REPO_ROOT / "design" / "mascot" / "source"

SRC_DIR = str(Path.home() / "Downloads")  # 生图产物默认落在这里，可用参数覆盖
OUT_ROOT = REPO_ROOT / "src" / "public" / "mascot" / "anim"
PREVIEW_DIR = SOURCE_DIR / "preview"
OUT_ROOT.mkdir(parents=True, exist_ok=True)
PREVIEW_DIR.mkdir(parents=True, exist_ok=True)


def remove_bg_and_shadow(im: Image.Image) -> Image.Image:
    """边缘泛洪去背：浅灰背景 + 柔和阴影（亮度>135）从边缘清除；
    黑猫本体保留，被身体包裹的浅色（眼白/星星/内耳）不受影响。"""
    im = im.convert("RGBA")
    px = im.load()
    w, h = im.size
    corners = [px[0, 0][:3], px[w-1, 0][:3], px[0, h-1][:3], px[w-1, h-1][:3]]
    bg = tuple(sorted(c[i] for c in corners)[1] for i in range(3))

    def near_bg(p):
        r, g, b = p[:3]
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


def process_sheet(path: Path, sheet_no: int):
    img = Image.open(path).convert("RGB")
    W, H = img.size
    quads = [
        img.crop((0, 0, W//2, H//2)),
        img.crop((W//2, 0, W, H//2)),
        img.crop((0, H//2, W//2, H)),
        img.crop((W//2, H//2, W, H)),
    ]
    frames = []
    for q in quads:
        f = remove_bg_and_shadow(q)
        bbox = f.getbbox()
        if bbox:
            f = f.crop(bbox)
        frames.append(f)

    PAD = 12
    cw = max(f.width for f in frames) + PAD * 2
    ch = max(f.height for f in frames) + PAD * 2

    out_dir = OUT_ROOT / f"sheet-{sheet_no:02d}"
    out_dir.mkdir(parents=True, exist_ok=True)

    canvas_frames = []
    for i, f in enumerate(frames):
        c = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
        c.paste(f, ((cw - f.width) // 2, ch - PAD - f.height), f)
        c.save(out_dir / f"frame-{i+1}.png")
        canvas_frames.append(c)

    preview = Image.new("RGBA", (cw * 4 + 50, ch + 20), (255, 255, 255, 255))
    for i, c in enumerate(canvas_frames):
        preview.paste(c, (10 + i * (cw + 10), 10), c)
    preview.convert("RGB").save(PREVIEW_DIR / f"preview-sheet-{sheet_no:02d}.png")
    print(f"sheet-{sheet_no:02d}: 4 frames ({cw}x{ch}) -> {out_dir}")


paths = sorted(glob.glob(f"{SRC_DIR}/生成动作互动图 (*).jpeg"),
               key=lambda p: int(re.search(r"\((\d+)\)", p).group(1)))
for p in paths:
    no = int(re.search(r"\((\d+)\)", p).group(1))
    try:
        process_sheet(Path(p), no)
    except Exception as e:
        print(f"sheet-{no:02d}: FAILED {e}")
print("batch done")
