"""Generate the original rounded crop mark as SVG, PNG and Windows ICO.

Uses only Python's standard library. The app surface follows the supplied
SimpleTodo reference (#2563EB); tray artwork is an independent alpha mask.
The rasterizer samples the same circular arcs and round-ended lines as the SVG.
"""
from pathlib import Path
import math
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent / "assets" / "icons"
BLUE = (37, 99, 235)
RADIUS = 33
LINES = [(480, 320, 368, 320), (320, 368, 320, 560),
         (544, 704, 656, 704), (704, 656, 704, 464)]
ARCS = [(368, 368, -1), (656, 656, 1)]
PATH = "M480 320H368A48 48 0 0 0 320 368V560M544 704H656A48 48 0 0 0 704 656V464"


def stroke_distance(x, y):
    best = float("inf")
    for ax, ay, bx, by in LINES:
        dx, dy = bx - ax, by - ay
        t = max(0, min(1, ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)))
        best = min(best, math.hypot(x - ax - t * dx, y - ay - t * dy) - RADIUS)
    for cx, cy, direction in ARCS:
        if (x - cx) * direction >= 0 and (y - cy) * direction >= 0:
            best = min(best, abs(math.hypot(x - cx, y - cy) - 48) - RADIUS)
    return best


def render(size, tray=False):
    # Tray keeps the same symbol with an optical size suited to menu bars.
    span, offset = (768, 128) if tray else (1024, 0)
    step = span / size
    rows = []
    for y in range(size):
        row = bytearray()
        for x in range(size):
            fx, fy = (x + .5) * step + offset, (y + .5) * step + offset
            ink = max(0, min(1, .5 - stroke_distance(fx, fy) / step))
            if tray:
                row.extend((0, 0, 0, round(255 * ink)))
            else:
                qx, qy = abs(fx - 512) - 256, abs(fy - 512) - 256
                distance = math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - 208
                alpha = max(0, min(1, .5 - distance / step))
                rgb = [round(c + (255 - c) * ink) for c in BLUE]
                row.extend((*rgb, round(255 * alpha)) if alpha else (0, 0, 0, 0))
        rows.append(b"\0" + row)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b""))


ROOT.mkdir(parents=True, exist_ok=True)
mark = f'<path d="{PATH}" fill="none" stroke="{{color}}" stroke-width="66" stroke-linecap="round" stroke-linejoin="round"/>'
(ROOT / "app.svg").write_text(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">'
    '<rect x="48" y="48" width="928" height="928" rx="208" fill="#2563eb"/>'
    + mark.format(color="#fff") + '</svg>\n', encoding="utf-8")
(ROOT / "tray.svg").write_text(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="128 128 768 768">'
    + mark.format(color="#000") + '</svg>\n', encoding="utf-8")
(ROOT / "app.png").write_bytes(render(1024))
(ROOT / "tray.png").write_bytes(render(64, tray=True))

# Multiple native icon sizes keep Windows small icons sharp.
payloads = [(size, render(size)) for size in (16, 32, 48, 256)]
offset = 6 + 16 * len(payloads)
entries = []
for size, data in payloads:
    entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
(ROOT / "app.ico").write_bytes(
    struct.pack('<HHH', 0, 1, len(entries)) + b''.join(entries) + b''.join(data for _, data in payloads))
