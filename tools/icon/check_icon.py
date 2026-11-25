"""Check the shipped ICO embeds every original 32-bit PNG at its own size."""

from pathlib import Path
from struct import Struct


ICON_DIR = Path(__file__).resolve().parents[2] / "assets" / "icon"
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
HEADER = Struct("<HHH")
ENTRY = Struct("<BBBBHHII")
PNG_HEADER = Struct(">8sI4sIIBB")


def main():
    ico = (ICON_DIR / "meetings.ico").read_bytes()
    assert HEADER.unpack_from(ico) == (0, 1, len(SIZES)), "wrong ICO header"
    expected_offset = HEADER.size + len(SIZES) * ENTRY.size
    for index, size in enumerate(SIZES):
        entry = ENTRY.unpack_from(ico, HEADER.size + index * ENTRY.size)
        width, height, colors, reserved, planes, depth, length, offset = entry
        dimension = size % 256
        assert (width, height, colors, reserved, planes, depth) == (
            dimension, dimension, 0, 0, 1, 32
        ), f"wrong {size}px ICO entry"
        assert offset == expected_offset, f"wrong {size}px payload offset"
        end = offset + length
        png = (ICON_DIR / "png" / f"{size}.png").read_bytes()
        assert ico[offset:end] == png, f"{size}px PNG differs from ICO"
        assert PNG_HEADER.unpack_from(png) == (
            b"\x89PNG\r\n\x1a\n", 13, b"IHDR", size, size, 8, 6
        ), f"wrong {size}px RGBA PNG"
        expected_offset = end
    assert expected_offset == len(ico), "unexpected bytes after the last ICO image"
    print("Sidenote ICO has nine exact RGBA PNG frames")


if __name__ == "__main__":
    main()
