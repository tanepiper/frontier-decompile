"""Record the calls an emulated function makes to chosen functions, in order, with call sites.

The usual use is the game's random number generator: most "almost right" reimplementations of
a simulation draw random numbers in a different order or count. Record the original's draws
here, record the port's draws the same way, and compare with tools/verify/seqdiff.py.

    from call_trace import CallTrace
    trace = CallTrace(emu, {emu.export('engine.dll', 'rng_next'): 'rng_next',
                            emu.export('engine.dll', 'rng_float'): 'rng_float'})
    emu.call(simulate, 1234, 600, out)
    trace.write('orig.trace')   # one line per call: "label<TAB>module+offset" of the return address

Only outermost calls are recorded: rng_float calling rng_next logs one line, as the port would
count one draw. Pass `args=N` to record the first N integer arguments too (`label(a, b)`).
"""
import struct

from unicorn import UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_R8, UC_X86_REG_R9, UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_RSP

ARG_REGS = [UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_R9]


class CallTrace:
    def __init__(self, emu, functions, args=0):
        self.emu = emu
        self.log = []
        self.depth_until = 0  # while inside a traced call, its return address's stack depth
        for addr, label in functions.items():
            emu.uc.hook_add(UC_HOOK_CODE, self._hook(label, args), begin=addr, end=addr)

    def _where(self, addr):
        for m in self.emu.modules.values():
            if m.contains(addr):
                return f'{m.name}+{addr - m.base:#x}'
        return f'{addr:#x}'

    def _hook(self, label, nargs):
        def on_entry(uc, address, size, _):
            sp = uc.reg_read(UC_X86_REG_RSP)
            if sp < self.depth_until:  # nested inside a traced call
                return
            ret = struct.unpack('<Q', bytes(uc.mem_read(sp, 8)))[0]
            text = label
            if nargs:
                text += '(' + ', '.join(str(uc.reg_read(r)) for r in ARG_REGS[:nargs]) + ')'
            # The return address names the call site exactly (the call instruction ends there).
            self.log.append(f'{text}\t{self._where(ret)}')
            self.depth_until = sp
            # Clear the depth mark once this call returns.
            def on_return(uc2, a2, s2, h):
                self.depth_until = 0
                uc2.hook_del(h[0])
            holder = []
            holder.append(uc.hook_add(UC_HOOK_CODE, lambda *a: on_return(*a[:3], holder), begin=ret, end=ret))
        return on_entry

    def write(self, path):
        with open(path, 'w') as f:
            f.write('\n'.join(self.log) + '\n')
