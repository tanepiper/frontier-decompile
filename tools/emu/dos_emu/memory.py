"""Memory: selectors and their bases, addresses, reading and writing, segments for scripts,
registers and the stack.
Part of DosEmu (see __init__.py); the methods use its state."""
import struct

from unicorn import UcError

from .types import FIRST_SELECTOR, FarPtr, GDT_BASE, REGS, descriptor


class Memory:
    def selector(self, n):
        """The selector of NE segment number `n` (1-based), as Ghidra numbers it."""
        return FIRST_SELECTOR + 8 * (n - 1)

    def linear(self, seg, off=0):
        if self.kind == 'NE':
            base = self.bases.get(seg & 0xFFF8) if hasattr(self, 'bases') else None
            return (((seg >> 3) << 16) if base is None else base) + off
        return (seg << 4) + off

    def set_descriptor(self, sel, base, limit=0xFFFF, access=0x92):
        """NE only: point selector `sel` at a linear base (mapping 64 KB there if needed)."""
        self.bases[sel & 0xFFF8] = base
        page = base & ~0xFFF
        try:
            self.uc.mem_map(page, ((base + 0x10000 + 0xFFF) & ~0xFFF) - page)
        except UcError:
            pass   # already mapped (low memory, or another selector's block)
        self.uc.mem_write(GDT_BASE + (sel & 0xFFF8), descriptor(base, limit, access))

    def new_selector(self, base=None, access=0x92):
        """NE only: a fresh selector, with its own 64 KB, or pointing at `base`."""
        if base is None:
            return self._new_selector(access)
        sel = self.next_sel
        self.next_sel += 8
        self.set_descriptor(sel, base, access=access)
        return sel

    def _new_selector(self, access, size=0x10000):
        sel = self.next_sel
        self.next_sel += 8
        base = self.linear(sel)
        self.uc.mem_map(base, 0x10000)
        self.uc.mem_write(GDT_BASE + sel, descriptor(base, size - 1, access))
        return sel

    def _selector_at(self, linear):
        """A data selector whose base is a fixed linear address (low memory is mapped)."""
        sel = self.next_sel
        self.next_sel += 8
        self.bases[sel] = linear
        self.uc.mem_write(GDT_BASE + sel, descriptor(linear, 0xFFFF, 0x92))
        return sel

    def addr(self, where):
        """A FarPtr from an export name, '1000:0028', 'FUN_1000_0028', a tuple or a FarPtr."""
        if isinstance(where, tuple):
            return FarPtr(*where)
        if where.upper() in self.exports:
            return self.exports[where.upper()]
        text = where.upper().removeprefix('FUN_').replace('_', ':')
        if ':' in text:
            seg, off = text.split(':')
            return FarPtr(int(seg, 16), int(off, 16))
        raise KeyError(f'{where}: not an export of {self.path} and not a seg:off address')

    def read(self, ptr, n):
        return bytes(self.uc.mem_read(self.linear(*ptr), n))

    def write(self, ptr, data):
        self.uc.mem_write(self.linear(*ptr), bytes(data))

    def u16(self, ptr):
        return struct.unpack('<H', self.read(ptr, 2))[0]

    def u32(self, ptr):
        return struct.unpack('<I', self.read(ptr, 4))[0]

    def malloc(self, n):
        """Room for `n` bytes (zeroed); returns a FarPtr. Each block stays inside one segment."""
        if n > 0x10000:
            raise ValueError('a 16-bit block is at most 64 KB')
        if self.heap_top + n > 0x10000:
            self.heap_seg = self._new_selector(0x92) if self.kind == 'NE' else self.heap_seg + 0x1000
            self.heap_top = 0
        ptr = FarPtr(self.heap_seg, self.heap_top)
        self.heap_top = (self.heap_top + n + 15) & ~15
        self.write(ptr, b'\0' * n)
        return ptr

    def new_segment(self, data=b''):
        """A fresh 64 KB segment (NE: a new data selector; MZ: 4096 paragraphs from the heap)
        holding `data` at offset 0, as a protected-mode GetMem of a large block returns it.
        Returns the selector or segment; use it as a FarPtr (seg, 0) or store it in a global."""
        if len(data) > 0x10000:
            raise ValueError('a segment holds at most 64 KB')
        if self.kind == 'NE':
            seg = self._new_selector(0x92)
        else:
            self.heap_seg += 0x1000
            seg = self.heap_seg
            self.heap_top = 0x10000   # the next malloc starts a new heap segment
        self.write(FarPtr(seg, 0), b'\0' * 0x10000 if self.kind == 'NE' else b'\0' * 0xFFFF)
        if data:
            self.write(FarPtr(seg, 0), data)
        return seg

    def bytes_in(self, data):
        ptr = self.malloc(len(data))
        self.write(ptr, data)
        return ptr

    def reg(self, name):
        return self.uc.reg_read(REGS[name])

    def set(self, **regs):
        for name, value in regs.items():
            self.uc.reg_write(REGS[name], value)

    def push(self, word):
        sp = (self.reg('sp') - 2) & 0xFFFF
        self.set(sp=sp)
        self.write((self.reg('ss'), sp), struct.pack('<H', word & 0xFFFF))

    def pop(self):
        sp = self.reg('sp')
        v = self.u16((self.reg('ss'), sp))
        self.set(sp=(sp + 2) & 0xFFFF)
        return v
