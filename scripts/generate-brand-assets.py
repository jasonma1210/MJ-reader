#!/usr/bin/env python3
"""MJ Nexus Reader 品牌资产生成器。

从单一几何定义（44x44 点阵：MJ 字母 + 平放字典的 6 层页口）派生：
  1. frontend/src/components/brand/mjDotMatrix.ts  —— 运行时静态点坐标数据
  2. frontend/public/favicon.svg                    —— 浏览器 favicon
  3. src-tauri/icons/ 下全平台 PNG                  —— Tauri bundle 图标
  4. gen/android / gen/apple 生成副本               —— 保持与源同步

设计定稿参数（勿随意改，改了要整套重建）：
  - 点阵网格 44x44，MJ 用 Arial Bold 18px 居中于 (22,13)
  - 书页为左右各 6 条二次贝塞尔弧线，lineWidth 1.4
  - 采样阈值 alpha > 100
  - 双版本：黑白（白底黑点）/ 白黑（黑底白点）
"""
from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
SS = 8                      # 超采样倍率
GRID = 44
THRESHOLD = 100
FONT_PATH = "/System/Library/Fonts/Supplemental/Arial Bold.ttf"
FONT_SIZE = 18
STROKE_W = 1.4

WHITE = (255, 255, 255)
BLACK = (0, 0, 0)


def bezier2(p0, p1, p2, steps=96):
    """二次贝塞尔曲线采样。"""
    out = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        x = u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0]
        y = u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]
        out.append((x, y))
    return out


def render_mask() -> Image.Image:
    """复现浏览器预览的 canvas 几何，返回 44x44 灰度 mask。"""
    size = GRID * SS
    img = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(img)

    # MJ 字母（textAlign=center, textBaseline=middle）
    font = ImageFont.truetype(FONT_PATH, FONT_SIZE * SS)
    draw.text((22 * SS, 13 * SS), "MJ", font=font, fill=255, anchor="mm")

    # 书页：左右各 6 层二次贝塞尔弧线
    width = max(1, round(STROKE_W * SS))
    for i in range(6):
        left = bezier2(
            (21, 38 - i * 0.7),
            (13, 34 - i * 0.9),
            (5 + i * 0.4, 36 - i * 1.0),
        )
        right = bezier2(
            (23, 38 - i * 0.7),
            (31, 34 - i * 0.9),
            (39 - i * 0.4, 36 - i * 1.0),
        )
        draw.line(
            [(x * SS, y * SS) for x, y in left],
            fill=255, width=width, joint="curve",
        )
        draw.line(
            [(x * SS, y * SS) for x, y in right],
            fill=255, width=width, joint="curve",
        )

    # 中缝顶部短横
    draw.line(
        [(21 * SS, 34.5 * SS), (23 * SS, 34.5 * SS)],
        fill=255, width=width,
    )

    return img.resize((GRID, GRID), Image.LANCZOS)


def sample_dots(mask: Image.Image) -> list[tuple[int, int]]:
    """从 mask 采样出点坐标（0..43）。"""
    px = mask.load()
    dots = []
    for y in range(GRID):
        for x in range(GRID):
            if px[x, y] > THRESHOLD:
                dots.append((x, y))
    return dots


def write_dot_matrix(dots: list[tuple[int, int]]) -> None:
    out = ROOT / "frontend/src/components/brand/mjDotMatrix.ts"
    lines = [
        "/**",
        " * MJ Nexus Reader 点阵品牌数据（由 scripts/generate-brand-assets.py 生成，勿手改）。",
        " * 44x44 网格：MJ 字母 + 平放字典的 6 层页口。",
        " * 静态坐标避免运行时 canvas 采样，启动动画与静态 logo 共用同一数据源。",
        " */",
        "export const DOT_GRID = 44;",
        "",
        "export const MJ_DOTS: ReadonlyArray<readonly [number, number]> = [",
    ]
    for x, y in dots:
        lines.append(f"  [{x}, {y}],")
    lines += ["] as const;", "", f"export const MJ_DOT_COUNT = {len(dots)};", ""]
    out.write_text("\n".join(lines), encoding="utf-8")
    print(f"  TS 数据: {out.relative_to(ROOT)} ({len(dots)} 点)")


def render_png(dots, size: int, bg, dot_color, path: Path, transparent=False) -> None:
    """把点阵渲染到指定尺寸 PNG。transparent=True 用于 Android adaptive foreground。"""
    scale = 4 if size <= 64 else 2
    s = size * scale
    # Tauri 要求所有图标为 RGBA（含 alpha 通道），即使背景不透明也必须存 RGBA
    mode = "RGBA"
    if transparent:
        img = Image.new(mode, (s, s), (0, 0, 0, 0))
        fill = dot_color if isinstance(dot_color, tuple) and len(dot_color) == 4 else (*dot_color, 255)
    else:
        img = Image.new(mode, (s, s), (*bg, 255) if len(bg) == 3 else bg)
        fill = (*dot_color, 255) if len(dot_color) == 3 else dot_color
    draw = ImageDraw.Draw(img)
    pad = s * 0.10
    step = (s - 2 * pad) / (GRID - 1)
    radius = max(1.0, step * 0.42)
    for x, y in dots:
        cx = pad + x * step
        cy = pad + y * step
        draw.ellipse(
            [cx - radius, cy - radius, cx + radius, cy + radius],
            fill=fill,
        )
    img = img.resize((size, size), Image.LANCZOS)
    if not path.parent.is_dir():
        path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)


# ---- 平台图标尺寸表 -------------------------------------------------------
ROOT_ICONS = [
    ("icon.png", 512),
    ("32x32.png", 32),
    ("64x64.png", 64),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("icon_source.png", 1024),
    ("StoreLogo.png", 50),
    ("Square30x30Logo.png", 30),
    ("Square44x44Logo.png", 44),
    ("Square71x71Logo.png", 71),
    ("Square89x89Logo.png", 89),
    ("Square107x107Logo.png", 107),
    ("Square142x142Logo.png", 142),
    ("Square150x150Logo.png", 150),
    ("Square284x284Logo.png", 284),
    ("Square310x310Logo.png", 310),
]
ANDROID_DENSITIES = {
    "mipmap-mdpi": (48, 108),
    "mipmap-hdpi": (49, 162),
    "mipmap-xhdpi": (96, 216),
    "mipmap-xxhdpi": (144, 324),
    "mipmap-xxxhdpi": (192, 432),
}
IOS_ICONS = {
    "AppIcon-20x20@1x.png": 20,
    "AppIcon-20x20@2x.png": 40,
    "AppIcon-20x20@2x-1.png": 40,
    "AppIcon-20x20@3x.png": 60,
    "AppIcon-29x29@1x.png": 29,
    "AppIcon-29x29@2x.png": 58,
    "AppIcon-29x29@2x-1.png": 58,
    "AppIcon-29x29@3x.png": 87,
    "AppIcon-40x40@1x.png": 40,
    "AppIcon-40x40@2x.png": 80,
    "AppIcon-40x40@2x-1.png": 80,
    "AppIcon-40x40@3x.png": 120,
    "AppIcon-60x60@2x.png": 120,
    "AppIcon-60x60@3x.png": 180,
    "AppIcon-76x76@1x.png": 76,
    "AppIcon-76x76@2x.png": 152,
    "AppIcon-83.5x83.5@2x.png": 167,
    "AppIcon-512@2x.png": 1024,
}


def generate_platform_icons(dots) -> None:
    """从点阵数据生成全平台图标矩阵（黑白主版：白底黑点）。"""
    icons_root = ROOT / "src-tauri/icons"
    count = 0

    print("  Tauri 根目录图标 ...")
    for name, size in ROOT_ICONS:
        render_png(dots, size, WHITE, BLACK, icons_root / name)
        count += 1

    # Windows ICO：直接从点阵渲染多尺寸帧
    ico = icons_root / "icon.ico"
    ico_frames = []
    for size in (256, 128, 64, 32):
        tmp = icons_root / f".ico-{size}.tmp.png"
        render_png(dots, size, WHITE, BLACK, tmp)
        ico_frames.append(Image.open(tmp).convert("RGBA"))
        tmp.unlink()
    ico_frames[0].save(ico, format="ICO", sizes=[(256, 256), (128, 128), (64, 64), (32, 32)])
    count += 1
    render_png(dots, 1024, WHITE, BLACK, icons_root / "icon.icns.tmp.png")
    icns_tmp = icons_root / "icon.icns.tmp.png"
    try:
        Image.open(icns_tmp).save(icons_root / "icon.icns", format="ICNS")
        count += 1
    except Exception as exc:  # ICNS 非关键，失败不阻塞
        print(f"    警告: ICNS 生成失败: {exc}", file=sys.stderr)
    icns_tmp.unlink()

    print("  Android mipmap ...")
    for density, (launcher_size, fg_size) in ANDROID_DENSITIES.items():
        for base in (icons_root / "android", ROOT / "src-tauri/gen/android/app/src/main/res"):
            for suffix in ("ic_launcher.png", "ic_launcher_round.png"):
                render_png(dots, launcher_size, WHITE, BLACK, base / density / suffix)
                count += 1
            render_png(dots, fg_size, WHITE, BLACK, base / density / "ic_launcher_foreground.png")
            count += 1

    print("  iOS AppIcon ...")
    for base in (icons_root / "ios", ROOT / "src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset"):
        for name, size in IOS_ICONS.items():
            render_png(dots, size, WHITE, BLACK, base / name)
            count += 1

    print(f"  共生成 {count} 个平台图标")


def svg_circles(dots) -> str:
    return "\n".join(
        f'    <circle cx="{x}" cy="{y}" r="0.42" />' for x, y in dots
    )


def write_favicon(dots) -> None:
    """白底黑点 favicon（浏览器标签默认浅色）。"""
    out = ROOT / "frontend/public/favicon.svg"
    content = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 44 44">\n'
        '  <rect width="44" height="44" rx="9" fill="#ffffff" />\n'
        '  <g fill="#000000">\n'
        f'{svg_circles(dots)}\n'
        '  </g>\n'
        '</svg>\n'
    )
    out.write_text(content, encoding="utf-8")
    print(f"  favicon: {out.relative_to(ROOT)}")


def write_preview(dots) -> None:
    """生成两张检查用预览图。"""
    out_dir = ROOT / "deliverables"
    if not out_dir.is_dir():
        out_dir.mkdir()
    render_png(dots, 512, WHITE, BLACK, out_dir / "mj-logo-preview-light.png")
    render_png(dots, 512, BLACK, WHITE, out_dir / "mj-logo-preview-dark.png")
    print(f"  预览图: {out_dir}/mj-logo-preview-{{light,dark}}.png")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--preview-only", action="store_true", help="只生成点数据与预览图，不写平台图标"
    )
    parser.add_argument(
        "--icons", action="store_true", help="生成全平台图标矩阵（含 gen 副本）"
    )
    args = parser.parse_args()

    print("生成点阵 mask ...")
    mask = render_mask()
    dots = sample_dots(mask)
    if not dots:
        print("错误：点阵为空，检查字体路径", file=sys.stderr)
        return 1

    print(f"采样到 {len(dots)} 个点")
    write_dot_matrix(dots)
    write_favicon(dots)
    write_preview(dots)

    if args.preview_only:
        print("预览模式完成。")
        return 0

    if args.icons:
        generate_platform_icons(dots)
        print("平台图标生成完成。")
        return 0

    print("提示：加 --icons 生成全平台图标矩阵。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
