#!/usr/bin/env python3
"""Read Borland Turbo Debugger symbols (TDINFO, signature FB52) from a DOS or NE executable.

Usage: tdinfo.py BINARY COMMAND [ARGS]

Commands:
  summary            where the symbols are, their version and counts, and the modules (units)
  labels             TSV for labels/names.tsv: every procedure and global as seg:off, name, evidence
  procs [TEXT]       procedures: address, size, module, parameters and locals with their types
  globals [TEXT]     global variables and typed constants: address, size, type
  types [TEXT]       named types; records show each field with its offset (summed from sizes)
  lines [MODULE]     line numbers: address -> source file and line
  json OUT           everything above as one JSON file
  rust TYPE ...      Rust structs and readers for record types (and arrays of records), with a
                     comment on every field giving its Pascal type and offset
  rust-globals DIR   a module directory: one file per Pascal unit with its variables at their
                     data-segment offsets, types/ (one file per record type), and mod.rs's Globals with load/store to
                     and from a 64 KB image of the segment (checks ported code against the
                     original's memory after an emulated run)
  rust-locals PROC   a struct of a procedure's locals at their BP offsets, with load/store on an
                     image of the stack segment (for code nested in a big procedure)
  rust-reader        the small PascalReader / PascalWriter module the generated code uses

Borland Pascal and Borland C++ write these symbols when a program is linked with debug
information (TPC /V, BPC /V, TLINK /v), either appended to the program or in a .TDS file. Many
shipped games kept them. They give the original names of procedures, globals, parameters,
locals and types, and a map from code addresses to source lines: run this before tracing
anything by hand, and load the labels into Ghidra (scripts/ghidra-run.sh BINARY labels ...).

Addresses use Ghidra's numbering: NE segment N is selector 0x1000 + 8 * (N - 1); in an MZ
program the symbol's segment is relative to the load image, which Ghidra loads at 0x1000.

Format notes (from reading real files; Borland's own description is not public in full):
  - The block starts with the 16-bit magic 0x52FB and a version word (0x0300 = 3.0). A CodeView
    style "NBxx" trailer at the end of the file may point to it.
  - Tables follow the header and its extension, in this order: symbols (9 bytes), modules (16),
    source files (6), line numbers (4), scopes (12), segments (16), correlations (8), types (8),
    members (5, one more than the header's count), then the name pool (zero-terminated names,
    numbered from 1).
  - A type is one 8-byte slot: id, name, size, then 3 bytes whose meaning depends on the id.
    Subranges and arrays take a second slot (bounds, index type); type numbers are slot numbers.
  - Record fields (members) store a name and a type but no offset: field offsets here are the
    running sum of field sizes (Pascal records are not padded). Treat them as *inferred* and
    check one against the code before relying on it. Variant records are not decoded.
It does not read Turbo Debugger 4+ (32-bit) symbols, CodeView or Microsoft/Watcom formats.
"""
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ne import MZ_LOAD, load  # noqa: E402

TYPE_NAMES = {
    0x00: 'void', 0x01: 'lstr', 0x02: 'dstr', 0x03: 'string', 0x04: 'shortint', 0x05: 'integer',
    0x06: 'longint', 0x07: 'comp', 0x08: 'byte', 0x09: 'word', 0x0A: 'dword', 0x0B: 'uquad',
    0x0C: 'char', 0x0D: 'single', 0x0E: 'real', 0x0F: 'double', 0x10: 'extended', 0x11: 'bcd4',
    0x12: 'bcd8', 0x13: 'bcd10', 0x15: 'near pointer', 0x16: 'far pointer', 0x17: 'segment',
    0x1A: 'C array', 0x1C: 'array', 0x1E: 'record', 0x1F: 'union', 0x22: 'enum', 0x23: 'procedure',
    0x24: 'label', 0x25: 'set', 0x26: 'text', 0x27: 'file', 0x28: 'boolean', 0x29: 'enum',
    0x2A: 'pword', 0x2B: 'tbyte', 0x2D: 'object', 0x36: 'wordbool', 0x37: 'longbool',
}
TWO_SLOTS = {0x04, 0x05, 0x06, 0x08, 0x09, 0x0A, 0x0C, 0x1C}   # subranges and arrays
CLASSES = {0: 'static', 1: 'absolute', 2: 'local', 3: 'var', 4: 'register', 5: 'const',
           6: 'type', 7: 'tag'}


def find(data):
    """The file offset of the TDINFO header, or None."""
    if len(data) > 8 and data[-8:-6] == b'NB':
        base = len(data) - struct.unpack_from('<I', data, len(data) - 4)[0]
        for at in range(max(base, 0), min(base + 64, len(data) - 2)):
            if data[at:at + 2] == b'\xfbR':
                return at
    pos = data.find(b'\xfbR')
    while pos != -1:
        if valid(data, pos):
            return pos
        pos = data.find(b'\xfbR', pos + 1)
    return None


def valid(data, at):
    if at + 0x30 > len(data):
        return False
    version, names_size = struct.unpack_from('<HI', data, at + 2)
    nsym = struct.unpack_from('<H', data, at + 0x0E)[0]
    return version >> 8 in (2, 3) and 0 < names_size < len(data) - at and at + 0x30 + 9 * nsym < len(data)


class TDInfo:
    def __init__(self, path):
        self.exe = load(path)
        d = self.data = self.exe.data
        at = self.at = find(d)
        if at is None:
            raise SystemExit(f'{path}: no Turbo Debugger symbols (FB52) found')
        (_, self.version, names_size, n_names, n_types, n_members, n_symbols, _, n_modules, _,
         n_scopes, n_lines, n_files, n_segments, n_corr) = struct.unpack_from('<HHIHHHHHHHHHHHH', d, at)
        ext, = struct.unpack_from('<H', d, at + 0x2E)
        end = len(d) - 8 if d[-8:-6] == b'NB' else self._names_end(at, names_size)
        self.names = [''] + d[end - names_size:end].decode('latin-1').split('\0')
        p = at + 0x30 + ext
        def table(fmt, count):
            nonlocal p
            size = struct.calcsize(fmt)
            rows = [struct.unpack_from(fmt, d, p + size * i) for i in range(count)]
            p += size * count
            return rows
        self.symbols = table('<HHHHB', n_symbols)            # name, type, offset, segment, class bits
        self.modules = table('<HBBHHHHHH', n_modules)          # name, language, model, ...
        self.files = table('<HI', n_files)                     # name, timestamp
        self.lines = table('<HH', n_lines)                     # line, offset
        self.scopes = table('<HHHHHH', n_scopes)
        self.segments = table('<HHHHHHHH', n_segments)         # module, segment, offset, length, ...
        self.correlations = table('<HHHH', n_corr)             # segment record, file, first line, count
        self.types = table('<BHHBH', n_types)                  # id, name, size, byte, word
        self.type_raw_at = p - 8 * n_types
        self.members = table('<BHH', n_members + 1)            # flags, name, type
        self.counts = dict(names=n_names, types=n_types, members=n_members, symbols=n_symbols,
                           modules=n_modules, scopes=n_scopes, lines=n_lines, files=n_files,
                           segments=n_segments, correlations=n_corr)

    def _names_end(self, at, names_size):
        return len(self.data)   # appended at the very end when there is no NB trailer

    # ------------------------------------------------------------------ helpers

    def name(self, i):
        return self.names[i] if 0 < i < len(self.names) else ''

    def address(self, segment, offset):
        if self.exe.kind == 'NE':
            return f'{0x1000 + 8 * (segment - 1):04x}:{offset:04x}'
        return f'{MZ_LOAD + segment:04x}:{offset:04x}'

    def type(self, t):
        """Type number t (1-based) -> (id, name, size, byte, word), or None."""
        return self.types[t - 1] if 0 < t <= len(self.types) else None

    def type_slot(self, t):
        return self.data[self.type_raw_at + 8 * (t - 1):self.type_raw_at + 8 * t]

    def type_size(self, t):
        ty = self.type(t)
        return ty[2] if ty else 0

    def type_name(self, t, depth=0):
        ty = self.type(t)
        if ty is None:
            return f'type#{t}'
        tid, name, size, b, w = ty
        if name and depth:
            return self.name(name)
        kind = TYPE_NAMES.get(tid, f'type{tid:#x}')
        if depth > 4:
            return self.name(name) or kind
        if tid in (0x04, 0x05, 0x06, 0x08, 0x09, 0x0A, 0x0C) and t < len(self.types):
            lo, hi = struct.unpack('<ii', self.type_slot(t + 1))
            base = self.name(name) or kind
            return base if name else f'{lo}..{hi}'
        if tid == 0x1C and t < len(self.types):
            index, = struct.unpack_from('<H', self.type_slot(t + 1))
            return f'array[{self.type_name(index, depth + 1)}] of {self.type_name(w, depth + 1)}'
        if tid == 0x03:
            return f'string[{b}]' if b != 255 else 'string'
        if tid in (0x15, 0x16):
            return f'^{self.type_name(w, depth + 1)}' if w else 'pointer'
        if tid == 0x23:
            return 'function: ' + self.type_name(w, depth + 1) if w > 1 else 'procedure'
        if tid == 0x1E:
            return (self.name(name) if depth else '') or f'record({size} bytes)'
        return self.name(name) or kind

    def record_fields(self, t):
        """(offset, name, type, size) for a record type; offsets are inferred from sizes."""
        ty = self.type(t)
        if ty is None or ty[0] != 0x1E or not ty[4]:
            return []
        out, off, i = [], 0, ty[4] - 1
        while 0 <= i < len(self.members):
            flags, name, mt = self.members[i]
            if not flags & 0x40:
                msize = self.type_size(mt)
                out.append((off, self.name(name), self.type_name(mt, 1), msize))
                off += msize
            if flags & 0x80:
                break
            i += 1
        return out

    def module_of_symbol(self, index):
        """The unit of a data symbol: the unit of the nearest procedure before it in the symbol
        table (procedures have code addresses, which the segment records place exactly). This is
        inferred: the module records' own symbol indexes do not line up with the units."""
        for i in range(index - 1, 0, -1):
            name, t, off, seg, bits = self.symbols[i - 1]
            ty = self.type(t)
            if bits & 7 == 0 and seg and ty and ty[0] == 0x23:
                return self.module_of(seg, off)
        return ''

    def module_of(self, segment, offset):
        for m, seg, start, length, *_ in self.segments:
            if seg == segment and start <= offset < start + length:
                return self.name(self.modules[m - 1][0]) if 0 < m <= len(self.modules) else ''
        return ''

    def procedures(self):
        """Procedures: static symbols whose type is a procedure, with sizes from the scopes."""
        out = []
        for i, (name, t, off, seg, bits) in enumerate(self.symbols, 1):
            if bits & 7 == 0 and seg and self.type(t) and self.type(t)[0] == 0x23:
                out.append(dict(index=i, name=self.name(name), segment=seg, offset=off,
                                address=self.address(seg, off), module=self.module_of(seg, off),
                                type=self.type_name(t), far=bool(bits & 0x10)))
        # Sizes: the distance to the next procedure or the end of the module's code range.
        by_seg = {}
        for p in out:
            by_seg.setdefault(p['segment'], []).append(p)
        for seg, procs in by_seg.items():
            procs.sort(key=lambda p: p['offset'])
            ends = [start + length for _, s, start, length, *_ in self.segments if s == seg]
            for a, b in zip(procs, procs[1:] + [None]):
                limit = b['offset'] if b else max(ends or [a['offset']])
                inside = [e for e in ends if a['offset'] < e <= limit]
                a['size'] = (min(inside) if inside else limit) - a['offset']
        self._attach_locals(out)
        return out

    def _attach_locals(self, procs):
        """Parameters and locals: the scope whose symbols follow a procedure belong to it."""
        by_index = {p['index']: p for p in procs}
        for first, count, _, _, off, length in self.scopes:
            owner = None
            for p in procs:
                if p['offset'] == off and length:
                    owner = p
                    break
            if owner is None:
                continue
            for i in range(first, first + count):
                if not 0 < i <= len(self.symbols):
                    continue
                name, t, o, seg, bits = self.symbols[i - 1]
                cls = bits & 7
                if cls in (2, 3, 4):
                    rel = o - 0x10000 if o >= 0x8000 else o
                    kind = 'param' if bits & 0x08 else 'local'
                    if cls == 3:
                        kind = 'var param'
                    owner.setdefault('vars', []).append(
                        dict(name=self.name(name), kind=kind, bp=rel, type=self.type_name(t, 1)))
        return by_index

    def globals(self):
        out = []
        procs = {(p['segment'], p['offset']) for p in self.procedures()}
        for i, (name, t, off, seg, bits) in enumerate(self.symbols, 1):
            if bits & 7 == 0 and seg and (seg, off) not in procs and self.type(t) \
                    and self.type(t)[0] not in (0x23, 0x24):
                out.append(dict(name=self.name(name), address=self.address(seg, off), segment=seg,
                                offset=off, size=self.type_size(t), type=self.type_name(t, 1),
                                module=self.module_of_symbol(i)))
        return out

    def named_types(self):
        out = []
        for name, t, off, seg, bits in self.symbols:
            if bits & 7 == 6 and self.type(t):
                fields = self.record_fields(t)
                out.append(dict(name=self.name(name), type=t, size=self.type_size(t),
                                kind=TYPE_NAMES.get(self.type(t)[0], hex(self.type(t)[0])),
                                definition=self.type_name(t), fields=fields))
        return out

    def line_map(self):
        """(address, file, line) for every line record, through the correlations."""
        out = []
        for segrec, file, first, count in self.correlations:
            if not 0 < segrec <= len(self.segments):
                continue
            seg = self.segments[segrec - 1][1]
            fname = self.name(self.files[file - 1][0]) if 0 < file <= len(self.files) else '?'
            for line, off in self.lines[first - 1:first - 1 + count]:
                out.append((self.address(seg, off), fname, line))
        return out


def element_record(td, t):
    """The record type inside t: t itself, or the element of an array (of arrays) of records."""
    for _ in range(4):
        ty = td.type(t)
        if ty is None:
            return None
        if ty[0] == 0x1E:
            return t
        if ty[0] != 0x1C:
            return None
        t = ty[4]
    return None


def show_record(td, t, indent):
    rec = element_record(td, t)
    if rec is None:
        return
    fields = td.record_fields(rec)
    for off, name, ty, size in fields:
        print(f'{indent}+{off:#06x}  {name}: {ty}  ({size})')
    total, size = sum(f[3] for f in fields), td.type_size(rec)
    if total != size:
        print(f'{indent}(fields add up to {total} bytes but the record is {size}: a variant part or '
              f'padding; check offsets in the code)')


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        sys.exit(2)
    path, cmd, args = argv[0], argv[1], argv[2:]
    if cmd.startswith('rust'):
        import td_rust
    if cmd == 'rust-reader':
        print(td_rust.reader(), end='')
        return
    td = TDInfo(path)
    needle = (args[0] if args else '').lower()
    if cmd == 'summary':
        print(f'{path}: Turbo Debugger symbols version {td.version >> 8}.{td.version & 0xFF} at file+{td.at:#x}')
        print('  ' + ', '.join(f'{v} {k}' for k, v in td.counts.items()))
        print(f'  {len(td.procedures())} procedures, {len(td.globals())} globals, {len(td.named_types())} named types')
        print('  modules (unit, source file, code ranges):')
        for i, m in enumerate(td.modules, 1):
            files = [td.name(td.files[f - 1][0]) for f in range(m[5], m[5] + m[6]) if 0 < f <= len(td.files)]
            ranges = [f'{td.address(s, o)}+{n:#x}' for mod, s, o, n, *_ in td.segments if mod == i and n]
            print(f'    {td.name(m[0]):<12} {", ".join(files):<16} {" ".join(ranges)}')
    elif cmd == 'labels':
        # A name used in two units (unit-private procedures) gets its unit: drawlines_fonthand.
        rows = [(p['address'], p['name'], p['module'], f'{p["size"]} bytes') for p in td.procedures()]
        rows += [(g['address'], g['name'], g['module'], g['type']) for g in td.globals()]
        seen = {}
        for _, name, *_ in rows:
            seen[name.lower()] = seen.get(name.lower(), 0) + 1
        print('# address\tlabel\tevidence (from tdinfo.py: the original debug symbols)')
        printed = set()
        for addr, name, module, what in rows:
            label = f'{name}_{module}' if seen[name.lower()] > 1 and module else name
            if label in printed:
                label = f'{label}_{addr.replace(":", "_")}'
            printed.add(label)
            print(f'{addr}\t{label}\tobserved: TD symbol {name}, unit {module or "?"}, {what}')
    elif cmd == 'procs':
        for p in td.procedures():
            if needle in p['name'].lower() or needle in p['module'].lower():
                print(f'{p["address"]}  {p["size"]:6d} bytes  {p["module"]:<10} {p["name"]}'
                      f'{"" if p["far"] else "  (near)"}')
                for v in p.get('vars', []):
                    print(f'        {v["kind"]:<9} [bp{v["bp"]:+d}] {v["name"]}: {v["type"]}')
    elif cmd == 'globals':
        for g in td.globals():
            if needle in g['name'].lower() or needle in g['module'].lower():
                print(f'{g["address"]}  {g["size"]:6d}  {g["module"]:<10} {g["name"]}: {g["type"]}')
    elif cmd == 'types':
        for t in td.named_types():
            if needle in t['name'].lower():
                print(f'type {t["name"]} = {t["definition"]}  ({t["size"]} bytes)')
                show_record(td, t['type'], '    ')
    elif cmd == 'lines':
        for addr, fname, line in td.line_map():
            if needle in fname.lower():
                print(f'{addr}\t{fname}:{line}')
    elif cmd == 'rust':
        print(td_rust.rust_for(td, args), end='')
    elif cmd == 'rust-locals':
        print(td_rust.rust_locals(td, args[0]), end='')
    elif cmd == 'rust-globals':
        if not args:
            sys.exit('rust-globals OUT_DIR: the module directory to write (for example src/globals)')
        units = td_rust.rust_globals(td, args[0])
        print(f'wrote {args[0]}: mod.rs, types/ and {len(units)} units')
    elif cmd == 'json':
        out = dict(version=td.version, counts=td.counts, procedures=td.procedures(), globals=td.globals(),
                   types=td.named_types(), lines=td.line_map())
        with open(args[0], 'w') as f:
            json.dump(out, f, indent=1)
        print(f'wrote {args[0]}')
    else:
        print(__doc__)
        sys.exit(2)


if __name__ == '__main__':
    main(sys.argv[1:])
