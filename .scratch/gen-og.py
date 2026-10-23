#!/usr/bin/env python3
"""Generate docs/og-image.png (1200x630) for social sharing cards."""
import random

from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 1200, 630
ZH_FONT = "/System/Library/Fonts/Hiragino Sans GB.ttc"
EN_FONT = "/System/Library/Fonts/HelveticaNeue.ttc"

img = Image.new("RGB", (W, H), (10, 14, 31))

# --- background orbs (blurred gradient circles) ---
orbs = [
    ((760, -120), 460, (79, 172, 254, 60)),
    ((1050, 420), 400, (168, 85, 247, 70)),
    ((180, 620), 380, (99, 102, 241, 55)),
    ((620, 700), 300, (79, 172, 254, 35)),
]
orb_layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
od = ImageDraw.Draw(orb_layer)
for (cx, cy), r, color in orbs:
    od.ellipse([cx - r, cy - r, cx + r, cy + r], fill=color)
orb_layer = orb_layer.filter(ImageFilter.GaussianBlur(120))
img = Image.alpha_composite(img.convert("RGBA"), orb_layer)

# --- stars ---
random.seed(42)
sd = ImageDraw.Draw(img)
for _ in range(140):
    x, y = random.randint(0, W), random.randint(0, H)
    a = random.randint(40, 170)
    s = random.choice([1, 1, 1, 2])
    sd.ellipse([x, y, x + s, y + s], fill=(255, 255, 255, a))
img = img.convert("RGB")
draw = ImageDraw.Draw(img)

# --- app icon (rounded) ---
icon = Image.open("docs/icon.png").convert("RGBA")
ICON = 210
icon = icon.resize((ICON, ICON), Image.LANCZOS)
mask = Image.new("L", (ICON, ICON), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, ICON, ICON], radius=48, fill=255)
img.paste(icon, (110, 150), mask)

# --- title ---
en = ImageFont.truetype(EN_FONT, 118, index=0)
zh_sub = ImageFont.truetype(ZH_FONT, 46, index=0)
feature = ImageFont.truetype(ZH_FONT, 34, index=0)
badge = ImageFont.truetype(ZH_FONT, 26, index=0)

draw.text((400, 130), "Astra", font=en, fill=(255, 255, 255))
draw.text((402, 292), "AI  桌面智能体工作台", font=zh_sub, fill=(148, 190, 255))

draw.line([(110, 418), (1090, 418)], fill=(60, 80, 130), width=2)

draw.text((110, 456), "本地 Agent 运行时 · 工具调用 · 记忆系统 · MCP", font=feature, fill=(200, 214, 245))
draw.text((110, 520), "React 19 + Tauri 2 · macOS / Windows", font=feature, fill=(130, 148, 190))

# --- top-right badge ---
bt = "Open Source"
bw = draw.textlength(bt, font=badge)
draw.rounded_rectangle([W - bw - 44 - 108, 44, W - 44, 92], radius=24, outline=(99, 130, 200), width=2)
draw.text((W - bw - 44 - 86, 54), bt, font=badge, fill=(140, 170, 230))

img.save("docs/og-image.png", optimize=True)
print("saved docs/og-image.png", img.size)
