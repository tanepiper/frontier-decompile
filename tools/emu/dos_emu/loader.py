"""Loading NE (16-bit protected mode) and MZ (real mode) programs: segments, selectors as
Ghidra numbers them, relocations, imports as stubs, constant imports.
Part of DosEmu (see __init__.py); the methods use its state."""
import struct

from unicorn import UC_ARCH_X86, UC_MODE_16, UC_MODE_32, Uc
from unicorn.x86_const import UC_X86_REG_GDTR

from .types import CONSTANT_IMPORTS, FIRST_SELECTOR, FarPtr, GDT_BASE, MZ_HEAP, MZ_LOAD, MZ_STUBS, TRAP


class Loader:
    def _load_ne(self, ne):
        d = self.data
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        self.uc.mem_map(0, 0x110000)          # real-mode memory: BIOS data, VGA, text screens
        self.bases = {}                        # selectors whose base is not (sel >> 3) << 16
        self.uc.mem_map(GDT_BASE, 0x10000)
        self.uc.reg_write(UC_X86_REG_GDTR, (0, GDT_BASE, 0xFFFF, 0))
        h = lambda fmt, off: struct.unpack_from(fmt, d, ne + off)
        self.autodata, = h('<H', 0x0E)
        heap, stack = h('<HH', 0x10)
        nseg, nmod = h('<HH', 0x1C)
        segtab, _, restab, modtab, imptab = h('<HHHHH', 0x22)
        nonres_file, = h('<I', 0x2C)
        nonres_len, = h('<H', 0x20)
        entry_off, entry_len = h('<HH', 0x04)
        align, = h('<H', 0x32)
        self.next_sel = FIRST_SELECTOR
        self.segments = []   # (file offset, file length, flags, min alloc)
        for i in range(nseg):
            off, length, flags, minalloc = h('<HHHH', segtab + 8 * i)
            self.segments.append((off << align, length or (0x10000 if off else 0), flags, minalloc or 0x10000))
            sel = self._new_selector(0x92 if flags & 1 else 0x9A)
            if off:
                self.uc.mem_write(self.linear(sel), d[(off << align):(off << align) + (length or 0x10000)])
        self.stub_seg = self._new_selector(0x9A)
        self.uc.mem_write(self.linear(self.stub_seg, TRAP), b'\xf4')
        self.next_stub = 0
        self.heap_seg, self.heap_top = self._new_selector(0x92), 0
        self.constants = {}   # import key -> (selector or value), for CONSTANT_IMPORTS
        self.modules = []
        for i in range(nmod):
            o, = h('<H', modtab + 2 * i)
            self.modules.append(self._pstr(ne + imptab + o))
        # Entry table: ordinal -> (segment number, offset).
        entries, pos, ordinal = {}, ne + entry_off, 1
        while pos < ne + entry_off + entry_len and d[pos]:
            count, kind = d[pos], d[pos + 1]
            pos += 2
            for _ in range(count):
                if kind == 0:
                    pass
                elif kind == 0xFF:      # movable: flags, int 3Fh, segment, offset
                    entries[ordinal] = (d[pos + 3], struct.unpack_from('<H', d, pos + 4)[0])
                    pos += 6
                else:                   # fixed: flags, offset
                    entries[ordinal] = (kind, struct.unpack_from('<H', d, pos + 1)[0])
                    pos += 3
                ordinal += 1
        self.entries = entries
        for o, (seg, off) in entries.items():
            self.exports[f'#{o}'] = FarPtr(self.selector(seg), off)
        for table, end in ((ne + restab, ne + modtab), (nonres_file, nonres_file + nonres_len)):
            pos, first = table, True
            while pos < end and d[pos]:
                name = d[pos + 1:pos + 1 + d[pos]].decode('latin-1')
                o, = struct.unpack_from('<H', d, pos + 1 + d[pos])
                pos += 1 + d[pos] + 2
                if not first and o in entries:
                    self.exports[name.upper()] = self.exports[f'#{o}']
                first = False
        for i, (off, length, flags, _) in enumerate(self.segments, 1):
            if flags & 0x0100 and off:
                self._relocate_ne(i, off + length, ne, imptab)
        ds = self.selector(self.autodata) if self.autodata else self.heap_seg
        size = min(0x10000, self.segments[self.autodata - 1][3] + heap + stack) if self.autodata else 0x10000
        self.regs = {'ds': ds, 'es': ds, 'ss': ds, 'sp': (size - 2) & 0xFFFE}
        ip, cs, sp, ss = h('<HHHH', 0x14)
        if ss and ss != self.autodata:
            # The program has its own stack segment (Borland Pascal 7 DPMI16 does): use it,
            # or local variables land on top of the data segment's globals.
            self.regs.update(ss=self.selector(ss), sp=sp or 0xFFFE)
        self.entry = FarPtr(self.selector(cs), ip) if cs else None

    def _relocate_ne(self, segno, at, ne, imptab):
        d = self.data
        count, = struct.unpack_from('<H', d, at)
        base = self.linear(self.selector(segno))
        for i in range(count):
            src, flags, off = struct.unpack_from('<BBH', d, at + 2 + 8 * i)
            kind = flags & 3
            if kind == 0:     # internal reference
                seg, _, target = struct.unpack_from('<BBH', d, at + 2 + 8 * i + 4)
                if seg == 0xFF:
                    seg, target = self.entries[target]
                sel, target = self.selector(seg), target
            elif kind in (1, 2):   # import by ordinal or by name
                mod, what = struct.unpack_from('<HH', d, at + 2 + 8 * i + 4)
                module = self.modules[mod - 1]
                key = f'{module}.{what}' if kind == 1 else f'{module}.{self._pstr(ne + imptab + what)}'
                if key in CONSTANT_IMPORTS:
                    if key not in self.constants:
                        how, v = CONSTANT_IMPORTS[key]
                        self.constants[key] = v if how == 'value' else self._selector_at(v)
                    sel = target = self.constants[key]
                else:
                    sel, target = self.stub_seg, self._stub_for(key)
            else:              # OS fixup (floating-point emulator): left as it is
                continue
            while True:
                old, = struct.unpack('<H', self.uc.mem_read(base + off, 2))
                if src == 2:
                    patch = struct.pack('<H', sel)
                elif src == 3:
                    patch = struct.pack('<HH', target, sel)
                elif src == 5:
                    patch = struct.pack('<H', target)
                else:
                    raise ValueError(f'segment {segno}: relocation type {src} is not supported')
                if flags & 4:   # additive: add to what is there, no chain
                    if src == 5:
                        patch = struct.pack('<H', (old + target) & 0xFFFF)
                    elif src == 3:
                        patch = struct.pack('<HH', (old + target) & 0xFFFF, sel)
                    self.uc.mem_write(base + off, patch)
                    break
                self.uc.mem_write(base + off, patch)
                if old == 0xFFFF or old == off:
                    break
                off = old

    def _stub_for(self, key):
        for off, k in self.imports.items():
            if k == key:
                return off
        off = self.next_stub
        self.next_stub += 4
        self.uc.mem_write(self.linear(self.stub_seg, off), b'\x90\x90\x90\x90')
        self.imports[off] = key
        return off

    def _pstr(self, pos):
        return self.data[pos + 1:pos + 1 + self.data[pos]].decode('latin-1')

    def _load_mz(self):
        d = self.data
        last, pages, nrel, hdr, minalloc, maxalloc, ss, sp, _, ip, cs, relat = struct.unpack_from('<12H', d, 2)
        size = (pages - 1) * 512 + (last or 512) if pages else len(d)
        image = d[hdr * 16:size]
        self.uc = Uc(UC_ARCH_X86, UC_MODE_16)
        self.uc.mem_map(0, 0x110000)
        self.uc.mem_write(MZ_LOAD * 16, image)
        for i in range(nrel):
            off, seg = struct.unpack_from('<HH', d, relat + 4 * i)
            at = (MZ_LOAD + seg) * 16 + off
            v, = struct.unpack('<H', self.uc.mem_read(at, 2))
            self.uc.mem_write(at, struct.pack('<H', (v + MZ_LOAD) & 0xFFFF))
        psp = MZ_LOAD - 0x10
        self.uc.mem_write(psp * 16, b'\xcd\x20')
        self.stub_seg = MZ_STUBS
        self.uc.mem_write(self.linear(MZ_STUBS, TRAP), b'\xf4')
        self.next_stub = 0
        self.heap_seg, self.heap_top = MZ_HEAP, 0
        self.regs = {'ds': psp, 'es': psp, 'ss': MZ_LOAD + ss, 'sp': sp}
        self.entry = FarPtr(MZ_LOAD + cs, ip)
