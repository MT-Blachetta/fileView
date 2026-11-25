#!/usr/bin/env python3
"""Writes assets/planar.ico, the Windows executable and installer icon.

Draws the same shapes as `theme::app_icon()` (a 64x64 design), supersampled
at every size Windows asks for. Uses only the standard library:
    python scripts/make-icon.py
"""
import os
import struct
import zlib

SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
BLUE = (0x2F, 0x6F, 0xEB)
WHITE = (255, 255, 255)


def inside_round(x, y, x0, y0, x1, y1, r):
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return x0 <= x <= x1 and y0 <= y <= y1 and (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def outline(x, y, x0, y0, x1, y1):
    return inside_round(x, y, x0, y0, x1, y1, 3.0) and not inside_round(x, y, x0 + 3.0, y0 + 3.0, x1 - 3.0, y1 - 3.0, 1.0)


def sample(x, y):
    """Colour at a point of the 64x64 design, or None outside the icon."""
    if not inside_round(x, y, 2.0, 2.0, 62.0, 62.0, 14.0):
        return None
    if outline(x, y, 13.0, 16.0, 51.0, 49.0) or outline(x, y, 30.0, 30.0, 45.0, 43.0):
        return WHITE
    if inside_round(x, y, 13.0, 11.0, 28.0, 18.0, 2.0) or inside_round(x, y, 17.0, 30.0, 25.0, 38.0, 1.5):
        return WHITE
    return BLUE


def render(size):
    ss = 8 if size <= 64 else 4
    scale = 64.0 / size
    rows = []
    for py in range(size):
        row = bytearray([0])  # PNG filter type: none
        for px in range(size):
            r = g = b = a = 0
            for sy in range(ss):
                for sx in range(ss):
                    c = sample((px + (sx + 0.5) / ss) * scale, (py + (sy + 0.5) / ss) * scale)
                    if c:
                        r += c[0]
                        g += c[1]
                        b += c[2]
                        a += 1
            if a:
                row += bytes([round(r / a), round(g / a), round(b / a), round(255 * a / (ss * ss))])
            else:
                row += bytes(4)
        rows.append(bytes(row))
    return png(size, b"".join(rows))


def png(size, raw):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main():
    images = [render(s) for s in SIZES]
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in zip(SIZES, images):
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out += b"".join(images)
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "planar.ico")
    with open(path, "wb") as f:
        f.write(out)
    print(f"wrote {os.path.normpath(path)} ({len(out)} bytes)")


if __name__ == "__main__":
    main()
