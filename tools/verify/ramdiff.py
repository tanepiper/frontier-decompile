#!/usr/bin/env python3
"""Compare the game's memory frame by frame, from the original and from the port, and report the
first frame and the first addresses that differ.

Usage: ramdiff.py A B [--size N] [--base 0xADDR] [--map FILE] [--ignore 0xSTART-0xEND ...]
                      [--frames N] [--all]

A and B are each a folder of snapshots, one file per frame in name order (frame_0001.bin,
frame_0002.bin ...), or one file of snapshots one after another, each --size bytes. An
emulator's script can write the original's memory after each frame of an input recording. The
port writes its own copy of the same state, in the original's layout, after each frame of the
same inputs. The first frame where they differ is where the port went wrong. Later frames only
show the difference spreading.

--base is the address of the first byte (0x0000 for the NES's 2 KB, 0x7E0000 for the SNES's work
RAM). --map names the addresses: a text file of lines "ADDRESS SIZE NAME", such as
"0x0075 1 player_x". Community RAM maps (Data Crystal, TASVideos) are written in this form.
--ignore leaves out ranges that legitimately differ: the stack, a frame counter the port keeps
elsewhere, buffers the port does not use. --all lists every differing frame, not only the
first. Standard library only.
"""
import os
import sys


def snapshots(path, size):
    if os.path.isdir(path):
        for name in sorted(os.listdir(path)):
            with open(os.path.join(path, name), 'rb') as f:
                yield name, f.read()
    else:
        if not size:
            sys.exit(f'{path}: a single file needs --size (the bytes in one snapshot)')
        with open(path, 'rb') as f:
            i = 0
            while True:
                chunk = f.read(size)
                if len(chunk) < size:
                    return
                yield f'frame {i}', chunk
                i += 1


def load_map(path):
    out = []
    for line in open(path, encoding='utf-8', errors='replace'):
        parts = line.split(None, 2)
        if len(parts) == 3 and not line.lstrip().startswith('#'):
            try:
                out.append((int(parts[0], 16), int(parts[1], 0), parts[2].strip()))
            except ValueError:
                pass
    return out


def name_of(addr, entries):
    for start, size, name in entries:
        if start <= addr < start + size:
            return name if addr == start else f'{name}+{addr - start}'
    return ''


def ranges(diffs):
    out = []
    for d in diffs:
        if out and d == out[-1][1] + 1:
            out[-1][1] = d
        else:
            out.append([d, d])
    return out


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0
    opt = lambda name, default: argv[argv.index(name) + 1] if name in argv else default
    size = int(opt('--size', '0'), 0)
    base = int(opt('--base', '0'), 0)
    entries = load_map(opt('--map', '')) if '--map' in argv else []
    ignore = []
    for i, a in enumerate(argv):
        if a == '--ignore':
            lo, hi = argv[i + 1].split('-')
            ignore.append((int(lo, 16), int(hi, 16)))
    limit = int(opt('--frames', '0'))
    show_all = '--all' in argv

    frames = diverged = 0
    first = None
    per_addr = {}
    for (na, a), (nb, b) in zip(snapshots(argv[0], size), snapshots(argv[1], size)):
        if limit and frames >= limit:
            break
        frames += 1
        n = min(len(a), len(b))
        diffs = [i for i in range(n) if a[i] != b[i] and not any(lo <= base + i <= hi for lo, hi in ignore)]
        if not diffs:
            continue
        diverged += 1
        for i in diffs:
            per_addr[i] = per_addr.get(i, 0) + 1
        if first is None or show_all:
            label = 'first difference' if first is None else 'differs'
            first = first if first is not None else frames - 1
            print(f'{label} at frame {frames - 1} ({na} / {nb}): {len(diffs)} bytes')
            for lo, hi in ranges(diffs)[:16]:
                name = name_of(base + lo, entries)
                va, vb = a[lo:hi + 1].hex(' '), b[lo:hi + 1].hex(' ')
                where = f'0x{base + lo:x}' + (f'-0x{base + hi:x}' if hi > lo else '')
                print(f'  {where:16} {name:24} original {va[:47]:24}  port {vb[:47]}')
            if len(ranges(diffs)) > 16:
                print(f'  ... {len(ranges(diffs)) - 16} more ranges')
    if frames == 0:
        sys.exit('no snapshots to compare')
    if first is None:
        print(f'identical: {frames} frames')
        return 0
    print(f'\n{diverged} of {frames} frames differ, the first at frame {first}')
    print('addresses that differ most often:')
    for i, count in sorted(per_addr.items(), key=lambda kv: -kv[1])[:10]:
        print(f'  0x{base + i:x}  {name_of(base + i, entries):24} {count} frames')
    return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
