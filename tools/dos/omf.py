#!/usr/bin/env python3
"""Read Intel/Microsoft OMF object files and libraries (.OBJ, .LIB): the format of the DOS and
16-bit Windows compilers (Borland, Microsoft, Watcom). List their modules and public names,
or write a signature for each public function, for tools/triage/sigscan.py.

Usage: omf.py FILE [FILE ...]                     modules, segments and public names
       omf.py FILE [FILE ...] --sigs [--len N]    one signature per public code name

A game for DOS carries its compiler's runtime library inside the executable: printf, the
file functions, the long arithmetic helpers, the floating-point emulator. Those functions
have no names in the game, and reading them is wasted work. The library they came from
names them. An OMF library is a set of object modules. Each module holds the bytes of its code
(LEDATA records), the public names and their offsets (PUBDEF), and the fixups (FIXUPP): the
bytes the linker changes when it places the code. The fixups mark exactly the bytes to leave
as wildcards, so a signature made from a library is precise.

    python3 tools/dos/omf.py BC31/LIB/CL.LIB --sigs > derived/borland-cl.sigs
    python3 tools/triage/sigscan.py scan derived/borland-cl.sigs target/GAME.EXE --labels derived/runtime.tsv

Use the library of the same compiler, version and memory model as the game (re-inventory,
tools/dos/dosinfo.py, names the compiler). The libraries come with the compiler: use your own
copy. Iterated data (LIDATA) and function-level records (COMDAT) are counted, not read.
Standard library only.
"""
import json
import struct
import sys

LOCATION_BYTES = {0: 1, 1: 2, 2: 2, 3: 4, 4: 1, 5: 2, 9: 4, 11: 6, 13: 4}


class Module:
    def __init__(self, name):
        self.name = name
        self.lnames = ['']
        self.segments = []               # [name, class, length, bytes, fixup mask]
        self.publics = []                # (name, segment index, offset)
        self.skipped = {}


def index(data, at):
    b = data[at]
    if b & 0x80:
        return ((b & 0x7F) << 8) | data[at + 1], at + 2
    return b, at + 1


def pstring(data, at):
    n = data[at]
    return data[at + 1:at + 1 + n].decode('latin-1'), at + 1 + n


def records(data):
    """(type, body) for each record; a library's padding between modules is skipped."""
    at, page = 0, 0
    while at + 3 <= len(data):
        kind = data[at]
        length = struct.unpack_from('<H', data, at + 1)[0]
        body = data[at + 3:at + 3 + length - 1]
        if kind == 0xF0:                  # library header: the record length sets the page size
            page = length + 3
        yield kind, body
        at += 3 + length
        if kind in (0x8A, 0x8B) and page:  # MODEND: the next module starts on a page boundary
            at = (at + page - 1) // page * page
        if kind == 0xF1:
            break


def parse(data):
    modules, mod, last = [], None, None
    for kind, body in records(data):
        if kind == 0x80:                  # THEADR
            mod = Module(pstring(body, 0)[0])
            modules.append(mod)
        elif mod is None:
            continue
        elif kind == 0x96:                # LNAMES
            at = 0
            while at < len(body):
                name, at = pstring(body, at)
                mod.lnames.append(name)
        elif kind in (0x98, 0x99):        # SEGDEF
            acbp = body[0]
            at = 1 + (3 if acbp >> 5 == 0 else 0)
            length = struct.unpack_from('<I' if kind == 0x99 else '<H', body, at)[0]
            if kind == 0x98 and acbp & 2:
                length = 0x10000           # the "big" bit: a full 64 KB segment
            at += 4 if kind == 0x99 else 2
            name_i, at = index(body, at)
            class_i, at = index(body, at)
            mod.segments.append([mod.lnames[name_i], mod.lnames[class_i], length,
                                 bytearray(length), [False] * length])
        elif kind in (0x90, 0x91):        # PUBDEF
            _, at = index(body, 0)
            seg, at = index(body, at)
            if seg == 0:
                at += 2
            while at < len(body):
                name, at = pstring(body, at)
                off = struct.unpack_from('<I' if kind == 0x91 else '<H', body, at)[0]
                at += 4 if kind == 0x91 else 2
                _, at = index(body, at)
                mod.publics.append((name, seg, off))
        elif kind in (0xA0, 0xA1):        # LEDATA
            seg, at = index(body, 0)
            off = struct.unpack_from('<I' if kind == 0xA1 else '<H', body, at)[0]
            at += 4 if kind == 0xA1 else 2
            chunk = body[at:]
            s = mod.segments[seg - 1]
            s[3][off:off + len(chunk)] = chunk
            last = (seg, off, kind)
        elif kind in (0x9C, 0x9D) and last:   # FIXUPP for the last LEDATA
            fixups(body, mod.segments[last[0] - 1], last[1], kind == 0x9D)
        elif kind in (0xA2, 0xA3, 0xC2, 0xC3):
            name = {0xA2: 'LIDATA', 0xA3: 'LIDATA', 0xC2: 'COMDAT', 0xC3: 'COMDAT'}[kind]
            mod.skipped[name] = mod.skipped.get(name, 0) + 1
    return modules


def fixups(body, seg, base, big):
    at = 0
    while at < len(body):
        b = body[at]
        if not b & 0x80:                  # THREAD: method, then an index for methods 0 to 2
            method = (b >> 2) & 7
            at += 1
            if method < 3:
                _, at = index(body, at)
            continue
        locat = (b << 8) | body[at + 1]
        location = (locat >> 10) & 0xF
        offset = locat & 0x3FF
        fix = body[at + 2]
        at += 3
        if not fix & 0x80 and ((fix >> 4) & 7) < 3:   # frame datum
            _, at = index(body, at)
        if not fix & 0x08:                             # target datum
            _, at = index(body, at)
        if not fix & 0x04:                             # target displacement
            at += 4 if big else 2
        for k in range(LOCATION_BYTES.get(location, 2)):
            if base + offset + k < len(seg[4]):
                seg[4][base + offset + k] = True


def signatures(modules, length):
    out, seen = [], {}
    for mod in modules:
        for name, seg, off in mod.publics:
            if not seg or seg > len(mod.segments):
                continue
            s = mod.segments[seg - 1]
            if 'CODE' not in s[1].upper() and 'TEXT' not in s[0].upper():
                continue
            ends = sorted(o for n, sg, o in mod.publics if sg == seg and o > off)
            end = min([off + length, s[2]] + ends[:1])
            body = ['??' if s[4][i] else f'{s[3][i]:02X}' for i in range(off, end)]
            if sum(b != '??' for b in body) < 6:
                continue
            pattern = ' '.join(body)
            seen.setdefault(pattern, []).append(name)
            out.append((name, pattern, mod.name))
    return [(n, p, m, seen[p]) for n, p, m in out]


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    length = int(argv[argv.index('--len') + 1]) if '--len' in argv else 32
    files = [a for i, a in enumerate(argv) if not a.startswith('--') and (i == 0 or argv[i - 1] != '--len')]
    modules = []
    for f in files:
        with open(f, 'rb') as fh:
            modules += parse(fh.read())
    if '--sigs' in argv:
        print(f'# signatures from {", ".join(files)} by tools/dos/omf.py')
        for name, pattern, module, same in signatures(modules, length):
            if len(same) > 1 and same[0] != name:
                continue                          # the same bytes as an earlier name: say it once
            alias = f'  # also {", ".join(same[1:])}' if len(same) > 1 else ''
            print(f'{name}: {pattern}{alias}' if not alias else f'# {module}{alias}\n{name}: {pattern}')
        return 0
    if '--json' in argv:
        print(json.dumps([{'module': m.name, 'segments': [(s[0], s[1], s[2]) for s in m.segments],
                           'publics': m.publics, 'skipped': m.skipped} for m in modules], indent=2))
        return 0
    for m in modules:
        segs = ', '.join(f'{s[0]}({s[1]}) {s[2]}' for s in m.segments)
        print(f'{m.name}: {segs}' + (f'  [not read: {m.skipped}]' if m.skipped else ''))
        for name, seg, off in m.publics:
            print(f'    {name:32s} {m.segments[seg - 1][0] if seg else "abs"}:{off:04x}')
    print(f'{len(modules)} modules, {sum(len(m.publics) for m in modules)} public names')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
