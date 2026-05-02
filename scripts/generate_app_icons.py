#!/usr/bin/env python3
"""Conduit 应用图标生成器 —— 多设计 × 多角色矩阵。

设计候选(每个设计同时输出 server / client 两个角色版本):
  V1 · pipe-dot     ── 当前线上版本:水平管道 + 单数据点
  V2 · gradient     ── pipe-dot 的渐变质感升级:背景深色对角渐变 + 数据点高光
  V3 · ripple       ── 同心圆波纹,象征信号 / 连接
  V4 · letter-c     ── 抽象字母 C(断口表示通道入口)+ 中心数据点
  V5 · arc-bridge   ── 两条对称细弧在中心交汇成数据节点
  V6 · remix        ── 直接采用 RemixIcon 现成图标:Server=Broadcast(广播),
                       Client=Plug2(插头).不再自造图形,与 Sidebar / 全局图标
                       系统保持视觉一致.

输出:
  scripts/build/<design>-<role>.png  (1024x1024)

用法:
  python3 scripts/generate_app_icons.py             # 出全部设计 × 角色
  python3 scripts/generate_app_icons.py V2          # 仅出 V2
  python3 scripts/generate_app_icons.py V2 client   # 仅出 V2 client

最终选定方案后,用:
  pnpm --filter @conduit/<role>-app tauri icon ../scripts/build/<design>-<role>.png
"""
from __future__ import annotations

import io
import math
import sys
from pathlib import Path
from typing import Callable, Dict

import cairosvg
from PIL import Image, ImageDraw, ImageFilter, ImageFont

SIZE = 1024
RADIUS = int(SIZE * 0.226)
BG_DARK = (24, 24, 27)       # zinc-900
BG_DARK2 = (39, 39, 42)      # zinc-800
FG = (255, 255, 255)
ACCENT = (16, 185, 129)      # emerald-500
ACCENT_LIGHT = (52, 211, 153)  # emerald-400


# ---------------------------------------------------------------------------
# 通用工具
# ---------------------------------------------------------------------------


def squircle_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return mask


def solid_bg(color=BG_DARK) -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    layer = Image.new("RGBA", (SIZE, SIZE), (*color, 255))
    layer.putalpha(squircle_mask(SIZE, RADIUS))
    img.alpha_composite(layer)
    return img


def diagonal_gradient_bg(c1=BG_DARK, c2=BG_DARK2) -> Image.Image:
    """从左上 c1 到右下 c2 的线性渐变,然后裁成 squircle"""
    grad = Image.new("RGBA", (SIZE, SIZE), (*c1, 255))
    px = grad.load()
    for y in range(SIZE):
        for x in range(SIZE):
            t = (x + y) / (2 * SIZE)
            r = int(c1[0] * (1 - t) + c2[0] * t)
            g = int(c1[1] * (1 - t) + c2[1] * t)
            b = int(c1[2] * (1 - t) + c2[2] * t)
            px[x, y] = (r, g, b, 255)
    grad.putalpha(squircle_mask(SIZE, RADIUS))
    return grad


def add_inner_highlight(img: Image.Image, intensity: int = 30) -> None:
    """在 squircle 顶部加一抹微妙的内高光,模拟 macOS app icon 的玻璃感。

    BUG-FIX: 早期版本用 layer.putalpha(squircle_mask),会把 layer 整个 alpha 覆盖
    成 squircle,导致下半部"看起来不存在的高光"也被强行 alpha=255 写入,把 squircle
    的上半部"擦"掉了。改成:
      1. layer 内只画顶部渐淡白条(其他区域 alpha=0)
      2. 再用 squircle mask **AND 操作**裁掉 squircle 之外的多余像素(可选,
         因为 squircle 之外的像素本来就是 0,不会污染)
      3. alpha_composite 到 img
    """
    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    for y in range(int(SIZE * 0.45)):
        alpha = int(intensity * (1 - y / (SIZE * 0.45)))
        if alpha > 0:
            draw.line([(0, y), (SIZE, y)], fill=(255, 255, 255, alpha))

    # 把高光层裁到 squircle 内(用 mask 的 AND 等价:取 alpha 与 squircle 的最小值)
    mask = squircle_mask(SIZE, RADIUS)
    layer_alpha = layer.split()[3]
    new_alpha = Image.eval(layer_alpha, lambda a: a)  # copy
    # 用 mask 作为上限,逐点 min(alpha, mask)
    new_alpha = Image.composite(layer_alpha, Image.new("L", (SIZE, SIZE), 0), mask)
    layer.putalpha(new_alpha)

    img.alpha_composite(layer)


def gradient_circle(center, radius, c1, c2) -> Image.Image:
    """画一个径向渐变球(c1 中心, c2 边缘)"""
    cx, cy = center
    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    px = layer.load()
    r2 = radius * radius
    for y in range(max(0, cy - radius - 1), min(SIZE, cy + radius + 1)):
        for x in range(max(0, cx - radius - 1), min(SIZE, cx + radius + 1)):
            dx = x - cx
            dy = y - cy
            d2 = dx * dx + dy * dy
            if d2 > r2:
                continue
            t = math.sqrt(d2) / radius
            r = int(c1[0] * (1 - t) + c2[0] * t)
            g = int(c1[1] * (1 - t) + c2[1] * t)
            b = int(c1[2] * (1 - t) + c2[2] * t)
            px[x, y] = (r, g, b, 255)
    return layer


# ---------------------------------------------------------------------------
# V1 · pipe-dot (当前线上版本)
# ---------------------------------------------------------------------------


def design_pipe_dot(role: str) -> Image.Image:
    img = solid_bg()
    cy = SIZE // 2
    pad = int(SIZE * 0.18)
    tube_w = int(SIZE * 0.115)
    left, right = pad, SIZE - pad
    draw = ImageDraw.Draw(img)
    draw.rounded_rectangle((left, cy - tube_w // 2, right, cy + tube_w // 2),
                           radius=tube_w // 2, fill=FG)
    dot_r = int(tube_w * 0.62)
    dot_x = int(left + (right - left) * (0.78 if role == "server" else 0.22))
    fx = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(fx).ellipse((dot_x - dot_r, cy - dot_r, dot_x + dot_r, cy + dot_r), fill=(*ACCENT, 255))
    img.alpha_composite(fx)
    return img


# ---------------------------------------------------------------------------
# V2 · gradient (V1 的质感升级)
# ---------------------------------------------------------------------------


def design_gradient(role: str) -> Image.Image:
    img = diagonal_gradient_bg()
    add_inner_highlight(img, intensity=22)
    cy = SIZE // 2
    pad = int(SIZE * 0.18)
    tube_w = int(SIZE * 0.115)
    left, right = pad, SIZE - pad

    # 管道:底层 + 顶部高光,模拟磨砂玻璃管
    pipe_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    pdraw = ImageDraw.Draw(pipe_layer)
    pdraw.rounded_rectangle((left, cy - tube_w // 2, right, cy + tube_w // 2),
                            radius=tube_w // 2, fill=(255, 255, 255, 235))
    # 顶部 1/3 加白色高光带
    pdraw.rounded_rectangle((left + 4, cy - tube_w // 2 + 4,
                             right - 4, cy - int(tube_w * 0.05)),
                            radius=tube_w // 2 - 4, fill=(255, 255, 255, 90))
    img.alpha_composite(pipe_layer)

    # 数据点:径向渐变 + 外发光
    dot_r = int(tube_w * 0.66)
    dot_x = int(left + (right - left) * (0.78 if role == "server" else 0.22))

    # 外发光(模糊圆)
    glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse(
        (dot_x - int(dot_r * 1.8), cy - int(dot_r * 1.8),
         dot_x + int(dot_r * 1.8), cy + int(dot_r * 1.8)),
        fill=(*ACCENT, 110),
    )
    glow = glow.filter(ImageFilter.GaussianBlur(radius=int(dot_r * 0.45)))
    img.alpha_composite(glow)

    # 渐变球
    img.alpha_composite(gradient_circle((dot_x, cy), dot_r, ACCENT_LIGHT, ACCENT))
    # 顶部小高光
    hl = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(hl).ellipse(
        (dot_x - int(dot_r * 0.45), cy - int(dot_r * 0.7),
         dot_x + int(dot_r * 0.15), cy - int(dot_r * 0.25)),
        fill=(255, 255, 255, 130),
    )
    img.alpha_composite(hl)
    return img


# ---------------------------------------------------------------------------
# V3 · ripple (同心圆波纹)
# ---------------------------------------------------------------------------


def design_ripple(role: str) -> Image.Image:
    img = diagonal_gradient_bg()
    add_inner_highlight(img, intensity=18)
    cx = cy = SIZE // 2
    # 三圈波纹,从内到外渐淡
    ring_color = ACCENT if role == "server" else FG
    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ld = ImageDraw.Draw(layer)
    for i, (radius_ratio, alpha, thickness_ratio) in enumerate([
        (0.42, 60,  0.018),
        (0.30, 110, 0.022),
        (0.20, 175, 0.028),
    ]):
        r = int(SIZE * radius_ratio)
        thickness = max(2, int(SIZE * thickness_ratio))
        ld.ellipse((cx - r, cy - r, cx + r, cy + r), outline=(*ring_color, alpha), width=thickness)
    img.alpha_composite(layer)
    # 中心点
    center_r = int(SIZE * 0.075)
    center_color = FG if role == "server" else ACCENT
    img.alpha_composite(gradient_circle((cx, cy), center_r, (255, 255, 255) if center_color == FG else ACCENT_LIGHT, center_color))
    return img


# ---------------------------------------------------------------------------
# V4 · letter-c (抽象字母 C + 中心数据点)
# ---------------------------------------------------------------------------


def design_letter_c(role: str) -> Image.Image:
    img = diagonal_gradient_bg()
    add_inner_highlight(img, intensity=18)
    cx = cy = SIZE // 2
    outer_r = int(SIZE * 0.34)
    thickness = int(SIZE * 0.075)
    inner_r = outer_r - thickness

    # C 形:画 outline 圆,然后用 squircle 颜色挖掉右侧 1/4 缺口
    c_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    cd = ImageDraw.Draw(c_layer)
    cd.ellipse((cx - outer_r, cy - outer_r, cx + outer_r, cy + outer_r), fill=(*FG, 250))
    cd.ellipse((cx - inner_r, cy - inner_r, cx + inner_r, cy + inner_r), fill=(0, 0, 0, 0))
    # 挖掉右侧扇形缺口(server 缺口在右,client 缺口在左)
    cut_size = int(thickness * 1.7)
    if role == "server":
        cd.rectangle((cx + inner_r - 4, cy - cut_size, cx + outer_r + 4, cy + cut_size), fill=(0, 0, 0, 0))
    else:
        cd.rectangle((cx - outer_r - 4, cy - cut_size, cx - inner_r + 4, cy + cut_size), fill=(0, 0, 0, 0))
    img.alpha_composite(c_layer)

    # 中心 emerald 数据点
    dot_r = int(SIZE * 0.07)
    img.alpha_composite(gradient_circle((cx, cy), dot_r, ACCENT_LIGHT, ACCENT))
    # 高光
    hl = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(hl).ellipse(
        (cx - int(dot_r * 0.4), cy - int(dot_r * 0.6),
         cx + int(dot_r * 0.15), cy - int(dot_r * 0.2)),
        fill=(255, 255, 255, 140),
    )
    img.alpha_composite(hl)
    return img


# ---------------------------------------------------------------------------
# V5 · arc-bridge (两条弧线交汇 + 中心节点)
# ---------------------------------------------------------------------------


def design_arc_bridge(role: str) -> Image.Image:
    img = diagonal_gradient_bg()
    add_inner_highlight(img, intensity=18)
    cx = cy = SIZE // 2
    pad = int(SIZE * 0.16)
    arc_thickness = int(SIZE * 0.045)
    # 两条对称的优雅弧线:从左/右边缘出发向中心收束
    # 用 PIL.ImageDraw 的 arc 以椭圆切片实现
    arc_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ad = ImageDraw.Draw(arc_layer)
    # 左弧:中心在 (right-pad, cy),向左侧出弧
    big_r = int(SIZE * 0.62)
    # 圆弧 1:左半弧,bbox 中心在右侧
    box1 = (cx - big_r, cy - big_r, cx + big_r, cy + big_r)
    # 顶部弧 (135° → 90° → 45°)
    ad.arc(box1, start=170, end=190, fill=(*FG, 230), width=arc_thickness)
    # 用两个独立 arc 形成"X 桥":
    pad2 = int(SIZE * 0.10)
    box_a = (-int(SIZE * 0.30), -int(SIZE * 0.30), int(SIZE * 0.78), int(SIZE * 0.78))
    box_b = (int(SIZE * 0.22), -int(SIZE * 0.30), int(SIZE * 1.30), int(SIZE * 0.78))
    box_c = (-int(SIZE * 0.30), int(SIZE * 0.22), int(SIZE * 0.78), int(SIZE * 1.30))
    box_d = (int(SIZE * 0.22), int(SIZE * 0.22), int(SIZE * 1.30), int(SIZE * 1.30))
    arc_color = (*FG, 235)
    # 上左弧 → 中心
    ad.arc(box_d, start=180, end=270, fill=arc_color, width=arc_thickness)
    # 上右弧 → 中心
    ad.arc(box_c, start=270, end=360, fill=arc_color, width=arc_thickness)
    # 下左弧 → 中心
    ad.arc(box_b, start=90,  end=180, fill=arc_color, width=arc_thickness)
    # 下右弧 → 中心
    ad.arc(box_a, start=0,   end=90,  fill=arc_color, width=arc_thickness)
    img.alpha_composite(arc_layer)

    # 中心节点(role 决定大小:server 略大表示数据汇聚源)
    node_r = int(SIZE * (0.085 if role == "server" else 0.075))
    glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse((cx - int(node_r * 2.0), cy - int(node_r * 2.0),
                                  cx + int(node_r * 2.0), cy + int(node_r * 2.0)),
                                 fill=(*ACCENT, 110))
    glow = glow.filter(ImageFilter.GaussianBlur(radius=int(node_r * 0.5)))
    img.alpha_composite(glow)
    img.alpha_composite(gradient_circle((cx, cy), node_r, ACCENT_LIGHT, ACCENT))
    hl = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(hl).ellipse(
        (cx - int(node_r * 0.4), cy - int(node_r * 0.65),
         cx + int(node_r * 0.15), cy - int(node_r * 0.2)),
        fill=(255, 255, 255, 130),
    )
    img.alpha_composite(hl)
    return img


# ---------------------------------------------------------------------------
# V6 · remix (RemixIcon SVG path 嵌入 squircle)
# ---------------------------------------------------------------------------

# RemixIcon 4.x 的 path 数据(viewBox 24x24,fill=currentColor)
# Server 用 RiBroadcastLine,Client 用 RiPlug2Line
REMIX_PATHS = {
    "server": "M4.92893 2.92896L6.34315 4.34317C4.89543 5.79088 4 7.79088 4 10C4 12.2092 4.89543 14.2092 6.34315 15.6569L4.92893 17.0711C3.11929 15.2614 2 12.7614 2 10C2 7.2386 3.11929 4.7386 4.92893 2.92896ZM19.0711 2.92896C20.8807 4.7386 22 7.2386 22 10C22 12.7614 20.8807 15.2614 19.0711 17.0711L17.6569 15.6569C19.1046 14.2092 20 12.2092 20 10C20 7.79088 19.1046 5.79088 17.6569 4.34317L19.0711 2.92896ZM7.75736 5.75738L9.17157 7.1716C8.44771 7.89545 8 8.89545 8 10C8 11.1046 8.44771 12.1046 9.17157 12.8285L7.75736 14.2427C6.67157 13.1569 6 11.6569 6 10C6 8.34317 6.67157 6.84317 7.75736 5.75738ZM16.2426 5.75738C17.3284 6.84317 18 8.34317 18 10C18 11.6569 17.3284 13.1569 16.2426 14.2427L14.8284 12.8285C15.5523 12.1046 16 11.1046 16 10C16 8.89545 15.5523 7.89545 14.8284 7.1716L16.2426 5.75738ZM12 12C10.8954 12 10 11.1046 10 10C10 8.89545 10.8954 8.00002 12 8.00002C13.1046 8.00002 14 8.89545 14 10C14 11.1046 13.1046 12 12 12ZM11 14H13V22H11V14Z",
    "client": "M13 18V20H19V22H13C11.8954 22 11 21.1046 11 20V18H8C5.79086 18 4 16.2091 4 14V7C4 6.44772 4.44772 6 5 6H7V2H9V6H15V2H17V6H19C19.5523 6 20 6.44772 20 7V14C20 16.2091 18.2091 18 16 18H13ZM8 16H16C17.1046 16 18 15.1046 18 14V11H6V14C6 15.1046 6.89543 16 8 16ZM18 8H6V9H18V8ZM12 14.5C11.4477 14.5 11 14.0523 11 13.5C11 12.9477 11.4477 12.5 12 12.5C12.5523 12.5 13 12.9477 13 13.5C13 14.0523 12.5523 14.5 12 14.5ZM11 2H13V5H11V2Z",
}


def _render_remix_svg(role: str, target_px: int, color: str) -> Image.Image:
    """把 24x24 viewBox 的 RemixIcon path 渲染成 target_px × target_px 的 PNG。"""
    path = REMIX_PATHS[role]
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" '
        f'width="{target_px}" height="{target_px}">'
        f'<path d="{path}" fill="{color}"/></svg>'
    )
    png_bytes = cairosvg.svg2png(bytestring=svg.encode("utf-8"),
                                 output_width=target_px, output_height=target_px)
    return Image.open(io.BytesIO(png_bytes)).convert("RGBA")


def design_remix(role: str) -> Image.Image:
    """RemixIcon 路径(白色) + 黑色 squircle 背景 + 顶部微高光。

    注意 RemixIcon 24x24 viewBox 的视觉重心通常不严格在 (12,12),
    所以贴的时候 y 方向也按 path bbox 居中,而不是 viewBox 居中。
    """
    img = solid_bg(BG_DARK)
    add_inner_highlight(img, intensity=20)

    # icon 占 squircle 内 ~52% 宽(squircle 本身就有 ~17% padding,
    # 这里再缩小一点,让 icon 视觉占 squircle 内核约 70%)
    icon_px = int(SIZE * 0.52)
    icon = _render_remix_svg(role, icon_px, "#ffffff")
    # 找到 icon 实际的非透明 bbox,以此为中心贴
    bbox = icon.getbbox()  # (l,t,r,b) 非透明区域
    if bbox is not None:
        l, t, r, b = bbox
        actual_w = r - l
        actual_h = b - t
        # 把 actual bbox 居中到 SIZE 中心
        cx = SIZE // 2
        cy = SIZE // 2
        paste_x = cx - actual_w // 2 - l
        paste_y = cy - actual_h // 2 - t
        img.alpha_composite(icon, (paste_x, paste_y))
    else:
        img.alpha_composite(icon, ((SIZE - icon_px) // 2, (SIZE - icon_px) // 2))
    return img


# ---------------------------------------------------------------------------
# Registry + CLI
# ---------------------------------------------------------------------------


DESIGNS: Dict[str, Callable[[str], Image.Image]] = {
    "V1-pipe-dot":   design_pipe_dot,
    "V2-gradient":   design_gradient,
    "V3-ripple":     design_ripple,
    "V4-letter-c":   design_letter_c,
    "V5-arc-bridge": design_arc_bridge,
    "V6-remix":      design_remix,
}

ROLES = ("server", "client")


def main() -> int:
    out_dir = Path(__file__).resolve().parent / "build"
    out_dir.mkdir(parents=True, exist_ok=True)

    args = sys.argv[1:]
    selected_designs = list(DESIGNS.keys())
    selected_roles = list(ROLES)
    if args:
        # 简单 CLI:第 1 参数选 design (含 'V2' 这种短前缀),第 2 参数选 role
        prefix = args[0]
        selected_designs = [d for d in DESIGNS if d.startswith(prefix) or d == prefix or prefix in d]
        if not selected_designs:
            print(f"unknown design '{prefix}',可选: {', '.join(DESIGNS)}", file=sys.stderr)
            return 2
        if len(args) > 1:
            if args[1] not in ROLES:
                print(f"unknown role '{args[1]}',可选: {', '.join(ROLES)}", file=sys.stderr)
                return 2
            selected_roles = [args[1]]

    for design in selected_designs:
        for role in selected_roles:
            img = DESIGNS[design](role)
            path = out_dir / f"{design}-{role}.png"
            img.save(path, "PNG")
            print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
