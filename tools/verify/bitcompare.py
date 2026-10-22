#!/usr/bin/env python3
"""Compare two raw dumps value by value, bit for bit: ground truth against a port's output.

Usage: bitcompare.py TYPE EXPECTED.bin ACTUAL.bin [--show N]

TYPE is f32, f64, i32, u32, i16, u16 or u8. Floats are compared by their bits, so 0.0 and -0.0
differ and NaNs compare equal only when identical, which is what bit-exact means. Prints the
match count, the first N mismatches with their index (default 10) and, for floats, the largest
difference in units in the last place (ULP): 1 ULP usually means one operation in a different
order or precision; large differences mean a wrong formula or a wrong input. Exit status 1 on
any mismatch.
"""
import struct
import sys

FORMATS = {'f32': ('f', 'I'), 'f64': ('d', 'Q'), 'i32': ('i', 'i'), 'u32': ('I', 'I'),
           'i16': ('h', 'h'), 'u16': ('H', 'H'), 'u8': ('B', 'B')}


def main(argv):
    show = 10
    if '--show' in argv:
        i = argv.index('--show')
        show = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    kind, exp_path, act_path = argv
    value_fmt, bits_fmt = FORMATS[kind]
    size = struct.calcsize(value_fmt)
    exp, act = open(exp_path, 'rb').read(), open(act_path, 'rb').read()
    if len(exp) != len(act):
        print(f'sizes differ: {len(exp)} and {len(act)} bytes')
    n = min(len(exp), len(act)) // size
    ev = struct.unpack(f'<{n}{value_fmt}', exp[:n * size])
    eb = struct.unpack(f'<{n}{bits_fmt}', exp[:n * size])
    av = struct.unpack(f'<{n}{value_fmt}', act[:n * size])
    ab = struct.unpack(f'<{n}{bits_fmt}', act[:n * size])
    bad = [i for i in range(n) if eb[i] != ab[i]]
    print(f'{n - len(bad)} of {n} values match bit for bit')
    for i in bad[:show]:
        print(f'  [{i}] expected {ev[i]!r}, got {av[i]!r}')
    if bad and kind in ('f32', 'f64'):
        print(f'  largest difference: {max(abs(eb[i] - ab[i]) for i in bad)} ULP')
    return 1 if bad or len(exp) != len(act) else 0


if __name__ == '__main__':
    if len(sys.argv) < 4:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1:]))
