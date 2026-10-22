"""Call functions inside a Windows x64 game's EXE and DLLs under Unicorn, without running the game.

This is how you get *ground truth*: the exact outputs of an original function for inputs you
choose, to test a reimplementation against bit for bit.

    from pe_emu import Emu, F32
    emu = Emu('game.exe', ['engine.dll'])          # load, relocate if needed, link imports
    emu.hook_export('engine.dll', 'mem_alloc', lambda a: emu.malloc(a[0]))
    out = emu.malloc(4 * 600)
    emu.call(emu.export('game.exe', 'simulate'), 1234, 600, out)
    print(emu.f32s(out, 600)[-1])

What it does:

- maps every module at its preferred base (or relocates it when two overlap) and writes the
  image as the Windows loader would;
- links each import to an export of another loaded module by name or ordinal; any other import
  becomes a stub that runs a Python handler (common C runtime and kernel32 functions have
  working handlers in `CRT_STUBS`; an import with no handler stops the run with its name, so you
  know exactly what to stub next);
- gives the emulated code a stack, a bump-allocator heap and a TEB (GS segment), so thread-local
  reads and stack checks work;
- `call(addr, *args)` uses the Windows x64 convention: the first four integer or pointer
  arguments in rcx, rdx, r8, r9, floats wrapped in `F32` in xmm0-xmm3, the rest on the stack
  above 32 bytes of shadow space. It returns rax; `call_f32` returns xmm0 as a float.

What it does not do: run DllMain or TLS callbacks, create threads, open files or windows, or
talk to Direct3D. Hook or stub anything like that (`hook_function`). Needs `unicorn` and
`pefile` (scripts/setup.sh puts them in .venv).
"""
import struct

import pefile
from unicorn import UC_ARCH_X86, UC_HOOK_CODE, UC_HOOK_MEM_INVALID, UC_MODE_64, Uc, UcError
from unicorn.x86_const import (
    UC_X86_REG_GS_BASE, UC_X86_REG_R8, UC_X86_REG_R9, UC_X86_REG_RAX, UC_X86_REG_RCX,
    UC_X86_REG_RDX, UC_X86_REG_RIP, UC_X86_REG_RSP, UC_X86_REG_XMM0, UC_X86_REG_XMM1,
    UC_X86_REG_XMM2, UC_X86_REG_XMM3,
)

PAGE = 0x1000
STACK_BASE, STACK_SIZE = 0x7F0000000000, 0x4000000
HEAP_BASE, HEAP_SIZE = 0x7E0000000000, 0x80000000
STUB_BASE, STUB_SIZE = 0x7D0000000000, 0x100000
TEB_BASE = 0x7C0000000000
RETURN_TRAP = STUB_BASE + STUB_SIZE - 0x1000

INT_REGS = [UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_R9]
XMM_REGS = [UC_X86_REG_XMM0, UC_X86_REG_XMM1, UC_X86_REG_XMM2, UC_X86_REG_XMM3]
MASK64 = 0xFFFF_FFFF_FFFF_FFFF


def align(v, a=PAGE):
    return (v + a - 1) & ~(a - 1)


class F32(float):
    """A float argument, passed in an XMM register (as a 32-bit float)."""


class F64(float):
    """A double argument, passed in an XMM register."""


class Module:
    """One loaded image: its name, base, size and exports (name or '#ordinal' -> address)."""

    def __init__(self, name, pe, base, preferred):
        self.name, self.pe, self.base = name, pe, base
        self.preferred = preferred  # the image base in the file (Ghidra's addresses use it)
        self.size = align(pe.OPTIONAL_HEADER.SizeOfImage)
        self.exports = {}
        if hasattr(pe, 'DIRECTORY_ENTRY_EXPORT'):
            for e in pe.DIRECTORY_ENTRY_EXPORT.symbols:
                addr = base + e.address
                if e.name:
                    self.exports[e.name.decode()] = addr
                self.exports[f'#{e.ordinal}'] = addr

    def contains(self, addr):
        return self.base <= addr < self.base + self.size


class Emu:
    def __init__(self, exe_path, dll_paths=(), stubs=None, verbose=False):
        """Load `exe_path` and each of `dll_paths`, then link all imports.

        `stubs` adds or replaces import handlers by function name (see `CRT_STUBS`).
        """
        self.uc = Uc(UC_ARCH_X86, UC_MODE_64)
        self.verbose = verbose
        self.stubs = {}
        self.next_stub = STUB_BASE
        self.heap_top = HEAP_BASE
        self.handlers = dict(CRT_STUBS)
        self.handlers.update(stubs or {})
        self.calls = []  # import calls seen, for `verbose` runs
        self.uc.mem_map(STACK_BASE, STACK_SIZE)
        self.uc.mem_map(HEAP_BASE, HEAP_SIZE)
        self.uc.mem_map(STUB_BASE, STUB_SIZE)
        self.uc.mem_map(TEB_BASE, 0x10000)
        self.uc.reg_write(UC_X86_REG_GS_BASE, TEB_BASE)
        self.write(TEB_BASE + 0x30, struct.pack('<Q', TEB_BASE))                   # TEB self
        self.write(TEB_BASE + 0x08, struct.pack('<Q', STACK_BASE + STACK_SIZE))    # stack base
        self.write(TEB_BASE + 0x10, struct.pack('<Q', STACK_BASE))                 # stack limit
        self.modules = {}
        for path in [*dll_paths, exe_path]:
            self._load(path)
        for m in self.modules.values():
            self._link(m)
        self.uc.hook_add(UC_HOOK_CODE, self._on_stub, begin=STUB_BASE, end=STUB_BASE + STUB_SIZE)
        self.uc.hook_add(UC_HOOK_MEM_INVALID, self._on_bad_mem)

    # ------------------------------------------------------------------ loading

    def _load(self, path):
        pe = pefile.PE(path, fast_load=True)
        pe.parse_data_directories()
        if pe.FILE_HEADER.Machine != 0x8664:
            raise ValueError(f'{path}: not an x64 image (machine {pe.FILE_HEADER.Machine:#x})')
        name = path.replace('\\', '/').rsplit('/', 1)[-1].lower()
        preferred = base = pe.OPTIONAL_HEADER.ImageBase
        size = align(pe.OPTIONAL_HEADER.SizeOfImage)
        if any(_overlaps(base, size, m.base, m.size) for m in self.modules.values()):
            base = max(m.base + m.size for m in self.modules.values())
            base = align(base, 0x10000)
            pe.relocate_image(base)  # applies the base relocations, as the loader would
        self.uc.mem_map(base, size)
        self.write(base, pe.get_memory_mapped_image()[:pe.OPTIONAL_HEADER.SizeOfImage])
        self.modules[name] = Module(name, pe, base, preferred)
        return self.modules[name]

    def _link(self, m):
        for d in getattr(m.pe, 'DIRECTORY_ENTRY_IMPORT', []):
            dll = d.dll.decode().lower()
            target_module = self.modules.get(dll)
            for imp in d.imports:
                name = imp.name.decode() if imp.name else f'#{imp.ordinal}'
                if target_module and name in target_module.exports:
                    target = target_module.exports[name]
                else:
                    target = self._new_stub(f'{dll}!{name}', self.handlers.get(name))
                # pefile moves import addresses with relocate_image, so this is the slot's offset.
                self.write(imp.address - m.pe.OPTIONAL_HEADER.ImageBase + m.base, struct.pack('<Q', target))

    def module(self, name):
        return self.modules[name.lower()]

    def export(self, module, name):
        """The address of an exported function or variable."""
        return self.module(module).exports[name]

    def rva(self, module, rva):
        """An address from an offset in a module (Ghidra addresses minus the image base)."""
        return self.module(module).base + rva

    def va(self, module, ghidra_va):
        """A Ghidra address (at the preferred base) mapped to where the module really loaded."""
        m = self.module(module)
        return ghidra_va - m.preferred + m.base

    # ------------------------------------------------------------------ hooks

    def _new_stub(self, name, handler=None):
        addr = self.next_stub
        self.next_stub += 0x10
        self.write(addr, b'\xc3')  # ret; the code hook runs the handler first
        self.stubs[addr] = (name, handler)
        return addr

    def hook_function(self, addr, name, handler):
        """Replace the function at `addr`: `handler(int_args)` runs instead and its value is
        returned in rax. Use it for callees that touch the OS, threads, files or the GPU."""
        stub = self._new_stub(name, handler)
        self.write(addr, b'\x48\xb8' + struct.pack('<Q', stub) + b'\xff\xe0')  # mov rax, stub; jmp rax

    def hook_export(self, module, name, handler):
        self.hook_function(self.export(module, name), f'{module}!{name}', handler)

    def _on_stub(self, uc, address, size, _):
        if address == RETURN_TRAP:
            uc.emu_stop()
            return
        name, handler = self.stubs.get(address, (None, None))
        if name is None:
            return
        if handler is None:
            uc.emu_stop()
            raise RuntimeError(f'unhandled import {name}: add a handler with stubs={{...}}')
        args = [uc.reg_read(r) for r in INT_REGS]
        if self.verbose:
            self.calls.append(name)
        result = handler(args) if handler.__code__.co_argcount == 1 else handler(args, self)
        if result is not None:
            uc.reg_write(UC_X86_REG_RAX, result & MASK64)

    def _on_bad_mem(self, uc, access, address, size, value, _):
        rip = uc.reg_read(UC_X86_REG_RIP)
        where = next((f'{m.name}+{rip - m.base:#x}' for m in self.modules.values()
                      if m.contains(rip)), f'{rip:#x}')
        print(f'invalid memory access ({access}) at {address:#x}, size {size}, from {where}: '
              'usually a pointer field the emulated object is missing')
        return False

    # ------------------------------------------------------------------ memory

    def malloc(self, size, alignment=16):
        """Allocate zeroed memory on the emulated heap (never freed; runs are short)."""
        self.heap_top = align(self.heap_top, alignment)
        addr = self.heap_top
        self.heap_top += max(int(size), 1)
        if self.heap_top > HEAP_BASE + HEAP_SIZE:
            raise MemoryError('emulated heap exhausted')
        self.write(addr, bytes(int(size)))
        return addr

    def read(self, addr, n):
        return bytes(self.uc.mem_read(addr, n))

    def write(self, addr, data):
        self.uc.mem_write(addr, bytes(data))

    def u8(self, addr):
        return self.read(addr, 1)[0]

    def u32(self, addr):
        return struct.unpack('<I', self.read(addr, 4))[0]

    def i32(self, addr):
        return struct.unpack('<i', self.read(addr, 4))[0]

    def u64(self, addr):
        return struct.unpack('<Q', self.read(addr, 8))[0]

    def f32s(self, addr, n):
        return struct.unpack(f'<{n}f', self.read(addr, 4 * n))

    def cstr(self, addr, limit=4096):
        data = self.read(addr, limit)
        return data.split(b'\0', 1)[0].decode('latin-1')

    def bytes_in(self, data):
        """Copy `data` into emulated memory and return its address."""
        addr = self.malloc(len(data) or 1)
        self.write(addr, data)
        return addr

    # ------------------------------------------------------------------ calls

    def call(self, addr, *args, timeout_insns=0):
        """Call `addr` with the Windows x64 convention. Returns rax (an unsigned 64-bit value)."""
        sp = STACK_BASE + STACK_SIZE - 0x10000
        stack_args = []
        for i, a in enumerate(args):
            if i < 4:
                if isinstance(a, F32):
                    self.uc.reg_write(XMM_REGS[i], struct.unpack('<I', struct.pack('<f', a))[0])
                elif isinstance(a, F64):
                    self.uc.reg_write(XMM_REGS[i], struct.unpack('<Q', struct.pack('<d', a))[0])
                else:
                    self.uc.reg_write(INT_REGS[i], int(a) & MASK64)
            else:
                stack_args.append(a)
        sp -= 0x20 + 8 * len(stack_args) + 8
        sp &= ~0xF
        sp -= 8  # the return address the caller's `call` would push
        self.write(sp, struct.pack('<Q', RETURN_TRAP))
        for i, a in enumerate(stack_args):
            if isinstance(a, F32):
                v = struct.unpack('<I', struct.pack('<f', a))[0]
            elif isinstance(a, F64):
                v = struct.unpack('<Q', struct.pack('<d', a))[0]
            else:
                v = int(a)
            self.write(sp + 8 + 0x20 + 8 * i, struct.pack('<Q', v & MASK64))
        self.uc.reg_write(UC_X86_REG_RSP, sp)
        try:
            self.uc.emu_start(addr, RETURN_TRAP + 1, count=timeout_insns)
        except UcError as e:
            rip = self.uc.reg_read(UC_X86_REG_RIP)
            raise RuntimeError(f'emulation failed at rip {rip:#x}: {e}') from e
        return self.uc.reg_read(UC_X86_REG_RAX)

    def call_i32(self, addr, *args, **kw):
        v = self.call(addr, *args, **kw) & 0xFFFFFFFF
        return v - (1 << 32) if v & 0x80000000 else v

    def call_f32(self, addr, *args, **kw):
        self.call(addr, *args, **kw)
        bits = self.uc.reg_read(UC_X86_REG_XMM0) & 0xFFFFFFFF
        return struct.unpack('<f', struct.pack('<I', bits))[0]


def _overlaps(a, an, b, bn):
    return a < b + bn and b < a + an


# ---------------------------------------------------------------------- import stand-ins
# Handlers take the four integer argument registers (and optionally the Emu) and return rax.


def _malloc(a, emu):
    return emu.malloc(a[0])


def _calloc(a, emu):
    return emu.malloc(a[0] * a[1])


def _realloc(a, emu):
    new = emu.malloc(a[1])
    if a[0]:
        emu.write(new, emu.read(a[0], a[1]))  # may over-read a smaller block; fine on our heap
    return new


def _memset(a, emu):
    emu.write(a[0], bytes([a[1] & 0xFF]) * a[2])
    return a[0]


def _memcpy(a, emu):
    emu.write(a[0], emu.read(a[1], a[2]))
    return a[0]


def _strlen(a, emu):
    return len(emu.cstr(a[0]))


def _zero(_a):
    return 0


def _one(_a):
    return 1


CRT_STUBS = {
    # C runtime
    'malloc': _malloc, 'calloc': _calloc, 'realloc': _realloc, 'free': _zero,
    '_aligned_malloc': _malloc, '_aligned_free': _zero,
    'memset': _memset, 'memcpy': _memcpy, 'memmove': _memcpy, 'strlen': _strlen,
    # kernel32: locks and errors are no-ops in a single-threaded run
    'EnterCriticalSection': _zero, 'LeaveCriticalSection': _zero,
    'InitializeCriticalSection': _zero, 'InitializeCriticalSectionAndSpinCount': _one,
    'DeleteCriticalSection': _zero, 'GetLastError': _zero, 'SetLastError': _zero,
    'GetCurrentThreadId': _one, 'GetCurrentProcessId': _one,
}
