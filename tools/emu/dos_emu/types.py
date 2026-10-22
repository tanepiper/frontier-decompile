"""Names shared by the dos_emu modules: registers, address constants, argument types."""
import struct

from unicorn import (
    UC_ARCH_X86, UC_HOOK_CODE, UC_HOOK_INTR, UC_HOOK_MEM_INVALID, UC_MODE_16, UC_MODE_32, Uc, UcError,
)
from unicorn.x86_const import (
    UC_X86_REG_AX, UC_X86_REG_BP, UC_X86_REG_BX, UC_X86_REG_CS, UC_X86_REG_CX, UC_X86_REG_DI,
    UC_X86_REG_DS, UC_X86_REG_DX, UC_X86_REG_EIP, UC_X86_REG_ES, UC_X86_REG_FLAGS, UC_X86_REG_GDTR,
    UC_X86_REG_SI, UC_X86_REG_SP, UC_X86_REG_SS,
)

REGS = {'ax': UC_X86_REG_AX, 'bx': UC_X86_REG_BX, 'cx': UC_X86_REG_CX, 'dx': UC_X86_REG_DX,
        'si': UC_X86_REG_SI, 'di': UC_X86_REG_DI, 'bp': UC_X86_REG_BP, 'sp': UC_X86_REG_SP,
        'ds': UC_X86_REG_DS, 'es': UC_X86_REG_ES, 'ss': UC_X86_REG_SS, 'cs': UC_X86_REG_CS,
        'ip': UC_X86_REG_EIP, 'flags': UC_X86_REG_FLAGS}
FIRST_SELECTOR = 0x1000
GDT_BASE = 0x120000            # above the real-mode first megabyte (mapped for NE programs too)
TRAP = 0xFFF0                      # the return trap's offset in the stub segment
MZ_LOAD = 0x1000                   # where Ghidra and this emulator load an MZ image
MZ_STUBS, MZ_HEAP = 0xF000, 0x8000
# Windows/DPMI16 KERNEL exports whose value is a constant, not a function: a selector for a
# fixed linear address, or a number. Borland Pascal 7 DPMI16 and Windows 3.x programs use them.
CONSTANT_IMPORTS = {
    'KERNEL.114': ('value', 8),          # __AHINCR: the selector step of huge (tiled) memory
    'KERNEL.174': ('linear', 0xA0000),   # __A000H: VGA graphics memory
    'KERNEL.181': ('linear', 0xB0000),   # __B000H: monochrome text memory
    'KERNEL.182': ('linear', 0xB8000),   # __B800H: colour text memory
    'KERNEL.193': ('linear', 0x00400),   # __0040H: the BIOS data area
}


class ProgramStopped(Exception):
    """The program asked to stop (emu.stop, for example its exit call); .args[0] is the value."""


class Long(int):
    """A 32-bit argument: pushed as two words, high word first."""


class FarPtr(tuple):
    """A far pointer (segment or selector, offset)."""

    def __new__(cls, seg, off):
        return super().__new__(cls, (seg, off))

    seg = property(lambda self: self[0])
    off = property(lambda self: self[1])

    def __add__(self, n):
        return FarPtr(self[0], (self[1] + n) & 0xFFFF)

    def __repr__(self):
        return f'{self[0]:04x}:{self[1]:04x}'


def descriptor(base, limit, access):
    """An 8-byte GDT entry for a 16-bit segment (byte granular, D = 0)."""
    return struct.pack('<Q', (limit & 0xFFFF) | ((base & 0xFFFFFF) << 16) | (access << 40)
                       | (((limit >> 16) & 0xF) << 48) | ((base >> 24) << 56))
