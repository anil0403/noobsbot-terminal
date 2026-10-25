#!/usr/bin/env python3
"""Builds noobsbot.icns from one square PNG (1024 px is best):

    python3 make-icns.py icon-1024.png noobsbot.icns

Why not `iconutil`: on macOS 27 it writes broken 16 and 32 px icons (ic04,
ic05), and System Settings then shows noise instead of the icon. Here the
16 and 32 px icons are written by hand (raw ARGB, PackBits per channel, as
in Apple's format); every other size is a PNG. Sizes are made with `sips`.
Needs only Python's standard library.
"""

import os
import struct
import subprocess
import sys
import tempfile
import zlib

# (type, pixels, format): ARGB = raw pixels, PNG = a PNG file.
ENTRIES = [
    (b"ic04", 16, "argb"),
    (b"ic05", 32, "argb"),
    (b"ic11", 32, "png"),  # 16@2x
    (b"ic12", 64, "png"),  # 32@2x
    (b"ic07", 128, "png"),
    (b"ic13", 256, "png"),  # 128@2x
    (b"ic08", 256, "png"),
    (b"ic14", 512, "png"),  # 256@2x
    (b"ic09", 512, "png"),
    (b"ic10", 1024, "png"),  # 512@2x
]


def resize(src, px, out):
    subprocess.run(["sips", "-z", str(px), str(px), "-s", "format", "png", src, "--out", out],
                   check=True, stdout=subprocess.DEVNULL)


def rgba_pixels(path):
    """RGBA bytes of an 8-bit, non-interlaced PNG (what sips writes)."""
    data = open(path, "rb").read()
    pos, idat = 8, b""
    while pos < len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    if depth != 8 or interlace or color not in (2, 6):
        sys.exit(f"{path}: need an 8-bit RGB or RGBA PNG")
    bpp = 4 if color == 6 else 3
    raw, stride = zlib.decompress(idat), w * bpp
    out, prev = bytearray(), bytearray(stride)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = line
        for x in range(w):
            px = line[x * bpp:(x + 1) * bpp]
            out += px if bpp == 4 else px + b"\xff"
    return out


def packbits(channel):
    """Apple's icon PackBits: 0-127 = n+1 literal bytes, 128-255 = byte repeated n-125 times."""
    out, i = bytearray(), 0
    while i < len(channel):
        run = 1
        while i + run < len(channel) and run < 130 and channel[i + run] == channel[i]:
            run += 1
        if run >= 3:
            out += bytes([run + 125, channel[i]])
            i += run
            continue
        start = i
        while i < len(channel) and i - start < 128:
            if i + 2 < len(channel) and channel[i] == channel[i + 1] == channel[i + 2]:
                break
            i += 1
        out += bytes([i - start - 1]) + channel[start:i]
    return out


def argb(path):
    rgba = rgba_pixels(path)
    planes = [rgba[k::4] for k in (3, 0, 1, 2)]  # A, R, G, B
    return b"ARGB" + b"".join(packbits(p) for p in planes)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, dest = sys.argv[1:]
    with tempfile.TemporaryDirectory() as tmp:
        parts = b""
        for kind, px, fmt in ENTRIES:
            png = os.path.join(tmp, f"{px}.png")
            if not os.path.exists(png):
                resize(src, px, png)
            body = argb(png) if fmt == "argb" else open(png, "rb").read()
            parts += kind + struct.pack(">I", len(body) + 8) + body
    with open(dest, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(parts) + 8) + parts)
    print(f"{dest}: {len(ENTRIES)} sizes, {len(parts) + 8} bytes")


if __name__ == "__main__":
    main()
