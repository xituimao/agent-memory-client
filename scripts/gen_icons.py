#!/usr/bin/env python3
"""Pure-stdlib PNG generator for tray status icons and the app icon."""
import struct, zlib, math, os

OUT = os.path.join(os.path.dirname(__file__), "src-tauri", "icons")

def write_png(path, w, h, pixels):
    """pixels: list of rows, each row list of (r,g,b,a)"""
    raw = b""
    for row in pixels:
        raw += b"\x00" + b"".join(struct.pack("4B", *px) for px in row)
    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)

def circle_icon(size, color):
    """Filled circle with a white ring, visible on light & dark menu bars."""
    cx = cy = (size - 1) / 2
    radius = size / 2 - 1
    ring_w = max(1.5, size * 0.07)
    px = []
    for y in range(size):
        row = []
        for x in range(size):
            d = math.hypot(x - cx, y - cy)
            if d <= radius - ring_w:
                row.append((*color, 255))
            elif d <= radius:
                row.append((255, 255, 255, 255))
            else:
                row.append((0, 0, 0, 0))
        px.append(row)
    return px

def blend(dst, src):
    sr, sg, sb, sa = src
    if sa == 255:
        return src
    dr, dg, db, da = dst
    a = sa / 255
    return (int(sr * a + dr * (1 - a)), int(sg * a + dg * (1 - a)),
            int(sb * a + db * (1 - a)), int(da + sa * (1 - da / 255)))

def app_icon(size):
    """Rounded-square indigo tile with a white 'connected nodes' glyph."""
    px = [[(0, 0, 0, 0)] * size for _ in range(size)]
    r = size * 0.22  # corner radius
    for y in range(size):
        for x in range(size):
            dx = max(0, max(r - x, x - (size - 1 - r)))
            dy = max(0, max(r - y, y - (size - 1 - r)))
            if math.hypot(dx, dy) <= r:
                t = y / size  # subtle vertical gradient
                px[y][x] = (int(88 + 30 * t), int(86 + 20 * t), int(214 - 20 * t), 255)
    # glyph: three memory nodes connected by lines
    nodes = [(0.30, 0.62), (0.50, 0.34), (0.70, 0.58)]
    edges = [(0, 1), (1, 2), (0, 2)]
    lw = size * 0.045
    def to_px(p):
        return (p[0] * size, p[1] * size)
    def dist_seg(px_, py_, a, b):
        ax, ay = a; bx, by = b
        vx, vy = bx - ax, by - ay
        wx, wy = px_ - ax, py_ - ay
        t = max(0, min(1, (wx * vx + wy * vy) / (vx * vx + vy * vy)))
        return math.hypot(px_ - (ax + t * vx), py_ - (ay + t * vy))
    for i, j in edges:
        a, b = to_px(nodes[i]), to_px(nodes[j])
        for y in range(size):
            for x in range(size):
                if dist_seg(x + .5, y + .5, a, b) <= lw:
                    px[y][x] = blend(px[y][x], (255, 255, 255, 235))
    for nx, ny in nodes:
        c = to_px((nx, ny)); nr = size * 0.075
        for y in range(size):
            for x in range(size):
                if math.hypot(x - c[0], y - c[1]) <= nr:
                    px[y][x] = blend(px[y][x], (255, 255, 255, 255))
    return px

GREEN = (52, 199, 89)    # systemGreen
RED = (255, 59, 48)      # systemRed
GRAY = (152, 152, 157)   # systemGray

os.makedirs(OUT, exist_ok=True)
for name, color in [("green", GREEN), ("gray", GRAY), ("red", RED)]:
    for s in (22, 44):
        write_png(os.path.join(OUT, f"tray-{name}-{s}.png"), s, s, circle_icon(s, color))
write_png(os.path.join(OUT, "icon.png"), 512, 512, app_icon(512))
print("icons written to", OUT)
