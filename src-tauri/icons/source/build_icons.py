"""Regenerate the app icons from the "01c · Song List" logo (ROADMAP item 13).

Writes three tile SVGs (full mark, 64 px variant, 32 px variant), renders each
at the sizes it was designed for with `npx tauri icon --png`, and packs
icon.ico from those renders so small sizes use the hand-tuned variants.

Run from the repo root:  python src-tauri/icons/source/build_icons.py
"""

import io
import os
import struct
import subprocess
import tempfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ICONS = os.path.dirname(HERE)

# The marks, verbatim from the design canvas (viewBox 0 0 100 100).
MARK_FULL = """<g transform="skewX(-12) translate(10 0)">
<rect x="8" y="40" width="86" height="20" rx="10" fill="#2ED9FF" fill-opacity="0.18" stroke="#2ED9FF" stroke-width="2"/>
<circle cx="20" cy="16" r="5.5" fill="#17E289"/><rect x="32" y="12.5" width="50" height="7" rx="3.5" fill="#C7E0FF"/>
<circle cx="20" cy="33" r="5.5" fill="#F32B37"/><rect x="32" y="29.5" width="38" height="7" rx="3.5" fill="#C7E0FF"/>
<circle cx="20" cy="50" r="5.5" fill="#FFBB0D"/><rect x="32" y="46.5" width="54" height="7" rx="3.5" fill="#FFFFFF"/>
<circle cx="20" cy="67" r="5.5" fill="#3784F9"/><rect x="32" y="63.5" width="30" height="7" rx="3.5" fill="#C7E0FF"/>
<circle cx="20" cy="84" r="5.5" fill="#FF8413"/><rect x="32" y="80.5" width="44" height="7" rx="3.5" fill="#C7E0FF"/>
</g>"""

MARK_64 = """<g transform="skewX(-12) translate(10 0)">
<rect x="6" y="36" width="88" height="28" rx="14" fill="#2ED9FF" fill-opacity="0.2" stroke="#2ED9FF" stroke-width="3"/>
<circle cx="20" cy="17" r="8" fill="#17E289"/><rect x="34" y="12" width="46" height="10" rx="5" fill="#C7E0FF"/>
<circle cx="20" cy="50" r="8" fill="#FFBB0D"/><rect x="34" y="45" width="52" height="10" rx="5" fill="#FFFFFF"/>
<circle cx="20" cy="83" r="8" fill="#FF8413"/><rect x="34" y="78" width="36" height="10" rx="5" fill="#C7E0FF"/>
</g>"""

MARK_32 = """<g transform="skewX(-12) translate(10 0)">
<circle cx="20" cy="17" r="11" fill="#17E289"/><rect x="36" y="10" width="46" height="14" rx="7" fill="#C7E0FF"/>
<circle cx="20" cy="50" r="11" fill="#FFBB0D"/><rect x="36" y="43" width="52" height="14" rx="7" fill="#2ED9FF"/>
<circle cx="20" cy="83" r="11" fill="#FF8413"/><rect x="36" y="76" width="36" height="14" rx="7" fill="#C7E0FF"/>
</g>"""

# (name, mark, design tile px, mark px, corner radius px) from the canvas.
VARIANTS = [
    ("full", MARK_FULL, 160, 112, 36),
    ("64", MARK_64, 64, 46, 14),
    ("32", MARK_32, 32, 26, 7),
]


def tile_svg(mark, tile, mark_px, radius):
    """A 1024 px app-icon tile: #0E1626 rounded square, 1 px #2A3A55 border."""
    k = 1024 / tile
    border = 1 * k
    m = mark_px * k
    off = (1024 - m) / 2
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
<rect x="{border / 2}" y="{border / 2}" width="{1024 - border}" height="{1024 - border}" rx="{radius * k}" fill="#0E1626" stroke="#2A3A55" stroke-width="{border}"/>
<svg x="{off}" y="{off}" width="{m}" height="{m}" viewBox="0 0 100 100">
{mark}
</svg>
</svg>
"""


def write_ico(path, images):
    """Write an .ico with PNG-compressed entries, in the given order."""
    blobs = []
    for im in images:
        buf = io.BytesIO()
        im.save(buf, format="PNG")
        blobs.append(buf.getvalue())
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    for im, blob in zip(images, blobs):
        w, h = im.size
        entries += struct.pack(
            "<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(blob), offset
        )
        offset += len(blob)
    with open(path, "wb") as f:
        f.write(header + entries + b"".join(blobs))


def render(svg_path, sizes, out_dir):
    subprocess.run(
        ["npx", "tauri", "icon", svg_path, "-o", out_dir, "-p", ",".join(map(str, sizes))],
        check=True,
        shell=os.name == "nt",
    )
    return {s: Image.open(os.path.join(out_dir, f"{s}x{s}.png")).convert("RGBA") for s in sizes}


def main():
    svgs = {}
    for name, mark, tile, mark_px, radius in VARIANTS:
        path = os.path.join(HERE, f"icon-{name}.svg")
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(tile_svg(mark, tile, mark_px, radius))
        svgs[name] = path

    with tempfile.TemporaryDirectory() as tmp:
        small = render(svgs["32"], [16, 24, 32], os.path.join(tmp, "s"))
        mid = render(svgs["64"], [48, 64], os.path.join(tmp, "m"))
        big = render(svgs["full"], [128, 256, 512], os.path.join(tmp, "b"))

        small[32].save(os.path.join(ICONS, "32x32.png"))
        big[128].save(os.path.join(ICONS, "128x128.png"))
        big[512].save(os.path.join(ICONS, "icon.png"))

        # Tauri uses only the FIRST .ico entry as the default window/taskbar
        # icon (tauri-codegen image.rs: entries()[0]), so the largest goes first.
        # (Pillow re-sorts ICO entries by size, hence write_ico.)
        write_ico(
            os.path.join(ICONS, "icon.ico"),
            [big[256], small[16], small[24], small[32], mid[48], mid[64]],
        )


    print("icons written to", ICONS)


if __name__ == "__main__":
    main()
