#!/usr/bin/env python3
"""Find skeletons and poses in a model or animation file: arrays of parent indices, and runs of
unit quaternions.

Usage: skeleton.py FILE [--min N]          parent arrays and quaternion runs, at least N joints (8)
       skeleton.py FILE --at 0xOFFSET --width 1|2|4 [--count N]    print one parent array as a tree

A skeleton is usually stored as a list of joints in which each joint names its parent by
index, and every parent comes before its children. The root has no parent, written as -1 (all
bits set). This tool looks for that shape: a run that starts at -1 and in which every later
entry is smaller than its own position. It tries 8-bit, 16-bit and 32-bit entries in both
byte orders. A run of zeros also has the shape, so a run needs at least three different parents
to count. A run must also look like a skeleton: at most two roots, a depth of at least three,
and no joint with more than sixteen children. Other data with the same shape (indices, flags)
rarely passes all three.

Many formats keep each joint as a record: a name, the parent index, then the pose. The parent
indices are then spread through the file, often at varying spacing when the names differ in
length. The tool also looks for these: a 32-bit parent index followed directly by a unit
quaternion, repeated in file order with every parent before its child. The printable text just
before each record is shown as the joint's name.

Joint rotations are quaternions: four floats whose squares add up to one. A bind pose, or one
sample of an animation, is a run of them at a fixed spacing, alone (16 bytes apart) or with a
translation and a scale (28 to 64 bytes apart). The tool reports runs of such quaternions with
the spacing. Compressed animation does not show here: quantised rotations are 16-bit integers
or packed into 48 or 64 bits. Read the decoder for those.

The joint names are often near the skeleton, in the same order. Strings such as Bip01, Bone,
pelvis, spine or head are listed with their offsets. Standard library only.
"""
import math
import re
import struct
import sys
from array import array

RIG = re.compile(rb'(?i)(bip0\d[ \w]*|bone[_ ]?\d*\w*|pelvis\w*|spine\d*\w*|head\w*|neck\w*|clavicle\w*|'
                 rb'[lr]_\w+|\w*(?:thigh|calf|forearm|upperarm|hand|foot|toe|finger)\w*)\x00')


def parent_runs(data, width, big, minimum):
    fmt = {1: 'b', 2: 'h', 4: 'i'}[width]
    end = '>' if big else '<'
    out = []
    for align in range(width):
        part = data[align:]
        n = len(part) // width
        vals = struct.unpack(f'{end}{n}{fmt}', part[:n * width])
        i = 0
        while i < n:
            if vals[i] != -1:
                i += 1
                continue
            j = i + 1
            while j < n and (vals[j] == -1 or 0 <= vals[j] < j - i):
                j += 1
            run = vals[i:j]
            parents = [p for p in run if p >= 0]
            if (len(run) >= minimum and len(set(parents)) >= 3 and any(p != k - 1 for k, p in enumerate(run) if p >= 0)
                    and plausible(run)):
                out.append((align + i * width, width, big, list(run)))
            i = j
    return out


def plausible(run):
    """A parent array that looks like a skeleton: few roots, some depth, narrow forks."""
    children = [0] * len(run)
    for p in run:
        if p >= 0:
            children[p] += 1
    return sum(1 for p in run if p < 0) <= 2 and max(depth(run)) >= 3 and max(children) <= 16


def unit_quat(data, at):
    if at + 16 > len(data):
        return False
    q = struct.unpack_from('<4f', data, at)
    s = sum(v * v for v in q)
    return abs(s - 1.0) < 1e-3 and not math.isnan(s)


def bone_records(data, minimum):
    """Joints stored as records: a 32-bit parent index directly followed by a unit quaternion."""
    anchors = []
    for at in range(0, len(data) - 20):
        p = struct.unpack_from('<i', data, at)[0]
        if -1 <= p < 1024 and unit_quat(data, at + 4):
            anchors.append((at, p))
    out, used = [], set()
    for k, (at, p) in enumerate(anchors):
        if p != -1 or at in used:
            continue
        run, offs, last = [p], [at], at
        for at2, p2 in anchors[k + 1:]:
            if at2 - last > 512:
                break
            if at2 - last < 20:
                continue
            if 0 <= p2 < len(run):
                run.append(p2)
                offs.append(at2)
                last = at2
            elif p2 == -1:
                break
        if len(run) >= minimum and plausible(run):
            used.update(offs)
            names = []
            for o in offs:
                m = re.search(rb'([\x20-\x7e]{2,64})\x00?$', data[max(0, o - 66):o].rstrip(b'\x00'))
                names.append(m.group(1).decode('ascii') if m else '')
            out.append((at, run, names))
    return out


def depth(run):
    d = []
    for p in run:
        d.append(0 if p < 0 else d[p] + 1)
    return d


def describe(run):
    d = depth(run)
    children = [0] * len(run)
    for p in run:
        if p >= 0:
            children[p] += 1
    roots = sum(1 for p in run if p < 0)
    leaves = sum(1 for c in children if c == 0)
    return f'{len(run)} joints, {roots} root(s), depth {max(d)}, {leaves} leaves, widest fork {max(children)}'


def quat_runs(data, minimum):
    out = []
    for align in range(4):
        out += [(off + align, stride, count, distinct) for off, stride, count, distinct in quat_runs_at(data[align:], minimum)]
    return sorted(out)


def quat_runs_at(data, minimum):
    n = len(data) // 4
    floats = array('f')
    floats.frombytes(data[:n * 4])
    if sys.byteorder == 'big':
        floats.byteswap()
    unit = bytearray(n)
    for i in range(n - 3):
        a, b, c, d = floats[i], floats[i + 1], floats[i + 2], floats[i + 3]
        s = a * a + b * b + c * c + d * d
        if abs(s - 1.0) < 1e-3 and not (math.isnan(s)):
            unit[i] = 1
    out = []
    taken = set()
    starts = [i for i in range(n) if unit[i]]
    for stride in (4, 7, 8, 10, 12, 13, 16):  # in floats: 16, 28, 32, 40, 48, 52, 64 bytes
        for start in starts:
            if start in taken or (start >= stride and unit[start - stride]):
                continue
            k = start
            while k < n and unit[k]:
                k += stride
            count = (k - start) // stride
            if count >= minimum:
                distinct = {tuple(round(floats[start + j * stride + e], 4) for e in range(4)) for j in range(count)}
                if len(distinct) >= 2:
                    out.append((start * 4, stride * 4, count, len(distinct)))
                    taken.update(range(start, k, stride))
    return sorted(out)


def tree(run, names=None):
    kids = {}
    for i, p in enumerate(run):
        kids.setdefault(p, []).append(i)

    def walk(i, pad):
        print(f'{pad}{i}' + (f' {names[i]}' if names and i < len(names) else ''))
        for c in kids.get(i, []):
            walk(c, pad + '  ')
    for r in kids.get(-1, []):
        walk(r, '')


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0
    with open(argv[0], 'rb') as f:
        data = f.read()
    opt = lambda name, default: argv[argv.index(name) + 1] if name in argv else default
    minimum = int(opt('--min', '8'))
    if '--at' in argv:
        at, width = int(opt('--at', '0'), 0), int(opt('--width', '1'))
        big = '--big' in argv
        runs = [r for r in parent_runs(data[at:], width, big, 1) if r[0] == 0]
        if not runs:
            print(f'no parent array at 0x{at:x}')
            return 1
        run = runs[0][3][:int(opt('--count', '100000'))]
        print(describe(run))
        tree(run)
        return 0

    found = []
    for width in (1, 2, 4):
        for big in ((False,) if width == 1 else (False, True)):
            found += parent_runs(data, width, big, minimum)
    # A 16-bit or 32-bit array also reads as a narrower one with zeros between its entries: keep the wider.
    found = [r for r in found if not any(o[1] > r[1] and o[0] <= r[0] < o[0] + len(o[3]) * o[1] for o in found)]
    found.sort()
    print(f'parent arrays (parents before children, root -1), at least {minimum} joints:')
    for off, width, big, run in found:
        order = '' if width == 1 else (' BE' if big else ' LE')
        print(f'  0x{off:08x}  {width * 8}-bit{order}  {describe(run)}')
        print(f'              {run[:24]}{" ..." if len(run) > 24 else ""}')
    if not found:
        print('  none')
    print(f'\njoint records (a parent index, then a rotation), at least {minimum} joints:')
    records = bone_records(data, minimum)
    for off, run, names in records:
        print(f'  0x{off:08x}  {describe(run)}')
        named = [n for n in names if n]
        if named:
            print(f'              names: {", ".join(named[:12])}{" ..." if len(named) > 12 else ""}')
    if not records:
        print('  none')
    print(f'\nruns of unit quaternions (float32), at least {minimum}:')
    quats = quat_runs(data, minimum)
    for off, stride, count, distinct in quats:
        print(f'  0x{off:08x}  {count} quaternions, {stride} bytes apart, {distinct} different')
    if not quats:
        print('  none')
    rig = [(m.start(), m.group(1).decode('ascii')) for m in RIG.finditer(data)]
    if rig:
        print(f'\njoint-like names: {len(rig)}')
        for off, name in rig[:20]:
            print(f'  0x{off:08x}  {name}')
        if len(rig) > 20:
            print(f'  ... {len(rig) - 20} more')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
