"""Render an engraving as a blue dot-matrix image (ordered dither), for the home hero.

Brightness is averaged per cell, then each cell gets a square dot whose shade comes
from a 4x4 Bayer threshold, so dot density follows the image; a soft blue glow sits
behind the dots. Output is RGBA WebP (lossless) with transparent background.

Usage (from packages/frontend):
  uv run --with pillow --with numpy python scripts/stealth/dither_art.py \
    public/assets/v2/observatory.webp public/assets/v2/observatory-blue.webp --width 2480 --height 1396
  uv run --with pillow --with numpy python scripts/stealth/dither_art.py \
    public/assets/v2/observatory.webp public/assets/v2/observatory-blue-960.webp --width 1364 --height 768

Cells are 12 px (6 CSS px at 2x); dots are 5 px.
"""
import argparse

import numpy as np
from PIL import Image, ImageFilter

BAYER4 = (np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]) + 0.5) / 16.0
PALETTE = ["#16408f", "#2a63c9", "#4f8ae6", "#9cc0f5"]


def rgb(hex_color: str) -> np.ndarray:
    h = hex_color.lstrip("#")
    return np.array([int(h[i : i + 2], 16) for i in (0, 2, 4)], dtype=float)


def render(src: Image.Image, out_w: int, out_h: int, cell: int = 12, dot: int = 5, gain: float = 1.15,
           glow: float = 0.75, glow_hi: str = "#123a8a", grad: float = 0.18, gamma: float = 0.85) -> Image.Image:
    cols, rows = out_w // cell, out_h // cell
    lum = np.array(src.convert("L").resize((cols, rows), Image.BOX)).astype(float) / 255.0
    lo, hi = np.percentile(lum, 5), np.percentile(lum, 99.5)
    lum = np.clip((lum - lo) / max(1e-6, hi - lo), 0, 1) ** gamma
    y = np.linspace(0, 1, rows)[:, None]
    field = np.clip(gain * lum + grad * y * lum, 0, 1)

    pal = [rgb(c) for c in PALETTE]
    thresholds = np.tile(BAYER4, (rows // 4 + 1, cols // 4 + 1))[:rows, :cols]
    level = np.clip(np.floor(field * len(pal) + thresholds).astype(int), 0, len(pal))

    blurred = Image.fromarray((field * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(2.2))
    soft = np.array(blurred.resize((cols * cell, rows * cell), Image.BICUBIC)).astype(float) / 255.0
    img = np.zeros((rows * cell, cols * cell, 4), dtype=float)
    img[..., :3] = rgb(glow_hi) * soft[..., None]
    img[..., 3] = np.clip(glow * soft * 1.3, 0, 1)

    off = (cell - dot) // 2
    for r in range(rows):
        for c in range(cols):
            lv = level[r, c]
            if lv > 0:
                y0, x0 = r * cell + off, c * cell + off
                img[y0 : y0 + dot, x0 : x0 + dot, :3] = pal[lv - 1]
                img[y0 : y0 + dot, x0 : x0 + dot, 3] = 1.0

    out = np.dstack([np.clip(img[..., :3], 0, 255), np.clip(img[..., 3] * 255, 0, 255)]).astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("source")
    parser.add_argument("output")
    parser.add_argument("--width", type=int, required=True)
    parser.add_argument("--height", type=int, required=True)
    args = parser.parse_args()
    render(Image.open(args.source), args.width, args.height).save(args.output, "WEBP", lossless=True, method=6)


if __name__ == "__main__":
    main()
