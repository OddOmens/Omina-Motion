#!/usr/bin/env python3
"""Draws Studio's app icon at every size macOS uses, into assets/icon (studio-512@2x.png is the
1024-pixel original):
a violet tile (the editor's accent) with three stacked layer cards and a play triangle, laid out on
Apple's icon grid (the tile fills 824 of the 1024 pixels). Needs Pillow:

    python3 scripts/make-icon.py
"""

import pathlib

from PIL import Image, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "icon"
SIZE = 1024
# Drawn four times larger, then scaled down, for smooth edges.
SCALE = 4


def gradient(size, top, bottom):
    """A vertical gradient from `top` to `bottom` (RGB)."""
    image = Image.new("RGB", (size, size))
    pixels = image.load()
    for y in range(size):
        t = y / (size - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        for x in range(size):
            pixels[x, y] = color
    return image


def draw():
    big = SIZE * SCALE
    unit = lambda value: round(value * SCALE)
    icon = Image.new("RGBA", (big, big), (0, 0, 0, 0))

    # The tile: Apple's grid puts it 100 pixels in from each edge, with rounded corners.
    margin, radius = unit(100), unit(185)
    tile_mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(tile_mask).rounded_rectangle((margin, margin, big - margin, big - margin), radius, fill=255)

    # A soft shadow under the tile.
    shadow = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((margin, margin + unit(14), big - margin, big - margin + unit(14)), radius, fill=(0, 0, 0, 90))
    icon = Image.alpha_composite(icon, shadow.filter(ImageFilter.GaussianBlur(unit(16))))

    tile = gradient(big, (139, 92, 246), (67, 32, 160)).convert("RGBA")
    tile.putalpha(tile_mask)
    icon = Image.alpha_composite(icon, tile)

    # Three layer cards, stacked back to front, each a little lighter.
    cards = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    pen = ImageDraw.Draw(cards)
    for step, alpha in ((2, 70), (1, 130), (0, 255)):
        dx, dy = unit(54 * step), unit(-54 * step)
        box = (unit(250) + dx, unit(330) + dy, unit(690) + dx, unit(690) + dy)
        fill = (255, 255, 255, alpha) if step else (250, 248, 255, 255)
        pen.rounded_rectangle(box, unit(56), fill=fill)
    icon = Image.alpha_composite(icon, cards)

    # A play triangle on the front card.
    play = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    cx, cy, r = unit(470), unit(510), unit(105)
    ImageDraw.Draw(play).polygon([(cx - r * 0.55, cy - r), (cx - r * 0.55, cy + r), (cx + r, cy)], fill=(109, 64, 220, 255))
    icon = Image.alpha_composite(icon, play)

    return icon.resize((SIZE, SIZE), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    icon = draw()
    # macOS's icon set: each size at 1× and 2× (a 2× file is twice the pixels, named "@2x").
    for points in (512, 256, 128, 32, 16):
        for density, suffix in ((1, ""), (2, "@2x")):
            size = points * density
            picture = icon if size == SIZE else icon.resize((size, size), Image.LANCZOS)
            picture.save(OUT / f"studio-{points}{suffix}.png")
    print(f"Wrote icons to {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
