#!/usr/bin/env python3
"""Find the fields in many messages or records of one kind: which byte offsets are constant,
which count up, which hold the length, which pick the kind of message, and which vary.

Usage: fields.py FILE --stride N [--offset N] [--count N]    fixed-size records in one file
       fields.py DIR                                         one message per file (pcap.py --dump)
       options: --by-type OFFSET   group the messages by the byte at OFFSET first
                --png OUT.png      a picture: one row per record, one column per byte

A protocol message or a record in a data file is a row of fields. Laid side by side, many of
them show their layout: a byte that never changes is a magic number or a version; a value that
rises by one each message is a sequence number; a value equal to the message's length is a
length field; a byte with a few values that go with different lengths is the type. This tool
checks each offset for these, as 8-bit, 16-bit and 32-bit values in both byte orders, and
prints a table. The picture shows the same at a glance: constant columns are flat colour,
counters are smooth ramps, varying data is noise.

Start from the first bytes, where headers are. Group by the type byte (--by-type) once you
have found it: each type has its own layout. Messages that look like noise all the way through
are compressed or encrypted. Try zlib on them (filetypes.py and xorscan.py help), then look
again. The browser version is the records explorer
(https://tanepiper.github.io/game-frame/lab/records). Standard library only.
"""
import os
import struct
import sys


def load(argv):
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    path = argv[0]
    if os.path.isdir(path):
        rows = []
        for name in sorted(os.listdir(path)):
            with open(os.path.join(path, name), 'rb') as f:
                rows.append(f.read())
        return rows
    with open(path, 'rb') as f:
        data = f.read()[int(opt('--offset', '0'), 0):]
    stride = int(opt('--stride', '0'), 0)
    if not stride:
        raise SystemExit('give --stride N for a file of fixed-size records, or a folder of messages')
    count = int(opt('--count', str(len(data) // stride)), 0)
    return [data[i * stride:(i + 1) * stride] for i in range(count)]


def classify(rows):
    """For each offset that every row has: a description of what it looks like."""
    width = min(len(r) for r in rows)
    out = []
    for at in range(width):
        notes = []
        col = [r[at] for r in rows]
        if len(set(col)) == 1:
            notes.append(f'constant 0x{col[0]:02x}')
        for size, codes in ((2, ('<H', '>H')), (4, ('<I', '>I'))):
            if at + size > width:
                continue
            for code in codes:
                vals = [struct.unpack_from(code, r, at)[0] for r in rows]
                order = 'LE' if code[0] == '<' else 'BE'
                # The high-order bytes of a counter are zero or change; a constant non-zero byte
                # there is a magic number next to a smaller counter.
                high = range(at + 1, at + size) if order == 'LE' else range(at, at + size - 1)
                plausible = all(len({r[h] for r in rows}) > 1 or rows[0][h] == 0 for h in high)
                step = vals[1] - vals[0] if len(vals) > 1 else 0
                if plausible and len(rows) > 2 and step > 0 and step % 256 and all(b - a == step for a, b in zip(vals, vals[1:])):
                    notes.append(f'counter u{size * 8} {order} (+{step} each)')
                if all(v == len(r) for v, r in zip(vals, rows)) and len(set(vals)) > 1:
                    notes.append(f'length u{size * 8} {order} (the whole message)')
                elif all(v == len(r) - at - size for v, r in zip(vals, rows)) and len(set(vals)) > 1:
                    notes.append(f'length u{size * 8} {order} (the bytes after it)')
        step = col[1] - col[0] if len(col) > 1 else 0
        if len(rows) > 2 and step > 0 and all(b - a == step for a, b in zip(col, col[1:])) \
                and not any(n.startswith('counter') for n in notes):
            notes.append(f'counter u8 (+{step} each)')
        if all(v == len(r) for v, r in zip(col, rows)) and len(set(col)) > 1:
            notes.append('length u8 (the whole message)')
        elif all(v == len(r) - at - 1 for v, r in zip(col, rows)) and len(set(col)) > 1:
            notes.append('length u8 (the bytes after it)')
        if not notes:
            distinct = len(set(col))
            if 1 < distinct <= 8 and len(rows) >= 8:
                # Few values: a type, if the lengths go with the value.
                lens = {}
                for v, r in zip(col, rows):
                    lens.setdefault(v, set()).add(len(r))
                fixed = sum(1 for s in lens.values() if len(s) == 1)
                notes.append(f'{distinct} values: {", ".join(f"0x{v:02x}" for v in sorted(set(col)))}'
                             + (' (each with its own length: a type?)' if fixed == distinct and len({min(s) for s in lens.values()}) > 1 else ''))
            else:
                notes.append(f'varies ({distinct} values)')
        out.append((at, notes))
    return width, out


def picture(rows, path):
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__))))
    from rawimage import png
    width = max(len(r) for r in rows)
    lines = []
    for r in rows:
        line = bytearray()
        for i in range(width):
            if i >= len(r):
                line += b'\x28\x28\x28'
                continue
            v = r[i]                          # a colour wheel: neighbouring values, near colours
            h = v / 256 * 6
            k = int(h)
            f = h - k
            q, t = int(255 * (1 - f)), int(255 * f)
            line += bytes([(255, q, 0, 0, t, 255)[k], (t, 255, 255, q, 0, 0)[k], (0, 0, t, 255, 255, q)[k]])
        lines.append(bytes(line))
    scale = max(1, min(8, 1024 // max(width, 1)))
    lines = [b''.join(l[i:i + 3] * scale for i in range(0, len(l), 3)) for l in lines for _ in range(scale)]
    png(path, width * scale, len(rows) * scale, lines)


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    rows = [r for r in load(argv) if r]
    if not rows:
        raise SystemExit('no records')
    groups = {'all': rows}
    if opt('--by-type'):
        at = int(opt('--by-type'), 0)
        groups = {}
        for r in rows:
            if len(r) > at:
                groups.setdefault(f'type 0x{r[at]:02x}', []).append(r)
    for name, members in sorted(groups.items()):
        lengths = sorted({len(r) for r in members})
        print(f'== {name}: {len(members)} records, lengths {lengths[:6]}{" ..." if len(lengths) > 6 else ""}')
        width, table = classify(members)
        for at, notes in table:
            print(f'  +0x{at:03x}  {"; ".join(notes)}')
        if width < max(lengths):
            print(f'  (offsets from 0x{width:x} on are not in every record)')
    if opt('--png'):
        picture(rows, opt('--png'))
        print(f'picture written to {opt("--png")}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
