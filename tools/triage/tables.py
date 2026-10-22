#!/usr/bin/env python3
"""Find the sine and cosine tables in a binary or a data file, and say exactly how each was
made: its length, its scale, its number type and how it was rounded.

Usage: tables.py FILE [FILE ...] [--json]

Games of the 1980s and 1990s rarely called sin() at run time. They kept a table: 256 or 1024
steps round the circle (sometimes 360, or a quarter circle), scaled to a fixed-point range such
as 127, 256, 16384 or 65536, stored as bytes, 16-bit or 32-bit integers, or floats. A port must
use the same table, entry for entry, or every rotation, wave and trajectory drifts. This tool
builds candidate tables for the common shapes and searches for their first entries. Each hit
names the shape, the address and how many entries match the formula. Where the rounding is
unusual, the entries that differ are listed: then copy the table from the binary
(tools/pe/readdata.py) instead of computing it.

The search covers sine and cosine with these choices:
  steps     64, 128, 256, 360, 512, 1024, 2048, 4096, 8192, 16384 per circle
  scale     the value at 90 degrees: 63, 64, 127, 128, 255, 256, 1023, 1024, 4095, 4096,
            16383, 16384, 32767, 32768, 65535, 65536 (and 1.0 for floats)
  type      signed 8-bit, 16-bit and 32-bit integers, little-endian and big-endian; 32-bit
            and 64-bit floats
  rounding  to nearest, and toward zero (truncation)
Standard library only.
"""
import json
import math
import struct
import sys

STEPS = [64, 128, 256, 360, 512, 1024, 2048, 4096, 8192, 16384]
SCALES = [63, 64, 127, 128, 255, 256, 1023, 1024, 4095, 4096, 16383, 16384, 32767, 32768, 65535, 65536]
INT_TYPES = [('i8', 'b', 1, 127), ('i16', 'h', 2, 32767), ('i32', 'i', 4, 2**31 - 1)]
PROBE = 16      # entries searched for; more are then compared


def values(fn, steps, scale, rounding, count):
    out = []
    for i in range(count):
        v = fn(2 * math.pi * i / steps) * scale
        out.append(round(v) if rounding == 'nearest' else int(v))
    return out


def candidates():
    for name, fn, start in (('sine', math.sin, 1), ('cosine', math.cos, 0)):
        for steps in STEPS:
            for scale in SCALES:
                for rounding in ('nearest', 'truncated'):
                    first = values(fn, steps, scale, rounding, PROBE + start)
                    # A sine table starts 0, s1, s2...: search from entry 1 to avoid "0" matching everywhere.
                    probe = first[start:start + PROBE]
                    if len(set(probe)) < 3:
                        continue          # too flat to be told apart from other data
                    for tname, code, size, limit in INT_TYPES:
                        if scale > limit:
                            continue
                        for order in ('<', '>'):
                            if size == 1 and order == '>':
                                continue
                            yield (name, steps, scale, rounding, tname + ('be' if order == '>' else ''),
                                   order + code, size, start, fn, probe)
        for steps in STEPS:
            for fcode, size, tname in (('f', 4, 'f32'), ('d', 8, 'f64')):
                probe = [fn(2 * math.pi * i / steps) for i in range(start, start + PROBE)]
                yield (name, steps, 1.0, 'float', tname, '<' + fcode, size, start, fn, probe)


def check(data, at, fmt, size, steps, scale, rounding, fn, start):
    """How many entries from the table start match, up to one full circle; and the differences."""
    begin = at - start * size
    good, diffs = 0, []
    for i in range(steps):
        p = begin + i * size
        if p < 0 or p + size > len(data):
            break
        got = struct.unpack_from(fmt, data, p)[0]
        if rounding == 'float':
            want = fn(2 * math.pi * i / steps)
            ok = abs(got - want) < (1e-6 if size == 4 else 1e-12)
        else:
            v = fn(2 * math.pi * i / steps) * scale
            want = round(v) if rounding == 'nearest' else int(v)
            ok = got == want
        if ok:
            good += 1
        else:
            diffs.append((i, got, want))
            if len(diffs) > max(8, steps // 16):
                break
    return begin, good, diffs


def scan(path):
    with open(path, 'rb') as f:
        data = f.read()
    hits, seen = [], set()
    for name, steps, scale, rounding, tname, fmt, size, start, fn, probe in candidates():
        needle = b''.join(struct.pack(fmt, v) for v in probe)
        at = data.find(needle)
        while at >= 0:
            begin, good, diffs = check(data, at, fmt, size, steps, scale, rounding, fn, start)
            key = (begin, tname)
            if key not in seen and good >= min(steps, 16):
                seen.add(key)
                hits.append({'file': path, 'offset': begin, 'function': name, 'steps': steps,
                             'scale': scale, 'type': tname, 'rounding': rounding,
                             'matching': good, 'differences': diffs[:8]})
            at = data.find(needle, at + 1)
    # Keep the best fit where hits overlap: a big-endian table also reads as a shifted
    # little-endian one, and a sine table holds the cosine table from a quarter circle on.
    size = {'i8': 1, 'i16': 2, 'i16be': 2, 'i32': 4, 'i32be': 4, 'f32': 4, 'f64': 8}
    hits.sort(key=lambda h: (-h['matching'], h['function'] != 'sine', h['offset']))
    kept = []
    for h in hits:
        end = h['offset'] + h['matching'] * size[h['type']]
        inside = [k for k in kept if k['offset'] <= h['offset'] < k['offset'] + k['matching'] * size[k['type']]
                  or h['offset'] <= k['offset'] < end]
        if not inside:
            kept.append(h)
            continue
        k = inside[0]
        if k['function'] == 'sine' and h['function'] == 'cosine' and k['steps'] == h['steps'] \
                and h['offset'] - k['offset'] == k['steps'] // 4 * size[k['type']]:
            k['note'] = 'the cosine table is the same table from entry %d on' % (k['steps'] // 4)
    return sorted(kept, key=lambda h: (h['file'], h['offset']))


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    rows = []
    for p in [a for a in argv if not a.startswith('--')]:
        rows += scan(p)
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    for h in rows:
        quarter = ' (a quarter circle or less is stored)' if h['matching'] <= h['steps'] // 4 + 1 else ''
        print(f'{h["file"]}  0x{h["offset"]:08x}  {h["function"]} table: {h["steps"]} steps per circle, '
              f'scale {h["scale"]}, {h["type"]}, {h["rounding"]}; {h["matching"]} entries match{quarter}')
        if h.get('note'):
            print(f'      {h["note"]}')
        for i, got, want in h['differences']:
            print(f'      entry {i}: {got} in the file, {want} by the formula')
    if not rows:
        print('no sine or cosine table of the common shapes found')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
