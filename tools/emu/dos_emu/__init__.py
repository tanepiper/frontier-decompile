"""Call functions inside a 16-bit DOS or Windows 3.x program (NE or MZ) under Unicorn.

The 16-bit counterpart of pe_emu.py: run one original routine with inputs you choose and read
its exact result (ground truth for a reimplementation), without DOS, DOSBox or the game.

    from dos_emu import DosEmu, Long, FarPtr
    emu = DosEmu('game.exe')                       # NE: protected mode, Ghidra's selectors
    emu.call('RNG_SEED', Long(1234))               # an exported name, '1000:000f', or (sel, off)
    print(emu.call('1000:0028', 100))              # AX; call_long() returns DX:AX
    buf = emu.bytes_in(b'abc')                     # a FarPtr into emulated memory
    emu.call('CHECKSUM', buf, 3)
    screen = emu.new_segment()                     # a whole 64 KB segment at offset 0
    emu.run_range(emu.entry, '1000:ca59')          # a running program: entry to a chosen point,
    emu.run_to_any(['1000:cb00', '1000:cb40'])     # then on to whichever comes first (its index)

NE programs (Borland Pascal 7 DPMI16, Windows 3.x) run in 16-bit protected mode. Segment N gets
selector 0x1000 + 8 * (N - 1), the same numbers Ghidra's NE loader uses, so an address from the
Ghidra export (`1000:0028`, `FUN_1000_0028`) is the address here. Relocations are applied:
selectors, far pointers and offsets to the program's own segments; imports (by ordinal or name)
become stubs. DS is the auto data segment; SS:SP come from the NE header (the auto data
segment when the header says so, as in Windows programs; Borland Pascal 7 DPMI16 programs
have a stack segment of their own).

MZ programs (real mode) load at segment 0x1000, as in Ghidra, with the PSP at 0x0FF0 and DS = ES
= PSP as DOS leaves them; SS:SP come from the header.

Arguments: an int is one word; `Long(v)` is a 32-bit value (high word pushed first); a `FarPtr`
or a (segment, offset) tuple is a far pointer (segment pushed first). The default convention is
`pascal`: arguments pushed left to right and removed by the callee, as Borland Pascal and the
Windows API do. Pass conv='cdecl' for C code (pushed right to left). Routines are called far
(a near routine returns with a plain `ret`: call it through a far caller).

Imports and interrupts: `stubs={'KERNEL.3': fn}` (module.ordinal or module.NAME) and
`interrupts={0x21: fn}` give Python handlers; each gets the emulator, reads and sets registers
with `emu.reg('ax')` / `emu.set(ax=...)`, and an import handler returns the number of argument
bytes to remove (a Pascal callee's `retf N`). An import or interrupt with no handler stops the
run with its name and the caller's address, so you know exactly what to stub next.

What it does not do: run DOS, load overlays or DLLs, emulate DPMI services, timers, VGA or sound
hardware, or the x87 (Borland's floating-point emulator fixups are left as they are). Use DOSBox-X
(scripts/dosbox-run.sh) when you need the whole program running. Needs `unicorn`.
"""
import struct

from unicorn import UC_HOOK_CODE, UC_HOOK_INTR, UC_HOOK_MEM_INVALID

from .loader import Loader
from .memory import Memory
from .run import Runner
from .trace import Tracer
from .types import (CONSTANT_IMPORTS, FIRST_SELECTOR, GDT_BASE, MZ_LOAD, TRAP, FarPtr, Long,  # noqa: F401
                    ProgramStopped, descriptor)


class DosEmu(Loader, Memory, Runner, Tracer):
    def __init__(self, path, stubs=None, interrupts=None, max_steps=50_000_000, verbose=False):
        self.data = open(path, 'rb').read()
        self.path = path
        self.stubs = dict(stubs or {})
        self.interrupts = dict(interrupts or {})
        self.max_steps = max_steps
        self.verbose = verbose
        self.calls = []          # (name, caller) of every import call, for verbose runs
        self.exports = {}        # NAME (upper case) or '#ordinal' -> FarPtr
        self.imports = {}        # stub offset -> 'MODULE.ordinal' or 'MODULE.NAME'
        self.error = None
        self.pending = None
        self.traces = []
        self.slice = None        # run in slices of this many instructions (for timers)
        self.on_slice = None     # called between slices with the emulator
        lfanew, = struct.unpack_from('<I', self.data, 0x3C) if len(self.data) > 0x40 else (0,)
        reloc_at, = struct.unpack_from('<H', self.data, 0x18)
        if reloc_at >= 0x40 and lfanew + 2 <= len(self.data) and self.data[lfanew:lfanew + 2] == b'NE':
            self.kind = 'NE'
            self._load_ne(lfanew)
        elif self.data[:2] in (b'MZ', b'ZM'):
            self.kind = 'MZ'
            self._load_mz()
        else:
            raise ValueError(f'{path}: not an MZ or NE program')
        self.uc.hook_add(UC_HOOK_CODE, self._on_stub, begin=self.linear(self.stub_seg, 0),
                         end=self.linear(self.stub_seg, 0xFFFF))
        self.uc.hook_add(UC_HOOK_INTR, self._on_interrupt)
        self.uc.hook_add(UC_HOOK_MEM_INVALID, self._on_bad_mem)


if __name__ == '__main__':
    print(__doc__)
