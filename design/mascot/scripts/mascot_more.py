"""从已拆好的吉祥物 PNG 程序化生成动画帧素材，供前端平面简单交互使用。

输出到 src/public/mascot/anim/：
  bounce-{0..4}.png    main 垂直浮动循环帧（呼吸/待机浮动）
  jump-{0..3}.png      main 弹跳帧（上跳 + 落地压缩，统一画布）
  walk-{0..3}.png      scene-execute 走路摇摆帧（水平位移 + 微旋转）
  flip-{name}.png      3 个场景的水平镜像版（朝左）
  decor-star-tail.png  从 main 裁出的尾巴四角星（透明）
  decor-star-scarf.png 从 main 裁出的围巾四角星（透明）
"""
from PIL import Image
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SOURCE_DIR = REPO_ROOT / "design" / "mascot" / "source"
OUT = REPO_ROOT / "src" / "public" / "mascot"
ANIM = OUT / "anim"
ANIM.mkdir(parents=True, exist_ok=True)


def load(name: str) -> Image.Image:
    return Image.open(OUT / f"{name}.png").convert("RGBA")


def save(im: Image.Image, name: str):
    im.save(ANIM / name)
    print(f"{name}: {im.size}")


def shift(im: Image.Image, dx: int, dy: int, pad: int = 16) -> Image.Image:
    canvas = Image.new("RGBA", (im.width + pad * 2, im.height + pad * 2), (0, 0, 0, 0))
    canvas.paste(im, (pad + dx, pad + dy), im)
    return canvas


def rotate_around_bottom(im: Image.Image, deg: float, pad: int = 24) -> Image.Image:
    canvas = Image.new("RGBA", (im.width + pad * 2, im.height + pad * 2), (0, 0, 0, 0))
    canvas.paste(im, (pad, pad), im)
    cx, cy = canvas.width / 2, canvas.height - pad - 2
    return canvas.rotate(deg, center=(cx, cy), resample=Image.BICUBIC)


# ===== 1) main 浮动帧 =====
main = load("main")
for i, dy in enumerate([0, -3, -5, -3, 0]):
    save(shift(main, 0, dy), f"bounce-{i}.png")

# ===== 2) main 弹跳帧（统一画布 = bounce 的画布）=====
b0 = Image.open(ANIM / "bounce-0.png").convert("RGBA")
canvas = Image.new("RGBA", b0.size, (0, 0, 0, 0))
# 起跳/最高/下落：在统一画布上移动
for i, dy in enumerate([-14, -24, -8]):
    c = canvas.copy()
    c.paste(main, (16, 16 + dy), main)
    save(c, f"jump-{i}.png")
# 落地压缩：同画布，底部对齐
compressed = main.resize((int(main.width * 0.96), int(main.height * 0.90)), Image.LANCZOS)
c = canvas.copy()
c.paste(compressed, (16 + (main.width - compressed.width) // 2, 16 + main.height - compressed.height), compressed)
save(c, "jump-3.png")

# ===== 3) 走路摇摆帧 =====
walk = load("scene-execute")
for i, (dx, deg) in enumerate([(-6, -2.5), (4, 2.0), (8, 0.0), (4, 2.5)]):
    f = rotate_around_bottom(walk, deg)
    canvas = Image.new("RGBA", (f.width + 12, f.height + 12), (0, 0, 0, 0))
    canvas.paste(f, (6 + dx, 6), f)
    save(canvas, f"walk-{i}.png")

# ===== 4) 场景镜像 =====
for name in ["explore", "execute", "grow"]:
    im = load(f"scene-{name}")
    save(im.transpose(Image.FLIP_LEFT_RIGHT), f"flip-{name}.png")

# ===== 5) 星星装饰：main.png（透明底）白色簇坐标定位 =====
# 已知 main.png 495x531：两只眼睛在上部对称，尾巴星在左中部，围巾星在胸口右中部。
# 硬编码坐标（由前面分析确认），避免形状判定误判。
TAIL_STAR = (60, 269, 109, 323)     # 左中部，尾巴尖的四角星
SCARF_STAR = (371, 325, 400, 359)   # 胸口右中部，围巾上的四角星
for box, outname in [(TAIL_STAR, "decor-star-tail.png"), (SCARF_STAR, "decor-star-scarf.png")]:
    bx0, by0, bx1, by1 = box
    star = main.crop((max(0, bx0 - 4), max(0, by0 - 4), bx1 + 4, by1 + 4))
    save(star, outname)

print("done ->", ANIM)
