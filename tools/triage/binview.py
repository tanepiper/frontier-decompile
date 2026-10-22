#!/usr/bin/env python3
"""Draw a file as a picture, one pixel per byte (or per block of bytes), to see its structure at
a glance: headers, tables of records, text, code, compressed or encrypted parts, padding.

Usage: binview.py FILE OUT.png [--colour class|entropy] [--layout rows|hilbert]
                  [--width N] [--offset N] [--length N] [--pixels N]

  --colour class    each byte by its kind (default):
                      black  0x00          white  0xFF
                      blue   printable ASCII
                      green  control bytes 0x01 to 0x1F, 0x7F
                      red    bytes 0x80 to 0xFE
  --colour entropy  each pixel by the entropy of the 1,024 bytes around it: dark blue for
                    ordered data, through green and yellow, to red for compressed or
                    encrypted data
  --layout rows     left to right, top to bottom, --width pixels per row (default 256)
  --layout hilbert  along a Hilbert curve, which keeps neighbouring bytes close in the
                    picture, so a region of the file is a patch instead of a band
  --pixels N        the most pixels to draw (default 1,048,576); a longer file gets one
                    pixel per block of bytes, coloured by the block's most common kind

What to look for: a run of red speckle is compressed or encrypted; regular stripes are a table
of fixed-size records (the stripe's period is the record size, best seen with --width set to a
multiple of it); blue is text; black is empty space or padding; an executable shows its
header, a dense code section, then tables and strings. Compare two versions of a file side by
side to see which parts changed. Writes PNG with the standard library only.
"""
import math
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'formats'))
from rawimage import png  # noqa: E402

CLASS = {'zero': (0, 0, 0), 'ff': (255, 255, 255), 'text': (55, 126, 220), 'low': (77, 175, 74),
         'high': (228, 26, 28)}


def kind(b):
    if b == 0:
        return 'zero'
    if b == 0xFF:
        return 'ff'
    if 32 <= b < 127 or b in (9, 10, 13):
        return 'text'
    if b < 32 or b == 127:
        return 'low'
    return 'high'


LOOKUP = [CLASS[kind(b)] for b in range(256)]


def ramp(x):
    """0..1 -> dark blue, cyan, green, yellow, red."""
    stops = [(0, (20, 20, 90)), (0.35, (30, 160, 200)), (0.6, (60, 180, 60)), (0.85, (240, 220, 40)), (1, (220, 30, 30))]
    for (a, ca), (b, cb) in zip(stops, stops[1:]):
        if x <= b:
            t = (x - a) / (b - a) if b > a else 0
            return tuple(int(ca[i] + (cb[i] - ca[i]) * t) for i in range(3))
    return stops[-1][1]


def entropy(chunk):
    if not chunk:
        return 0.0
    counts = {}
    for b in chunk:
        counts[b] = counts.get(b, 0) + 1
    n = len(chunk)
    return -sum(c / n * math.log2(c / n) for c in counts.values())


def hilbert(d, order):
    """Point d along a Hilbert curve of side 2**order -> (x, y)."""
    x = y = 0
    s, t = 1, d
    while s < (1 << order):
        rx = 1 & (t // 2)
        ry = 1 & (t ^ rx)
        if ry == 0:
            if rx == 1:
                x, y = s - 1 - x, s - 1 - y
            x, y = y, x
        x += s * rx
        y += s * ry
        t //= 4
        s *= 2
    return x, y


def colours(data, mode, per):
    """One colour for each block of `per` bytes."""
    out = []
    if mode == 'entropy':
        window = max(1024, per)
        for i in range(0, len(data), per):
            mid = i + per // 2
            chunk = data[max(0, mid - window // 2):mid + window // 2]
            most = math.log2(min(256, max(2, len(chunk))))   # what random data of this size reaches
            out.append(ramp(min(1.0, entropy(chunk) / most)))
        return out
    for i in range(0, len(data), per):
        block = data[i:i + per]
        if per == 1:
            out.append(LOOKUP[block[0]])
            continue
        tally = {}
        for b in block:
            k = kind(b)
            tally[k] = tally.get(k, 0) + 1
        out.append(CLASS[max(tally, key=tally.get)])
    return out


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    with open(argv[0], 'rb') as f:
        data = f.read()
    off = int(opt('--offset', '0'), 0)
    data = data[off:off + int(opt('--length'), 0)] if opt('--length') else data[off:]
    limit = int(opt('--pixels', str(1 << 20)), 0)
    per = max(1, math.ceil(len(data) / limit))
    cols = colours(data, opt('--colour', 'class'), per)
    if opt('--layout', 'rows') == 'hilbert':
        order = max(1, math.ceil(math.log2(math.sqrt(max(len(cols), 1)))))
        side = 1 << order
        grid = [[(40, 40, 40)] * side for _ in range(side)]
        for d, c in enumerate(cols):
            x, y = hilbert(d, order)
            grid[y][x] = c
        width, height = side, side
    else:
        width = int(opt('--width', '256'), 0)
        height = max(1, math.ceil(len(cols) / width))
        cols += [(40, 40, 40)] * (width * height - len(cols))
        grid = [cols[y * width:(y + 1) * width] for y in range(height)]
    png(argv[1], width, height, [b''.join(bytes(c) for c in row) for row in grid])
    print(f'{argv[1]}: {width} x {height}, {per} byte(s) per pixel, from offset 0x{off:x}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
