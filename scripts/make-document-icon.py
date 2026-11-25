#!/usr/bin/env python3
"""Draws the Finder icon for this app's project files: a white page with a folded corner, the app's own
icon on it, and the file extension underneath, as macOS draws documents of other apps. Writes
assets/icon/<extension>-document.icns (bundled into the app and named in Info.plist).

The three Omina apps each keep a copy of this script, set by the constants below; only those differ.
Needs Pillow and macOS's iconutil:

    python3 scripts/make-document-icon.py
"""

import pathlib
import shutil
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
APP_ICON = ROOT / "assets" / "icon" / "studio-512@2x.png"
EXTENSION = "omim"
# The band the extension sits on, in the app icon's accent colour, and the extension's colour on it.
ACCENT = (138, 0, 255)
LABEL = (255, 255, 255)

SIZE = 1024
# Drawn four times larger, then scaled down, for smooth edges.
SCALE = 4


def draw_page():
    """The page: Apple's document proportions (about 3:4 inside the square), a folded top-right corner
    and a soft shadow, with the app icon and the extension band."""
    big = SIZE * SCALE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    width, height = 0.70 * big, 0.90 * big
    left, top = (big - width) / 2, (big - height) / 2
    right, bottom = left + width, top + height
    fold = 0.22 * width
    radius = 0.035 * big

    # The page's shape: a rounded rectangle with its top-right corner cut off where it folds.
    shape = Image.new("L", (big, big), 0)
    shape_draw = ImageDraw.Draw(shape)
    shape_draw.rounded_rectangle((left, top, right, bottom), radius, fill=255)
    shape_draw.polygon([(right - fold, top - 1), (right + 1, top - 1), (right + 1, top + fold)], fill=0)

    # A soft shadow of that same shape just below it, then the page, its edge, and the fold.
    shadow = shape.filter(ImageFilter.GaussianBlur(SCALE * 6)).point(lambda value: value * 60 // 255)
    image.paste((0, 0, 0, 255), (0, int(0.01 * big)), shadow)
    edge = shape.filter(ImageFilter.MaxFilter(SCALE * 2 + 1))
    image.paste((196, 196, 202, 255), (0, 0), edge)
    image.paste((252, 252, 250, 255), (0, 0), shape)
    draw.polygon([(right - fold, top), (right - fold, top + fold), (right, top + fold)], fill=(214, 214, 220, 255))

    # The app icon, filling most of the page's width, in its upper part.
    # The app icon's tile without the clear margin around it (Apple's grid: the tile is 824 of 1024 pixels).
    icon = Image.open(APP_ICON).convert("RGBA")
    margin = icon.width * 100 // 1024
    icon = icon.crop((margin, margin, icon.width - margin, icon.height - margin))
    side = int(0.62 * width)
    icon = icon.resize((side, side), Image.LANCZOS)
    image.alpha_composite(icon, (int(left + (width - side) / 2), int(top + 0.16 * height)))

    # The extension on a band of the app's colour along the bottom.
    band_top = bottom - 0.20 * height
    draw.rectangle((left + SCALE * 2, band_top, right - SCALE * 2, bottom - radius), fill=ACCENT + (255,))
    draw.rounded_rectangle((left + SCALE * 2, bottom - 2 * radius, right - SCALE * 2, bottom - SCALE * 2), radius, fill=ACCENT + (255,))
    font = ImageFont.truetype("/System/Library/Fonts/SFNS.ttf", int(0.105 * height))
    try:
        font.set_variation_by_name("Bold")
    except (OSError, ValueError):
        pass
    label = EXTENSION.upper()
    box = draw.textbbox((0, 0), label, font=font)
    text_x = left + (width - (box[2] - box[0])) / 2 - box[0]
    text_y = band_top + (bottom - band_top - (box[3] - box[1])) / 2 - box[1]
    draw.text((text_x, text_y), label, font=font, fill=LABEL + (255,))
    return image.resize((SIZE, SIZE), Image.LANCZOS)


def main():
    page = draw_page()
    out = ROOT / "assets" / "icon" / f"{EXTENSION}-document.icns"
    with tempfile.TemporaryDirectory() as folder:
        iconset = pathlib.Path(folder) / "document.iconset"
        iconset.mkdir()
        # Every size macOS asks an .icns for, plain and @2x.
        for points in (16, 32, 128, 256, 512):
            page.resize((points, points), Image.LANCZOS).save(iconset / f"icon_{points}x{points}.png")
            page.resize((points * 2, points * 2), Image.LANCZOS).save(iconset / f"icon_{points}x{points}@2x.png")
        subprocess.run(["iconutil", "--convert", "icns", "--output", str(out), str(iconset)], check=True)
    preview = ROOT / "assets" / "icon" / f"{EXTENSION}-document.png"
    page.save(preview)
    print(f"Wrote {out.relative_to(ROOT)} and {preview.relative_to(ROOT)}")


if __name__ == "__main__":
    if shutil.which("iconutil") is None:
        raise SystemExit("iconutil isn't available: run this on a Mac.")
    main()
