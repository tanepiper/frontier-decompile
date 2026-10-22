#!/usr/bin/env python3
"""Turn raw pixel data into a PNG: a dump of video memory, a screen saved by a DOS game, or a
block of a data file that may hold a picture.

Usage: rawimage.py INPUT OUT.png --mode MODE [--width W] [--height H] [--offset N]
                   [--palette FILE] [--palette-offset N] [--start N] [--scale S]

  MODE        how the bytes hold pixels:
                linear8   one byte per pixel, a palette index (VGA mode 13h, most 256-colour data)
                modex     VGA unchained 256 colours: INPUT holds the four planes one after the
                          other (4 x 64 KB), pixel x is in plane x % 4 (Mode X)
                planar4   EGA and VGA 16 colours: four bit planes one after the other, one bit
                          per pixel in each (INPUT holds 4 x the plane size)
                cga2      CGA 320 x 200 in 4 colours: 2 bits per pixel, even lines from 0 and
                          odd lines from 0x2000 (a dump of B800:0000)
                packed4   4 bits per pixel, two pixels per byte, high nibble first
                packed2   2 bits per pixel, high bits first; packed1  1 bit per pixel
                rgb565, rgb555   16-bit colour, little-endian
  --width, --height    the size in pixels (defaults 320 x 200; Mode X 320 x 240)
  --offset N           skip N bytes of INPUT first; --start N starts at pixel row N's address
                       for modex and planar4 (a second page, a scrolled screen)
  --palette FILE       a palette: 768 bytes of RGB (6-bit VGA DAC values are scaled up when
                       no value is above 63), or a JASC .pal text file; --palette-offset N to
                       read it from inside a bigger file. Without it: the default VGA palette
                       for linear8 and modex, the EGA palette for 16 colours, CGA palette 1
  --scale S            make each pixel S x S (for looking, not for comparing)

Two uses. Run a picture you captured from the original (a DOSBox-X memory dump of A000:0000,
an emulator's video RAM) into a PNG, to compare with the port's frame (re-verify). Or try a
block of a data file as a picture: if a 64,000-byte block shows a screen at 320 x 200 with
the right palette, the format is a raw bitmap. A wrong width shows as slanted stripes: change
it until the picture stands straight.

Writes PNG with the standard library only.
"""
import os
import struct
import sys
import zlib


def png(path, width, height, rows):
    """rows: a list of bytes objects, each width * 3 bytes of RGB."""
    raw = b''.join(b'\x00' + r for r in rows)

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)))
        f.write(chunk(b'IDAT', zlib.compress(raw, 9)))
        f.write(chunk(b'IEND', b''))


def default_vga():
    """The VGA BIOS's default 256-colour palette: the 16 EGA colours, 16 greys, then nine rings
    of 24 hues (three brightness levels, each in three saturations), then 8 blacks."""
    pal = ega_palette()
    greys = [0, 5, 8, 11, 14, 17, 20, 24, 28, 32, 36, 40, 45, 50, 56, 63]
    pal += [(g * 255 // 63,) * 3 for g in greys]
    levels = [[0, 16, 31, 47, 63], [31, 39, 47, 55, 63], [45, 49, 54, 58, 63],
              [0, 7, 14, 21, 28], [14, 17, 21, 24, 28], [20, 22, 24, 26, 28],
              [0, 4, 8, 12, 16], [8, 10, 12, 14, 16], [11, 12, 13, 15, 16]]
    for v in levels:                     # blue, magenta, red, yellow, green, cyan, back to blue
        lo, hi = v[0], v[4]
        ring = [(v[i], lo, hi) for i in range(5)] + [(hi, lo, v[i]) for i in (3, 2, 1, 0)]
        ring += [(hi, v[i], lo) for i in (1, 2, 3, 4)] + [(v[i], hi, lo) for i in (3, 2, 1, 0)]
        ring += [(lo, hi, v[i]) for i in (1, 2, 3, 4)] + [(lo, v[i], hi) for i in (3, 2, 1)]
        pal += [tuple(c * 255 // 63 for c in rgb) for rgb in ring]
    pal += [(0, 0, 0)] * (256 - len(pal))
    return pal[:256]


def ega_palette():
    base = [(0, 0, 0), (0, 0, 42), (0, 42, 0), (0, 42, 42), (42, 0, 0), (42, 0, 42), (42, 21, 0),
            (42, 42, 42), (21, 21, 21), (21, 21, 63), (21, 63, 21), (21, 63, 63), (63, 21, 21),
            (63, 21, 63), (63, 63, 21), (63, 63, 63)]
    return [tuple(c * 255 // 63 for c in rgb) for rgb in base]


CGA1 = [(0, 0, 0), (85, 255, 255), (255, 85, 255), (255, 255, 255)]


def load_palette(path, offset):
    with open(path, 'rb') as f:
        data = f.read()
    if data.startswith(b'JASC-PAL'):
        lines = data.decode('latin-1').split()
        count = int(lines[2])
        vals = [int(x) for x in lines[3:3 + 3 * count]]
        return [tuple(vals[i:i + 3]) for i in range(0, len(vals), 3)]
    data = data[offset:offset + 768]
    six_bit = max(data) <= 63
    return [tuple((c * 255 // 63 if six_bit else c) for c in data[i:i + 3]) for i in range(0, len(data) - 2, 3)]


def indices(data, mode, width, height, start):
    """Palette indices, one list per row."""
    rows = []
    if mode == 'linear8':
        for y in range(height):
            rows.append(list(data[y * width:(y + 1) * width]))
    elif mode == 'modex':
        plane = len(data) // 4
        stride = width // 4
        for y in range(height):
            at = (start + y) * stride
            rows.append([data[(x % 4) * plane + at + x // 4] if at + x // 4 < plane else 0 for x in range(width)])
    elif mode == 'planar4':
        plane = len(data) // 4
        stride = width // 8
        for y in range(height):
            at = (start + y) * stride
            row = []
            for x in range(width):
                bit = 7 - x % 8
                v = 0
                for p in range(4):
                    i = p * plane + at + x // 8
                    if i < len(data) and data[i] >> bit & 1:
                        v |= 1 << p
                row.append(v)
            rows.append(row)
    elif mode == 'cga2':
        for y in range(height):
            at = (0x2000 if y % 2 else 0) + (y // 2) * (width // 4)
            rows.append([(data[at + x // 4] >> (6 - 2 * (x % 4))) & 3 if at + x // 4 < len(data) else 0
                         for x in range(width)])
    elif mode in ('packed4', 'packed2', 'packed1'):
        bits = int(mode[-1])
        per = 8 // bits
        stride = (width * bits + 7) // 8
        mask = (1 << bits) - 1
        for y in range(height):
            at = y * stride
            rows.append([(data[at + x // per] >> (8 - bits * (x % per + 1))) & mask if at + x // per < len(data)
                         else 0 for x in range(width)])
    else:
        raise SystemExit(f'unknown mode {mode}')
    return rows


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    mode = opt('--mode', 'linear8')
    width = int(opt('--width', '320'), 0)
    height = int(opt('--height', '240' if mode == 'modex' else '200'), 0)
    scale = int(opt('--scale', '1'))
    with open(argv[0], 'rb') as f:
        data = f.read()[int(opt('--offset', '0'), 0):]
    if mode in ('rgb565', 'rgb555'):
        rows = []
        for y in range(height):
            row = bytearray()
            for x in range(width):
                i = 2 * (y * width + x)
                v = struct.unpack_from('<H', data, i)[0] if i + 2 <= len(data) else 0
                if mode == 'rgb565':
                    r, g, b = v >> 11 & 31, v >> 5 & 63, v & 31
                    row += bytes((r * 255 // 31, g * 255 // 63, b * 255 // 31))
                else:
                    r, g, b = v >> 10 & 31, v >> 5 & 31, v & 31
                    row += bytes((r * 255 // 31, g * 255 // 31, b * 255 // 31))
            rows.append(bytes(row))
    else:
        if opt('--palette'):
            pal = load_palette(opt('--palette'), int(opt('--palette-offset', '0'), 0))
        elif mode in ('linear8', 'modex'):
            pal = default_vga()
        elif mode in ('planar4', 'packed4'):
            pal = ega_palette()
        elif mode in ('cga2', 'packed2'):
            pal = CGA1
        else:
            pal = [(0, 0, 0), (255, 255, 255)]
        pal = pal + [(0, 0, 0)] * (256 - len(pal))
        rows = [b''.join(bytes(pal[i]) for i in row) for row in indices(data, mode, width, height,
                                                                         int(opt('--start', '0'), 0))]
    if scale > 1:
        rows = [b''.join(r[i:i + 3] * scale for i in range(0, len(r), 3)) for r in rows for _ in range(scale)]
    png(argv[1], width * scale, height * scale, rows)
    print(f'{argv[1]}: {width * scale} x {height * scale}, mode {mode}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
