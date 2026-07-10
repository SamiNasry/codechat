#!/usr/bin/env python3
"""Generate the CodeChat icon set with zero dependencies (stdlib only).

Tauri refuses to build without the icon files listed in tauri.conf.json, so
this script renders a simple icon — a purple rounded square with a white chat
bubble — and writes every format Tauri needs:

    src-tauri/icons/32x32.png
    src-tauri/icons/128x128.png
    src-tauri/icons/128x128@2x.png   (256px)
    src-tauri/icons/icon.ico         (Windows/ICO, PNG-embedded)
    src-tauri/icons/icon.icns        (macOS, PNG-embedded)

The generated icons are committed to the repo, so you only rerun this if you
want to change the artwork:

    python3 scripts/gen_icons.py
"""

import math
import os
import struct
import zlib

OUT_DIR = os.path.normpath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")
)

# Twitch-ish purples for the background gradient, white bubble, purple dots.
GRAD_TOP = (145, 70, 255)     # #9146FF
GRAD_BOTTOM = (119, 44, 232)  # #772CE8
BUBBLE = (255, 255, 255)
DOT = (119, 44, 232)


# --- tiny RGBA PNG encoder ---------------------------------------------------

def encode_png(size, pixels):
    """pixels: list of rows, each row a list of (r, g, b, a) tuples."""
    raw = b"".join(
        b"\x00" + bytes(channel for px in row for channel in px) for row in pixels
    )

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(
            ">I", zlib.crc32(body) & 0xFFFFFFFF
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


# --- signed-distance shapes (negative = inside) -------------------------------

def sd_round_rect(x, y, cx, cy, hw, hh, r):
    qx = abs(x - cx) - (hw - r)
    qy = abs(y - cy) - (hh - r)
    return math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - r


def sd_circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def sd_diamond(x, y, cx, cy, r):
    return (abs(x - cx) + abs(y - cy)) - r


def clamp01(v):
    return 0.0 if v < 0.0 else (1.0 if v > 1.0 else v)


def render(size):
    """Render one frame at the given pixel size, anti-aliased over ~1px."""
    px = 1.0 / size

    def coverage(d):  # signed distance -> 0..1 alpha across one pixel
        return clamp01(0.5 - d / px)

    rows = []
    for j in range(size):
        row = []
        for i in range(size):
            x = (i + 0.5) / size
            y = (j + 0.5) / size

            # Background: rounded square with a vertical purple gradient.
            bg_a = coverage(sd_round_rect(x, y, 0.5, 0.5, 0.5, 0.5, 0.22))
            r = GRAD_TOP[0] + (GRAD_BOTTOM[0] - GRAD_TOP[0]) * y
            g = GRAD_TOP[1] + (GRAD_BOTTOM[1] - GRAD_TOP[1]) * y
            b = GRAD_TOP[2] + (GRAD_BOTTOM[2] - GRAD_TOP[2]) * y

            # Chat bubble: white rounded rect + diamond "tail" at bottom-left.
            bub_a = max(
                coverage(sd_round_rect(x, y, 0.5, 0.46, 0.27, 0.19, 0.09)),
                coverage(sd_diamond(x, y, 0.40, 0.665, 0.085)),
            )
            r += (BUBBLE[0] - r) * bub_a
            g += (BUBBLE[1] - g) * bub_a
            b += (BUBBLE[2] - b) * bub_a

            # Three typing dots inside the bubble.
            dot_a = max(
                coverage(sd_circle(x, y, 0.38, 0.46, 0.045)),
                coverage(sd_circle(x, y, 0.50, 0.46, 0.045)),
                coverage(sd_circle(x, y, 0.62, 0.46, 0.045)),
            )
            r += (DOT[0] - r) * dot_a
            g += (DOT[1] - g) * dot_a
            b += (DOT[2] - b) * dot_a

            row.append((int(round(r)), int(round(g)), int(round(b)), int(round(bg_a * 255))))
        rows.append(row)
    return rows


# --- container formats --------------------------------------------------------

def write_ico(path, entries):
    """PNG-embedded ICO (supported since Vista). entries: [(size, png_bytes)]."""
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory = b""
    blobs = b""
    for size, data in entries:
        wh = 0 if size >= 256 else size  # 0 means 256 in ICO directories
        directory += struct.pack("<BBBBHHII", wh, wh, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    with open(path, "wb") as f:
        f.write(header + directory + blobs)


def write_icns(path, entries):
    """PNG-embedded ICNS. entries: [(4-byte type, png_bytes)]."""
    body = b"".join(
        ostype + struct.pack(">I", len(data) + 8) + data for ostype, data in entries
    )
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    pngs = {size: encode_png(size, render(size)) for size in (32, 128, 256, 512)}

    def save(name, data):
        with open(os.path.join(OUT_DIR, name), "wb") as f:
            f.write(data)
        print(f"  wrote src-tauri/icons/{name} ({len(data)} bytes)")

    save("32x32.png", pngs[32])
    save("128x128.png", pngs[128])
    save("128x128@2x.png", pngs[256])
    save("icon.png", pngs[512])  # handy master copy
    write_ico(os.path.join(OUT_DIR, "icon.ico"), [(32, pngs[32]), (256, pngs[256])])
    print("  wrote src-tauri/icons/icon.ico")
    # ic07 = 128px, ic08 = 256px, ic09 = 512px (all PNG-based ICNS types).
    write_icns(
        os.path.join(OUT_DIR, "icon.icns"),
        [(b"ic07", pngs[128]), (b"ic08", pngs[256]), (b"ic09", pngs[512])],
    )
    print("  wrote src-tauri/icons/icon.icns")


if __name__ == "__main__":
    main()
