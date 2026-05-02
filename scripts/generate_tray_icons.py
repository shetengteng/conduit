#!/usr/bin/env python3
"""macOS 菜单栏托盘图标生成器(template image:只用 alpha 通道)。

macOS 菜单栏会把 icon 当 template:忽略颜色,只读 alpha,然后由系统在 light/dark
mode 下分别渲染为深色/浅色。所以这里的图形必须用"实色 + 透明"画,不能用渐变 /
半透明,否则 menu bar 上看到的会是糊掉的灰块。

输出: 按 macOS 菜单栏惯例输出 1x / 2x / 3x:
  scripts/build/tray-{role}.png        22 x 22  @1x
  scripts/build/tray-{role}@2x.png     44 x 44  @2x
  scripts/build/tray-{role}@3x.png     66 x 66  @3x

设计: 直接复用 RemixIcon 的 path,与 dock app icon (V6-remix) 保持视觉一致。
  Server: RiBroadcastLine
  Client: RiPlug2Line
"""
from __future__ import annotations

import io
import sys
from pathlib import Path

import cairosvg
from PIL import Image

ROLES = ("server", "client")
SIZE_AT_1X = 22
SIZES = [(SIZE_AT_1X, ""), (SIZE_AT_1X * 2, "@2x"), (SIZE_AT_1X * 3, "@3x")]

# 与 generate_app_icons.py 的 V6 同步。把这两个 path 集中放在 generate_app_icons
# 也行,但那样会引入循环依赖,这里复制一份保持脚本独立。
REMIX_PATHS = {
    "server": "M4.92893 2.92896L6.34315 4.34317C4.89543 5.79088 4 7.79088 4 10C4 12.2092 4.89543 14.2092 6.34315 15.6569L4.92893 17.0711C3.11929 15.2614 2 12.7614 2 10C2 7.2386 3.11929 4.7386 4.92893 2.92896ZM19.0711 2.92896C20.8807 4.7386 22 7.2386 22 10C22 12.7614 20.8807 15.2614 19.0711 17.0711L17.6569 15.6569C19.1046 14.2092 20 12.2092 20 10C20 7.79088 19.1046 5.79088 17.6569 4.34317L19.0711 2.92896ZM7.75736 5.75738L9.17157 7.1716C8.44771 7.89545 8 8.89545 8 10C8 11.1046 8.44771 12.1046 9.17157 12.8285L7.75736 14.2427C6.67157 13.1569 6 11.6569 6 10C6 8.34317 6.67157 6.84317 7.75736 5.75738ZM16.2426 5.75738C17.3284 6.84317 18 8.34317 18 10C18 11.6569 17.3284 13.1569 16.2426 14.2427L14.8284 12.8285C15.5523 12.1046 16 11.1046 16 10C16 8.89545 15.5523 7.89545 14.8284 7.1716L16.2426 5.75738ZM12 12C10.8954 12 10 11.1046 10 10C10 8.89545 10.8954 8.00002 12 8.00002C13.1046 8.00002 14 8.89545 14 10C14 11.1046 13.1046 12 12 12ZM11 14H13V22H11V14Z",
    "client": "M13 18V20H19V22H13C11.8954 22 11 21.1046 11 20V18H8C5.79086 18 4 16.2091 4 14V7C4 6.44772 4.44772 6 5 6H7V2H9V6H15V2H17V6H19C19.5523 6 20 6.44772 20 7V14C20 16.2091 18.2091 18 16 18H13ZM8 16H16C17.1046 16 18 15.1046 18 14V11H6V14C6 15.1046 6.89543 16 8 16ZM18 8H6V9H18V8ZM12 14.5C11.4477 14.5 11 14.0523 11 13.5C11 12.9477 11.4477 12.5 12 12.5C12.5523 12.5 13 12.9477 13 13.5C13 14.0523 12.5523 14.5 12 14.5ZM11 2H13V5H11V2Z",
}


def build(role: str, size: int) -> Image.Image:
    """渲染纯黑色 icon(menu bar 会用它的 alpha 通道做 template)。"""
    path = REMIX_PATHS[role]
    # 给 icon 留 ~9% padding,让 22x22 不顶住菜单栏边缘
    inner = max(8, round(size * 0.82))
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" '
        f'width="{inner}" height="{inner}">'
        f'<path d="{path}" fill="#000000"/></svg>'
    )
    png = cairosvg.svg2png(bytestring=svg.encode("utf-8"),
                           output_width=inner, output_height=inner)
    icon = Image.open(io.BytesIO(png)).convert("RGBA")

    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    bbox = icon.getbbox()
    if bbox is not None:
        l, t, r, b = bbox
        cx = size // 2
        cy = size // 2
        canvas.alpha_composite(icon, (cx - (r - l) // 2 - l, cy - (b - t) // 2 - t))
    else:
        canvas.alpha_composite(icon, ((size - inner) // 2, (size - inner) // 2))
    return canvas


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
