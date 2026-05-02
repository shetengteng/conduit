#!/usr/bin/env python3
"""macOS 菜单栏托盘图标生成器(template image:只用 alpha 通道)。

macOS 菜单栏会把 icon 当 template:忽略颜色,只读 alpha,然后由系统在 light/dark
mode 下分别渲染为深色/浅色。所以这里的图形必须用"实色 + 透明"画,不能用渐变 /
半透明,否则 menu bar 上看到的会是糊掉的灰块。

输出: 按 macOS 菜单栏惯例输出 1x / 2x / 3x:
  scripts/build/tray-{role}.png        22 x 22  @1x
  scripts/build/tray-{role}@2x.png     44 x 44  @2x
  scripts/build/tray-{role}@3x.png     66 x 66  @3x

设计:
  Server:    一根粗短管道 + 中心向右一个数据点(broadcast 出去)
  Client:    一根粗短管道 + 中心向左一个数据点(接入进来)

注意:tauri 当前 set_icon API 接受 ImageRgba 或 PathBuf,这里只生成 @2x 单文件
即可,因为我们后面的 set_icon 只能给一张.让 macOS 自己降采样;@1x/@3x 留作
未来如果需要 NSImage representations 时再用.
"""
from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROLES = ("server", "client")
SIZE_AT_1X = 22
SIZES = [(SIZE_AT_1X, ""), (SIZE_AT_1X * 2, "@2x"), (SIZE_AT_1X * 3, "@3x")]


def build(role: str, size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 比例:管道高度 ≈ 28% size,水平 padding ≈ 9% size
    pad_x = max(2, round(size * 0.09))
    tube_h = max(3, round(size * 0.28))
    cy = size // 2
    left, right = pad_x, size - pad_x
    top, bottom = cy - tube_h // 2, cy + tube_h // 2

    # 管道:实心(template 视角下就是这个形状)
    draw.rounded_rectangle(
        (left, top, right, bottom),
        radius=tube_h // 2,
        fill=(0, 0, 0, 255),
    )

    # 数据点:Server 居右,Client 居左
    dot_r = max(2, round(tube_h * 0.42))
    if role == "server":
        dot_x = round(left + (right - left) * 0.78)
    else:
        dot_x = round(left + (right - left) * 0.22)
    # 用透明(alpha=0)挖出数据点 -> 在 template 渲染时反而显示为对比色
    # macOS template image 看 alpha:不透明=系统色,透明=背景透出
    # 我们要"数据点"在视觉上突出,所以做成"管道里的洞"
    # ---> Pillow 的 ellipse 没有"擦除"模式,但我们是新图层,直接用 (0,0,0,0) 覆盖即可
    fx_layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    fx_draw = ImageDraw.Draw(fx_layer)
    fx_draw.ellipse(
        (dot_x - dot_r, cy - dot_r, dot_x + dot_r, cy + dot_r),
        fill=(0, 0, 0, 255),
    )
    # 用 fx 作为蒙版,把图像中对应区域的 alpha 设为 0 -> 形成镂空"洞"
    px_img = img.load()
    px_fx = fx_layer.load()
    for y in range(cy - dot_r, cy + dot_r + 1):
        for x in range(dot_x - dot_r, dot_x + dot_r + 1):
            if 0 <= x < size and 0 <= y < size and px_fx[x, y][3] > 0:
                # 让此像素透明,从而形成镂空效果
                r, g, b, _ = px_img[x, y]
                px_img[x, y] = (r, g, b, 0)
    return img


def main() -> int:
    out_dir = Path(__file__).resolve().parent / "build"
    out_dir.mkdir(parents=True, exist_ok=True)
    for role in ROLES:
        for size, suffix in SIZES:
            img = build(role, size)
            path = out_dir / f"tray-{role}{suffix}.png"
            img.save(path, "PNG")
            print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
