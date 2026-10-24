"""把多张吉祥物变体图拼成一张多格图（grid / contact sheet）。

用法:
  python3 mascot_grid.py <img1> <img2> ... [--cols N] [--out grid.png] [--pad 20] [--bg white|transparent]
  python3 mascot_grid.py <目录> [--cols N]  # 目录里所有 PNG 按文件名排序拼接

特性:
- 每张图缩放到同一高度（--cell-h，默认 512px），保持宽高比
- 自动按 cols 排布，不足一行的用空白占位
- 支持透明 PNG 输入；--bg white 时铺白底（便于查看/打印），transparent 输出透明底
"""
import argparse
import sys
from pathlib import Path

from PIL import Image

def parse_args():
    ap = argparse.ArgumentParser(description="拼吉祥物变体为多格图")
    ap.add_argument("inputs", nargs="+", help="图片文件或目录")
    ap.add_argument("--cols", type=int, default=3, help="每行格子数（默认 3）")
    ap.add_argument("--out", default="mascot-grid.png", help="输出文件名")
    ap.add_argument("--cell-h", type=int, default=512, help="每格目标高度像素（默认 512）")
    ap.add_argument("--pad", type=int, default=24, help="格子间距像素（默认 24）")
    ap.add_argument("--bg", choices=["white", "transparent"], default="white",
                    help="输出背景（默认 white）")
    return ap.parse_args()


def collect_paths(args) -> list[Path]:
    paths = []
    for inp in args.inputs:
        p = Path(inp)
        if p.is_dir():
            paths.extend(sorted(p.glob("*.png")))
        elif p.is_file():
            paths.append(p)
    if not paths:
        print("没有找到任何 PNG 图片")
        sys.exit(1)
    return paths


def load_cell(path: Path, target_h: int) -> Image.Image:
    im = Image.open(path).convert("RGBA")
    # 透明图先去透明边，再缩放到目标高度
    bbox = im.getbbox()
    if bbox:
        im = im.crop(bbox)
    w, h = im.size
    scale = target_h / h
    im = im.resize((max(1, int(w * scale)), target_h), Image.LANCZOS)
    return im


def main():
    args = parse_args()
    paths = collect_paths(args)
    cells = [load_cell(p, args.cell_h) for p in paths]
    print(f"共 {len(cells)} 张，每格高 {args.cell_h}px，每行 {args.cols} 列")

    cols = args.cols
    rows = (len(cells) + cols - 1) // cols
    cell_w = max(c.width for c in cells)
    pad = args.pad

    # 画布：cell_w*cols + pad*(cols+1) x cell_h*rows + pad*(rows+1)
    W = cell_w * cols + pad * (cols + 1)
    H = args.cell_h * rows + pad * (rows + 1)
    if args.bg == "white":
        canvas = Image.new("RGBA", (W, H), (255, 255, 255, 255))
    else:
        canvas = Image.new("RGBA", (W, H), (0, 0, 0, 0))

    for i, cell in enumerate(cells):
        r, c = i // cols, i % cols
        # 水平居中
        x = pad + c * (cell_w + pad) + (cell_w - cell.width) // 2
        y = pad + r * (args.cell_h + pad)
        canvas.paste(cell, (x, y), cell)

    out = Path(args.out)
    canvas.convert("RGB" if args.bg == "white" else "RGBA").save(out)
    print(f"已输出: {out} ({W}x{H})")


if __name__ == "__main__":
    main()
