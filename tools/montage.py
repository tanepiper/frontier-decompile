"""Tile 320x200 PNG captures into one image for review.

Usage: python3 -I tools/montage.py OUT.png IN1.png [IN2.png ...]
Reads the stored/zlib PNGs written by frontier-dos (8-bit RGBA, filter 0) and writes a grid of up
to five columns. Inputs and output belong in game-data/derived/ (git-ignored).
"""
import struct, sys, zlib

def load(path):
    d = open(path, 'rb').read()
    i, idat = 8, b''
    while i < len(d):
        n = struct.unpack('>I', d[i:i + 4])[0]
        if d[i + 4:i + 8] == b'IHDR':
            w, h = struct.unpack('>II', d[i + 8:i + 16])
        if d[i + 4:i + 8] == b'IDAT':
            idat += d[i + 8:i + 8 + n]
        i += 12 + n
    raw = zlib.decompress(idat)
    return w, h, [raw[r * (w * 4 + 1) + 1:(r + 1) * (w * 4 + 1)] for r in range(h)]

def main():
    out, ins = sys.argv[1], sys.argv[2:]
    ims = [load(p) for p in ins]
    w, h = ims[0][0], ims[0][1]
    cols = min(5, len(ims))
    rows_n = (len(ims) + cols - 1) // cols
    W, H = w * cols, h * rows_n
    canvas = [bytearray(W * 4) for _ in range(H)]
    for k, (_, _, rows) in enumerate(ims):
        ox, oy = (k % cols) * w, (k // cols) * h
        for y in range(h):
            canvas[oy + y][ox * 4:(ox + w) * 4] = rows[y]
    raw = b''.join(b'\0' + bytes(r) for r in canvas)
    chunk = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 6, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
    open(out, 'wb').write(png)

main()
