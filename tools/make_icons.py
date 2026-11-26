"""Render the Ditto-RS icon set with only the Python standard library.

App icon: violet-blue rounded square, two stacked "copies" of a clipboard sheet.
Tray icon: monochrome template glyph (macOS) plus a colour variant (Windows/Linux).
Usage: python3 -I make_icons.py <out_dir>
"""
import math
import os
import struct
import subprocess
import sys
import zlib


def png_bytes(w, h, rgba):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw.extend(rgba[y * w * 4:(y + 1) * w * 4])

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def sd_round_rect(px, py, cx, cy, hw, hh, r):
    qx = abs(px - cx) - hw + r
    qy = abs(py - cy) - hh + r
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    inside = min(max(qx, qy), 0.0)
    return outside + inside - r


def coverage(d, aa):
    # d < 0 inside; smooth over one pixel
    t = 0.5 - d / aa
    return 0.0 if t <= 0 else 1.0 if t >= 1 else t


def over(dst, src_rgb, a):
    r, g, b, da = dst
    sa = a
    oa = sa + da * (1 - sa)
    if oa == 0:
        return (0.0, 0.0, 0.0, 0.0)
    return (
        (src_rgb[0] * sa + r * da * (1 - sa)) / oa,
        (src_rgb[1] * sa + g * da * (1 - sa)) / oa,
        (src_rgb[2] * sa + b * da * (1 - sa)) / oa,
        oa,
    )


def render_app(size):
    s = float(size)
    aa = 1.0
    out = bytearray(size * size * 4)
    top = (0.36, 0.49, 0.98)
    bottom = (0.47, 0.30, 0.86)
    for y in range(size):
        for x in range(size):
            px, py = x + 0.5, y + 0.5
            u, v = px / s, py / s
            col = (0.0, 0.0, 0.0, 0.0)
            # background squircle-ish rounded square with a 6% margin (macOS grid)
            d = sd_round_rect(px, py, s / 2, s / 2, s * 0.44, s * 0.44, s * 0.10)
            a = coverage(d, aa)
            if a > 0:
                t = v
                bg = tuple(top[i] * (1 - t) + bottom[i] * t for i in range(3))
                col = over(col, bg, a)
                # back sheet ("the copy"), offset up-left, translucent
                d2 = sd_round_rect(px, py, s * 0.455, s * 0.47, s * 0.20, s * 0.25, s * 0.035)
                col = over(col, (1, 1, 1), 0.38 * coverage(d2, aa))
                # front sheet
                d3 = sd_round_rect(px, py, s * 0.545, s * 0.555, s * 0.20, s * 0.25, s * 0.035)
                col = over(col, (1, 1, 1), coverage(d3, aa))
                # clip at the top of the front sheet
                d4 = sd_round_rect(px, py, s * 0.545, s * 0.31, s * 0.085, s * 0.035, s * 0.025)
                col = over(col, (0.20, 0.17, 0.42), coverage(d4, aa))
                # text lines on the front sheet
                for i, wfrac in enumerate((0.13, 0.11, 0.08)):
                    ly = s * (0.44 + i * 0.085)
                    lx = s * 0.425 + s * wfrac
                    d5 = sd_round_rect(px, py, lx, ly, s * wfrac, s * 0.018, s * 0.018)
                    line = tuple(top[k] * 0.6 + bottom[k] * 0.4 for k in range(3))
                    col = over(col, line, coverage(d5, aa))
            r, g, b, al = col
            o = (y * size + x) * 4
            out[o:o + 4] = bytes((int(r * 255 + 0.5), int(g * 255 + 0.5), int(b * 255 + 0.5), int(al * 255 + 0.5)))
    return out


def render_tray(size, rgb):
    """Two offset sheets with a clip, drawn as solid glyph in `rgb`."""
    s = float(size)
    aa = 1.0
    out = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            px, py = x + 0.5, y + 0.5
            front = sd_round_rect(px, py, s * 0.56, s * 0.57, s * 0.27, s * 0.34, s * 0.06)
            front_hole = sd_round_rect(px, py, s * 0.56, s * 0.57, s * 0.27 - s * 0.085, s * 0.34 - s * 0.085, s * 0.03)
            back = sd_round_rect(px, py, s * 0.40, s * 0.43, s * 0.27, s * 0.34, s * 0.06)
            clip = sd_round_rect(px, py, s * 0.56, s * 0.24, s * 0.12, s * 0.055, s * 0.04)
            ring = max(front, -front_hole)  # outline of the front sheet
            back_visible = max(back, -front + s * 0.05)  # back sheet only outside the front one
            back_ring = max(back_visible, -sd_round_rect(px, py, s * 0.40, s * 0.43, s * 0.27 - s * 0.085, s * 0.34 - s * 0.085, s * 0.03))
            lines = min(
                sd_round_rect(px, py, s * 0.56, s * 0.52, s * 0.11, s * 0.035, s * 0.03),
                sd_round_rect(px, py, s * 0.53, s * 0.66, s * 0.08, s * 0.035, s * 0.03),
            )
            d = min(ring, back_ring, clip, lines)
            a = coverage(d, aa)
            o = (y * size + x) * 4
            out[o:o + 4] = bytes((rgb[0], rgb[1], rgb[2], int(a * 255 + 0.5)))
    return out


def write(path, size, data):
    with open(path, "wb") as f:
        f.write(png_bytes(size, size, data))


def write_ico(path, pngs):
    header = struct.pack("<HHH", 0, 1, len(pngs))
    entries = b""
    offset = 6 + 16 * len(pngs)
    blobs = b""
    for size, blob in pngs:
        b = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", b, b, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        blobs += blob
    with open(path, "wb") as f:
        f.write(header + entries + blobs)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    cache = {}

    def app(size):
        if size not in cache:
            cache[size] = png_bytes(size, size, render_app(size))
        return cache[size]

    for name, size in (("32x32.png", 32), ("64x64.png", 64), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
        with open(os.path.join(out, name), "wb") as f:
            f.write(app(size))
    write_ico(os.path.join(out, "icon.ico"), [(s, app(s)) for s in (16, 24, 32, 48, 64, 256)])

    iconset = os.path.join(out, "icon.iconset")
    os.makedirs(iconset, exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        with open(os.path.join(iconset, f"icon_{base}x{base}.png"), "wb") as f:
            f.write(app(base))
        with open(os.path.join(iconset, f"icon_{base}x{base}@2x.png"), "wb") as f:
            f.write(app(base * 2))
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(out, "icon.icns")], check=True)
    for f_ in os.listdir(iconset):
        os.remove(os.path.join(iconset, f_))
    os.rmdir(iconset)

    write(os.path.join(out, "tray-template.png"), 44, render_tray(44, (0, 0, 0)))
    write(os.path.join(out, "tray.png"), 64, render_tray(64, (255, 255, 255)))
    write(os.path.join(out, "tray-dark.png"), 64, render_tray(64, (40, 40, 48)))
    print("ok", sorted(os.listdir(out)))


if __name__ == "__main__":
    main()
