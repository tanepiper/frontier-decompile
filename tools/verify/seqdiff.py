#!/usr/bin/env python3
"""Compare two traces line by line and show the first place they differ.

Usage: seqdiff.py ORIGINAL.trace PORT.trace [--context N] [--field N]

A trace has one event per line, `label<TAB>where` (tools/emu/call_trace.py writes the
original's; make the port write the same format, with its own file:line as `where`). By default
only the first field (the label and arguments) is compared, since the call sites differ by
nature; `--field 0` compares whole lines. Prints the matching count, then the first divergence
with N lines of context (default 5) from both sides, so you can see which call site in the
original and which line in the port disagree. Exit status 1 when they differ.
"""
import sys


def main(argv):
    context, field = 5, 1
    for opt in ('--context', '--field'):
        if opt in argv:
            i = argv.index(opt)
            if opt == '--context':
                context = int(argv[i + 1])
            else:
                field = int(argv[i + 1])
            argv = argv[:i] + argv[i + 2:]
    a = open(argv[0]).read().splitlines()
    b = open(argv[1]).read().splitlines()
    key = (lambda l: l) if field == 0 else (lambda l: l.split('\t')[0])
    n = min(len(a), len(b))
    first = next((i for i in range(n) if key(a[i]) != key(b[i])), None)
    if first is None and len(a) == len(b):
        print(f'identical: {len(a)} events')
        return 0
    if first is None:
        first = n
        print(f'the first {n} events match; then only {"the original" if len(a) > n else "the port"} goes on '
              f'({len(a)} original, {len(b)} port)')
    else:
        print(f'the first {first} events match; event {first} differs')
    lo = max(0, first - context)
    for i in range(lo, min(max(len(a), len(b)), first + context + 1)):
        mark = '>>' if i == first else '  '
        left = a[i] if i < len(a) else '(end)'
        right = b[i] if i < len(b) else '(end)'
        print(f'{mark} {i:6d}  {left:<50} | {right}')
    return 1


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1:]))
