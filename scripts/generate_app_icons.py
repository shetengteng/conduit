#!/usr/bin/env python3
"""生成 Conduit 应用图标 —— Server 与 Client 一对(M-δ)。

设计语义:
  * 黑色圆角矩形(macOS squircle, 半径 ≈ 22.6%) —— 与 macOS 系统 UI 自然融合
  * 白色 "通道+数据流" 图形 —— 呼应 "conduit" 本意,数据从一端流向另一端
  * Server 与 Client 用 1 个色彩点位区分:
      - Server 端: 数据点位居右(发出)
      - Client 端: 数据点位居左(接入)
  * 不引入第三色,纯黑白 + 一抹绿色高亮(品牌色 emerald-500),克制、企业感

输出:
  scripts/build/conduit-server-icon.png   (1024x1024, 给 server-app)
  scripts/build/conduit-client-icon.png   (1024x1024, 给 client-app)

后续:
  pnpm --filter @conduit/server-app tauri icon ../../../scripts/build/conduit-server-icon.png
  pnpm --filter @conduit/client-app tauri icon ../../../scripts/build/conduit-client-icon.png
"""
from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 1024
RADIUS = int(SIZE * 0.226)   # macOS squircle 比例
PADDING = int(SIZE * 0.18)   # 增加留白,让主图形更聚焦
BG = (24, 24, 27)            # zinc-900,与 Sidebar primary 同色
FG = (255, 255, 255)         # 白
ACCENT = (16, 185, 129)      # emerald-500,品牌点缀色
TUBE_W = int(SIZE * 0.115)   # 加粗管道,32x32 也能看清


def rounded_rect_mask(size: int, radius: int) -> Image.Image:
    """生成 macOS squircle 蒙板"""
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return mask


def draw_conduit(img: Image.Image, *, role: str) -> None:
    """画 "通道 + 数据点":
    极简 3 元素 —— 白色圆角胶囊管道 + 1 个 emerald 数据点 + 1 个白色端口圆
    够小到在 32x32 下也能识别"管道里有个绿点"。

    - Server: 数据点位置居右 80% (发出端,绿点已抵达右端口口边)
    - Client: 数据点位置居左 20% (接入端,绿点刚从左端口涌入)
    """
    draw = ImageDraw.Draw(img)
    cy = SIZE // 2
    left = PADDING
    right = SIZE - PADDING
    top = cy - TUBE_W // 2
    bottom = cy + TUBE_W // 2
    draw.rounded_rectangle((left, top, right, bottom), radius=TUBE_W // 2, fill=FG)

    # Server 数据点居右、Client 数据点居左,留 12% 边距给端口空间
    if role == "server":
        dot_x = int(left + (right - left) * 0.78)
    else:
        dot_x = int(left + (right - left) * 0.22)

    dot_r = int(TUBE_W * 0.62)
    fx_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    fx_draw = ImageDraw.Draw(fx_layer)
    fx_draw.ellipse((dot_x - dot_r, cy - dot_r, dot_x + dot_r, cy + dot_r), fill=(*ACCENT, 255))
    img.alpha_composite(fx_layer)


def build_icon(role: str) -> Image.Image:
    assert role in ("server", "client")
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    bg_layer = Image.new("RGBA", (SIZE, SIZE), (*BG, 255))
    bg_layer.putalpha(rounded_rect_mask(SIZE, RADIUS))
    img.alpha_composite(bg_layer)
    draw_conduit(img, role=role)
    return img


def main() -> int:
    out_dir = Path(__file__).resolve().parent / "build"
    out_dir.mkdir(parents=True, exist_ok=True)
    for role in ("server", "client"):
        img = build_icon(role)
        path = out_dir / f"conduit-{role}-icon.png"
        img.save(path, "PNG")
        print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
