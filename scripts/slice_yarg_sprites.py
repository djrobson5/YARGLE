"""Slice YARG's sprite sheets into individual PNGs under public/icons/yarg/.

Reads each sheet's Unity `.meta` (spriteSheet.sprites: name + rect), flips the
rect's Y (Unity rects use a bottom-left origin), crops, and downsizes.

Run from the repo root:
    python scripts/slice_yarg_sprites.py [path/to/YARG-fork]
(defaults to ../YARG-fork next to this repo)
"""

import os
import re
import sys

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
YARG = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(ROOT), "YARG-fork")
ART = os.path.join(YARG, "Assets", "Art")
OUT = os.path.join(ROOT, "public", "icons", "yarg")

# (sheet path under Assets/Art, output folder, output px)
SHEETS = [
    ("Menu/Common/InstrumentIcons.png", "instruments", 128),
    ("Menu/Common/DifficultyIcons.png", "difficulty", 128),
]

SPRITE_RE = re.compile(
    r"- serializedVersion: 2\s+name: (?P<name>\S+)\s+rect:\s+serializedVersion: 2\s+"
    r"x: (?P<x>[\d.]+)\s+y: (?P<y>[\d.]+)\s+width: (?P<w>[\d.]+)\s+height: (?P<h>[\d.]+)"
)


def slice_sheet(rel, folder, px):
    png = os.path.join(ART, *rel.split("/"))
    meta = open(png + ".meta", encoding="utf-8").read()
    sheet = Image.open(png).convert("RGBA")
    out_dir = os.path.join(OUT, folder)
    os.makedirs(out_dir, exist_ok=True)
    names = []
    for m in SPRITE_RE.finditer(meta):
        x, y, w, h = (round(float(m.group(k))) for k in "xywh")
        top = sheet.height - y - h  # Unity's origin is bottom-left
        sprite = sheet.crop((x, top, x + w, top + h))
        scale = px / max(w, h)
        sprite = sprite.resize((max(1, round(w * scale)), max(1, round(h * scale))), Image.LANCZOS)
        sprite.save(os.path.join(out_dir, m.group("name") + ".png"), optimize=True)
        names.append(m.group("name"))
    print(f"{folder}: {len(names)} sprites -> {', '.join(names)}")


def main():
    if not os.path.isdir(ART):
        sys.exit(f"YARG source not found at {YARG}")
    for rel, folder, px in SHEETS:
        slice_sheet(rel, folder, px)


if __name__ == "__main__":
    main()
