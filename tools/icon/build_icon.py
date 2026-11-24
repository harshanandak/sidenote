"""Rasterise the app icon SVGs, pack the multi-size .ico and render the concept preview sheet.

    uv run --no-project --with pillow==13.0.0 --with resvg-py==0.5.0 python tools/icon/build_icon.py --assets-only

The ink icon uses the same curves at every size, with optical adjustments at 40 px and below.
Older concepts fall back to their small SVG up to SMALL_MAX and tiny SVG at 16 px.
Outputs: assets/icon/png/<size>.png and assets/icon/meetings.ico for the chosen concept.
Omit --assets-only to also write the concept preview sheets.
"""
import argparse
import io
import struct
from pathlib import Path
from xml.etree import ElementTree

import resvg_py
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
ICON_DIR = ROOT / "assets" / "icon"
OUT_DIR = ROOT / "artifacts" / "icon"
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SMALL_MAX = 32
TINY_MAX = 16  # pixel-grid aligned drawing, where one exists
INK_BASELINE = {16: 10, 20: 12, 24: 14, 32: 17, 40: 22}
CONCEPTS = {
    "sidenote-d3c-final-ink": "Sidenote  Ink",
    "concept-a-summary": "A  Summary bubble",
    "concept-b-check": "B  Bubble with check",
    "concept-c-card": "C  Meeting card",
}
DEFAULT_CONCEPT = "sidenote-d3c-final-ink"
LIGHT_BG, DARK_BG = (243, 243, 243), (32, 32, 32)


def small_ink(size: int) -> str:
    """Fill the small icon box and keep the light stroke and amber tip legible."""
    svg = ElementTree.parse(ICON_DIR / f"{DEFAULT_CONCEPT}.svg").getroot()
    parts = {part.get("id"): part for part in svg}
    frame = parts["frame"]
    x, y, side, radius = (float(frame.attrib[key]) for key in ("x", "y", "width", "rx"))
    svg.set("viewBox", f"{x:g} {y:g} {side:g} {side:g}")
    units_per_px = side / size
    inset = max(8.0, units_per_px)
    parts["face"].attrib.update({
        "x": f"{x + inset:g}",
        "y": f"{y + inset:g}",
        "width": f"{side - 2 * inset:g}",
        "height": f"{side - 2 * inset:g}",
        "rx": f"{radius - inset:g}",
    })
    if size < 32:
        # The master's rebound crowds the dip at tiny sizes.
        start = 66 if size == 16 else 57
        parts["ink"].set(
            "d",
            f"M{start} 138 C{start} 105 68 65 83 65 C101 65 96 168 114 168 C131 168 139 138 154 138 H194",
        )
        parts["cap"].set("cx", str(start))
    stroke = 2.0 * units_per_px
    parts["ink"].set("stroke-width", f"{stroke:g}")
    parts["cap"].set("r", f"{stroke / 2:g}")
    baseline = y + INK_BASELINE[size] * units_per_px
    mark = f"translate(194 {baseline:g}) scale(1.1 1) translate(-194 -138)"
    parts["ink"].set("transform", mark)
    parts["cap"].set("transform", mark)
    tip = stroke / 16.0
    parts["tip"].set("transform", f"{mark} translate(194 138) scale({max(1.0, tip):g} {tip:g}) translate(-194 -138)")
    ElementTree.register_namespace("", "http://www.w3.org/2000/svg")
    return ElementTree.tostring(svg, encoding="unicode")


def render(concept: str, size: int) -> Image.Image:
    name = f"{concept}.svg"
    if concept != DEFAULT_CONCEPT and size <= SMALL_MAX:
        name = f"{concept}-small.svg"
        if size <= TINY_MAX and (ICON_DIR / f"{concept}-tiny.svg").exists():
            name = f"{concept}-tiny.svg"
    if concept == DEFAULT_CONCEPT and size in INK_BASELINE:
        png = resvg_py.svg_to_bytes(svg_string=small_ink(size), width=size, height=size)
    else:
        png = resvg_py.svg_to_bytes(svg_path=str(ICON_DIR / name), width=size, height=size)
    return Image.open(io.BytesIO(bytes(png))).convert("RGBA")


def png_bytes(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def write_ico(path: Path, images: dict[int, Image.Image]) -> None:
    """PNG-in-ICO, one entry per size (Windows Vista and later). Pillow would resample one image."""
    sizes = sorted(images)
    blobs = [png_bytes(images[s]) for s in sizes]
    out = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    offset = 6 + 16 * len(sizes)
    for s, blob in zip(sizes, blobs):
        dim = 0 if s >= 256 else s
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    for blob in blobs:
        out += blob
    path.write_bytes(bytes(out))


def font(size: int) -> ImageFont.ImageFont:
    try:
        return ImageFont.truetype("segoeui.ttf", size)
    except OSError:
        return ImageFont.load_default(size)


def sheet(concepts: list[str], sizes: list[int], zoom: int = 1, title: str = "") -> Image.Image:
    """Concept rows by size columns, once on a light panel and once on a dark one."""
    pad, label_w, head = 20, 190, 44
    cols = [s * zoom for s in sizes]
    row_h = max(cols) + pad * 2
    panel_w = label_w + sum(c + pad * 2 for c in cols)
    img = Image.new("RGB", (panel_w * 2, head + row_h * len(concepts)), LIGHT_BG)
    d = ImageDraw.Draw(img)
    dark = Image.new("RGB", (panel_w, img.height - 0), DARK_BG)
    img.paste(dark, (panel_w, 0))
    for p, (bg, fg) in enumerate([(LIGHT_BG, (40, 40, 40)), (DARK_BG, (225, 225, 225))]):
        x0 = p * panel_w
        d.text((x0 + pad, 10), title or ("Light taskbar" if p == 0 else "Dark taskbar"), font=font(20), fill=fg)
        x = x0 + label_w
        for s, c in zip(sizes, cols):
            d.text((x + pad, head - 22), f"{s}px", font=font(14), fill=fg)
            x += c + pad * 2
        for r, name in enumerate(concepts):
            y = head + r * row_h
            if p == 0:
                d.text((x0 + pad, y + row_h // 2 - 10), CONCEPTS.get(name, name), font=font(16), fill=fg)
            x = x0 + label_w
            for s, c in zip(sizes, cols):
                tile = render(name, s)
                if zoom > 1:
                    tile = tile.resize((c, c), Image.NEAREST)
                img.paste(tile, (x + pad, y + (row_h - c) // 2), tile)
                x += c + pad * 2
    return img


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--concept", default=DEFAULT_CONCEPT, choices=list(CONCEPTS))
    ap.add_argument("--assets-only", action="store_true")
    args = ap.parse_args()

    (ICON_DIR / "png").mkdir(exist_ok=True)
    images = {s: render(args.concept, s) for s in SIZES}
    for s, im in images.items():
        im.save(ICON_DIR / "png" / f"{s}.png", optimize=True)
    write_ico(ICON_DIR / "meetings.ico", images)

    if not args.assets_only:
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        names = list(CONCEPTS)
        sheet(names, [16, 32, 48, 256]).save(OUT_DIR / "preview.png")
        sheet(names, [16, 20, 24, 32], zoom=6, title="Small sizes, 6x nearest").save(OUT_DIR / "preview-zoom.png")
    print(f"wrote {ICON_DIR / 'meetings.ico'} ({len(SIZES)} sizes) from {args.concept}")


if __name__ == "__main__":
    main()
