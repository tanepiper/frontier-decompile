"""Read MZ (DOS) and NE (16-bit protected mode, Windows 3.x) executables: the shared parser
behind dosinfo.py and readseg.py. Standard library only.

    from ne import load
    exe = load('game.exe')      # an MZ or NE object; exe.kind is 'MZ', 'NE', 'LE', 'LX' or 'PE'
    exe.segments                # NE: list of Segment (number, selector, file offset, length, flags)
    exe.file_offset(0x1008, 4)  # file offset of selector:offset (NE) or segment:offset (MZ)

Addresses follow Ghidra: NE segment N has selector 0x1000 + 8 * (N - 1); an MZ image is loaded
at segment 0x1000. It does not parse resources, LE/LX internals or overlays (it only reports
that they are there).
"""
import struct

FIRST_SELECTOR = 0x1000
MZ_LOAD = 0x1000


class Segment:
    def __init__(self, number, offset, length, flags, minalloc, relocs):
        self.number, self.offset, self.length = number, offset, length
        self.flags, self.minalloc, self.relocs = flags, minalloc, relocs
        self.selector = FIRST_SELECTOR + 8 * (number - 1)
        self.is_data = bool(flags & 1)

    def describe(self):
        bits = ['DATA' if self.is_data else 'CODE']
        bits += [name for bit, name in ((0x10, 'movable'), (0x20, 'shared'), (0x40, 'preload'),
                                        (0x1000, 'discardable')) if self.flags & bit]
        return ' '.join(bits)


class Executable:
    def __init__(self, data):
        self.data = data
        self.kind = 'MZ'
        (self.last, self.pages, self.nrel, self.hdr_paras, self.minalloc, self.maxalloc, self.ss,
         self.sp, _, self.ip, self.cs, self.relat, self.overlay) = struct.unpack_from('<13H', data, 2)
        self.image_end = (self.pages - 1) * 512 + (self.last or 512) if self.pages else len(data)
        self.lfanew = 0
        if self.relat >= 0x40 and len(data) >= 0x40:
            self.lfanew, = struct.unpack_from('<I', data, 0x3C)
            sig = data[self.lfanew:self.lfanew + 2] if self.lfanew + 2 <= len(data) else b''
            self.kind = {b'NE': 'NE', b'LE': 'LE', b'LX': 'LX', b'PE': 'PE'}.get(sig, 'MZ')
        self.segments, self.modules, self.exports, self.entries = [], [], [], {}
        self.import_refs = {}   # 'MODULE.ordinal' or 'MODULE.NAME' -> number of references
        if self.kind == 'NE':
            self._parse_ne(self.lfanew)

    def _h(self, fmt, off):
        return struct.unpack_from(fmt, self.data, self.lfanew + off)

    def _pstr(self, pos):
        return self.data[pos + 1:pos + 1 + self.data[pos]].decode('latin-1')

    def _parse_ne(self, ne):
        d = self.data
        self.linker = self._h('<BB', 2)
        self.entry_off, self.entry_len = self._h('<HH', 4)
        self.flags, self.autodata, self.heap, self.stack = self._h('<HHHH', 0x0C)
        self.ne_ip, self.ne_cs, self.ne_sp, self.ne_ss = self._h('<HHHH', 0x14)
        nseg, nmod, nonres_len = self._h('<HHH', 0x1C)
        segtab, rsrc, restab, modtab, imptab = self._h('<HHHHH', 0x22)
        nonres, = self._h('<I', 0x2C)
        align, = self._h('<H', 0x32)
        self.target_os = d[ne + 0x36]
        self.align = align
        for i in range(nseg):
            off, length, flags, minalloc = self._h('<HHHH', segtab + 8 * i)
            off <<= align
            length = length or (0x10000 if off else 0)
            relocs = struct.unpack_from('<H', d, off + length)[0] if flags & 0x100 and off else 0
            self.segments.append(Segment(i + 1, off, length, flags, minalloc or 0x10000, relocs))
        for i in range(nmod):
            o, = self._h('<H', modtab + 2 * i)
            self.modules.append(self._pstr(ne + imptab + o))
        pos, ordinal = ne + self.entry_off, 1
        while pos < ne + self.entry_off + self.entry_len and d[pos]:
            count, kind = d[pos], d[pos + 1]
            pos += 2
            for _ in range(count):
                if kind == 0xFF:
                    self.entries[ordinal] = (d[pos + 3], struct.unpack_from('<H', d, pos + 4)[0])
                    pos += 6
                elif kind:
                    self.entries[ordinal] = (kind, struct.unpack_from('<H', d, pos + 1)[0])
                    pos += 3
                ordinal += 1
        names = {}
        for table, end in ((ne + restab, ne + modtab), (nonres, nonres + nonres_len)):
            pos, first = table, True
            while pos < min(end, len(d)) and d[pos]:
                name = self._pstr(pos)
                o, = struct.unpack_from('<H', d, pos + 1 + d[pos])
                pos += 1 + d[pos] + 2
                if first:
                    if table == ne + restab:
                        self.module_name = name
                    else:
                        self.description = name
                else:
                    names[o] = name
                first = False
        for o, (seg, off) in sorted(self.entries.items()):
            self.exports.append((o, FIRST_SELECTOR + 8 * (seg - 1), off, names.get(o, '')))
        for s in self.segments:
            if not s.relocs:
                continue
            at = s.offset + s.length + 2
            for i in range(s.relocs):
                src, flags, _, a, b = struct.unpack_from('<BBHHH', d, at + 8 * i)
                if flags & 3 in (1, 2):
                    mod = self.modules[a - 1] if 0 < a <= len(self.modules) else f'module{a}'
                    key = f'{mod}.{b}' if flags & 3 == 1 else f'{mod}.{self._pstr(ne + imptab + b)}'
                    self.import_refs[key] = self.import_refs.get(key, 0) + 1

    def file_offset(self, seg, off):
        """The file offset holding seg:off, as Ghidra addresses it, or None."""
        if self.kind == 'NE':
            for s in self.segments:
                if s.selector == seg:
                    return s.offset + off if s.offset and off < s.length else None
            return None
        linear = (seg - MZ_LOAD) * 16 + off
        at = self.hdr_paras * 16 + linear
        return at if 0 <= linear and at < self.image_end else None

    def selector(self, number):
        return FIRST_SELECTOR + 8 * (number - 1)


def load(path):
    data = open(path, 'rb').read()
    if data[:2] not in (b'MZ', b'ZM'):
        raise ValueError(f'{path}: not an MZ executable')
    return Executable(data)


def parse_address(text):
    """'1008:0004', 'DAT_1008_0004' or 'FUN_1000_0028' -> (segment, offset)."""
    t = text.upper()
    for prefix in ('DAT_', 'FUN_', 'LAB_', 'PTR_'):
        t = t.removeprefix(prefix)
    seg, off = t.replace('_', ':').split(':')
    return int(seg, 16), int(off, 16)
