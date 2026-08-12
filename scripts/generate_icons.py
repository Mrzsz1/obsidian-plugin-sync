"""Generate indigo brand icons for Obsidian Plugin Sync."""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
OUT_PUBLIC = ROOT / "public" / "icon.png"
OUT_ICONS = ROOT / "src-tauri" / "icons"


def make_icon(size: int) -> Image.Image:
    base = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    # Vertical indigo gradient fill.
    pixels = base.load()
    for y in range(size):
        t = y / max(1, size - 1)
        r = int(99 + (79 - 99) * t)
        g = int(102 + (70 - 102) * t)
        b = int(241 + (229 - 241) * t)
        for x in range(size):
            pixels[x, y] = (r, g, b, 255)

    margin = max(1, size // 16)
    radius = max(4, size // 5)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [margin, margin, size - 1 - margin, size - 1 - margin],
        radius=radius,
        fill=255,
    )
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    img.paste(base, (0, 0), mask)
    draw = ImageDraw.Draw(img)

    cx = cy = size / 2
    stroke = max(2, size // 10)
    ring = size * 0.28
    bbox_top = [cx - ring, cy - ring * 0.9, cx + ring, cy + ring * 0.5]
    bbox_bot = [cx - ring, cy - ring * 0.5, cx + ring, cy + ring * 0.9]
    draw.arc(bbox_top, start=200, end=30, fill=(255, 255, 255, 255), width=stroke)
    draw.arc(bbox_bot, start=20, end=210, fill=(255, 255, 255, 255), width=stroke)

    def arrow(angle_deg: float, direction: int) -> None:
        a = math.radians(angle_deg)
        x = cx + ring * math.cos(a)
        y = cy + ring * 0.75 * math.sin(a)
        s = max(3, size // 9)
        if direction > 0:
            pts = [(x - s * 0.2, y - s), (x + s, y), (x - s * 0.2, y + s * 0.35)]
        else:
            pts = [(x + s * 0.2, y + s), (x - s, y), (x + s * 0.2, y - s * 0.35)]
        draw.polygon(pts, fill=(255, 255, 255, 255))

    arrow(25, 1)
    arrow(205, -1)
    return img


def save_png(path: Path, size: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    make_icon(size).save(path, "PNG")
    print(f"wrote {path} ({size}px)")


def main() -> None:
    save_png(OUT_PUBLIC, 256)
    save_png(OUT_ICONS / "32x32.png", 32)
    save_png(OUT_ICONS / "128x128.png", 128)
    save_png(OUT_ICONS / "128x128@2x.png", 256)
    save_png(OUT_ICONS / "icon.png", 256)

    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [make_icon(s) for s in ico_sizes]
    ico_path = OUT_ICONS / "icon.ico"
    images[0].save(
        ico_path,
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
        append_images=images[1:],
    )
    print(f"wrote {ico_path}")


if __name__ == "__main__":
    main()
