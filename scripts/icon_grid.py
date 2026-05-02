#!/usr/bin/env python3
"""把 scripts/build/ 下的候选 logo 排成 2 行 5 列对比图,方便用户一眼挑选。

输出: scripts/build/preview.png
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

CELL = 256                # 每个图标缩到 256
GAP = 18
LABEL_H = 36
ROLES = ("server", "client")
DESIGNS = ["V1-pipe-dot", "V2-gradient", "V3-ripple", "V4-letter-c", "V5-arc-bridge"]

BUILD = Path(__file__).resolve().parent / "build"


def main() -> int:
    cols = len(DESIGNS)
    rows = len(ROLES)
    w = cols * CELL + (cols + 1) * GAP
    h = rows * (CELL + LABEL_H) + (rows + 1) * GAP + LABEL_H  # 顶部留一行总标题
    canvas = Image.new("RGBA", (w, h), (250, 250, 252, 255))
    draw = ImageDraw.Draw(canvas)

    try:
        font = ImageFont.truetype("/System/Library/Fonts/SFNSMono.ttf", 18)
        title_font = ImageFont.truetype("/System/Library/Fonts/SFNS.ttf", 22)
    except OSError:
        font = ImageFont.load_default()
        title_font = font

    # 顶部标题
    draw.text((GAP, GAP // 2), "Conduit · 5 个 logo 候选 (上排=server, 下排=client)", fill=(20, 20, 23), font=title_font)

    for ri, role in enumerate(ROLES):
        for ci, design in enumerate(DESIGNS):
            path = BUILD / f"{design}-{role}.png"
            if not path.exists():
                continue
            icon = Image.open(path).convert("RGBA").resize((CELL, CELL), Image.LANCZOS)
            x = GAP + ci * (CELL + GAP)
            y = LABEL_H + GAP + ri * (CELL + LABEL_H + GAP)
            canvas.alpha_composite(icon, (x, y))
            label = design.split("-", 1)[1] if "-" in design else design  # "pipe-dot"
            label = f"{design.split('-')[0]} · {label}"
            draw.text((x, y + CELL + 6), label, fill=(80, 80, 90), font=font)

    out = BUILD / "preview.png"
    canvas.save(out, "PNG")
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
