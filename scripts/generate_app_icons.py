#!/usr/bin/env python3
"""Conduit 应用图标生成器 —— 多设计 × 多角色矩阵。

设计候选(每个设计同时输出 server / client 两个角色版本):
  V1 · pipe-dot     ── 当前线上版本:水平管道 + 单数据点
  V2 · gradient     ── pipe-dot 的渐变质感升级:背景深色对角渐变 + 数据点高光
  V3 · ripple       ── 同心圆波纹,象征信号 / 连接
  V4 · letter-c     ── 抽象字母 C(断口表示通道入口)+ 中心数据点
  V5 · arc-bridge   ── 两条对称细弧在中心交汇成数据节点

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

import math
import sys
from pathlib import Path
from typing import Callable, Dict

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
    """在 squircle 顶部加一抹微妙的内高光,模拟 macOS app icon 的玻璃感"""
    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    for y in range(int(SIZE * 0.45)):
        alpha = int(intensity * (1 - y / (SIZE * 0.45)))
        if alpha > 0:
            draw.line([(0, y), (SIZE, y)], fill=(255, 255, 255, alpha))
    layer.putalpha(squircle_mask(SIZE, RADIUS))
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
# Registry + CLI
# ---------------------------------------------------------------------------


DESIGNS: Dict[str, Callable[[str], Image.Image]] = {
    "V1-pipe-dot":   design_pipe_dot,
    "V2-gradient":   design_gradient,
    "V3-ripple":     design_ripple,
    "V4-letter-c":   design_letter_c,
    "V5-arc-bridge": design_arc_bridge,
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
