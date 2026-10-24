"""从 Astra 黑猫品牌设计展示板裁切吉祥物并去除背景，输出透明 PNG。

用法: python3 mascot_split.py
背景是浅灰/白色，猫是纯黑 —— 用「边缘泛洪 + 颜色距离」去背，黑猫主体不会误删。
"""
from PIL import Image, ImageFilter, ImageDraw
from collections import deque
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SOURCE_DIR = REPO_ROOT / "design" / "mascot" / "source"

SRC = SOURCE_DIR / "Astra黑猫品牌设计展示板.png"
OUT = REPO_ROOT / "src" / "public" / "mascot"
OUT.mkdir(parents=True, exist_ok=True)

img = Image.open(SRC).convert("RGB")
W, H = img.size
print(f"source: {W}x{H}")


def remove_bg(im: Image.Image, tol: int = 38) -> Image.Image:
    """边缘泛洪去背：从四边向内扩散，颜色接近背景色的连续区域置透明。
    泛洪保证只删与边缘连通的背景，猫身上偶尔的浅色（眼白/围巾星）不受影响。"""
    im = im.convert("RGBA")
    px = im.load()
    w, h = im.size
    # 背景色取四角中值
    corners = [px[0, 0][:3], px[w - 1, 0][:3], px[0, h - 1][:3], px[w - 1, h - 1][:3]]
    bg = tuple(sorted(c[i] for c in corners)[1] for i in range(3))

    def near_bg(p):
        r, g, b = p[:3]
        return abs(r - bg[0]) + abs(g - bg[1]) + abs(b - bg[2]) < tol * 3

    seen = [[False] * w for _ in range(h)]
    q = deque()
    for x in range(w):
        q.append((x, 0))
        q.append((x, h - 1))
    for y in range(h):
        q.append((0, y))
        q.append((w - 1, y))
    while q:
        x, y = q.popleft()
        if x < 0 or y < 0 or x >= w or y >= h or seen[y][x]:
            continue
        seen[y][x] = True
        if not near_bg(px[x, y]):
            continue
        px[x, y] = (0, 0, 0, 0)
        q.extend(((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)))

    # 轻微收缩+羽化 alpha，去除白边
    a = im.getchannel("A").filter(ImageFilter.MinFilter(3)).filter(ImageFilter.GaussianBlur(0.8))
    im.putalpha(a)
    return im


def crop_frac(name: str, box_frac, tol=38, pad=6):
    """按比例裁切。box_frac=(x0,y0,x1,y1) 相对整板比例。自动 trim 透明边。"""
    x0, y0, x1, y1 = (int(v * (W if i % 2 == 0 else H)) for i, v in enumerate(box_frac))
    im = img.crop((x0, y0, x1, y1))
    out = remove_bg(im, tol=tol)
    bbox = out.getbbox()
    if bbox:
        b = (max(0, bbox[0] - pad), max(0, bbox[1] - pad),
             min(out.width, bbox[2] + pad), min(out.height, bbox[3] + pad))
        out = out.crop(b)
    out.save(OUT / f"{name}.png")
    print(f"{name}: {out.size}")


# ===== 主形象（左上 3D 渲染坐姿）=====
crop_frac("main", (0.015, 0.005, 0.50, 0.47), tol=30)

# ===== 表情行（5 个头像；只裁猫头本体，避开下方标签与相邻图案）=====
expr_row_y = (0.468, 0.600)  # y 范围，0.60 以下开始出现中文标签
exprs = [
    ("expr-curious",   0.218, 0.340),
    ("expr-happy",     0.358, 0.480),
    ("expr-focused",   0.502, 0.630),
    ("expr-surprised", 0.682, 0.808),
    ("expr-sleeping",  0.828, 0.972),
]
for name, x0, x1 in exprs:
    crop_frac(name, (x0, expr_row_y[0], x1, expr_row_y[1]), tol=45)

# ===== 场景行（3 个，含浅色面板背景）=====
crop_frac("scene-explore", (0.043, 0.655, 0.263, 0.872), tol=40)
crop_frac("scene-execute", (0.278, 0.655, 0.498, 0.872), tol=40)
crop_frac("scene-grow",    (0.511, 0.655, 0.731, 0.872), tol=55)

print("done ->", OUT)
