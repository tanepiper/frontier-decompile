"""Running code: calls with the Pascal or C convention, fragments between two addresses (or to
the first of several ends), run slices with interrupt delivery, import stubs, interrupt handlers, stopping.
Part of DosEmu (see __init__.py); the methods use its state."""
from unicorn import UC_HOOK_CODE, UcError

from .types import FarPtr, Long, ProgramStopped, TRAP


class Runner:
    def call(self, target, *args, conv='pascal', near=False, regs=None):
        """Run the routine at `target` with `args` and return AX. `regs` sets registers first:
        segment registers (ds=... for code that expects another data segment) and the others
        (ax=..., cx=... for runtime helpers that take their arguments in registers)."""
        ptr = self.addr(target)
        state = dict(self.regs)
        state.update(regs or {})
        for name in ('ss', 'sp', 'ds', 'es'):
            self.set(**{name: state[name]})
        for a in (args if conv == 'pascal' else reversed(args)):
            if isinstance(a, tuple):
                self.push(a[0]); self.push(a[1])
            elif isinstance(a, Long):
                self.push(a >> 16); self.push(a)
            else:
                self.push(a)
        if near:
            raise NotImplementedError('near routines: call the far routine that calls it, or '
                                      'write a far wrapper (push cs; call near; retf) into free memory')
        self.push(self.stub_seg)
        self.push(TRAP)
        self.set(**{k: v for k, v in (regs or {}).items() if k not in ('ss', 'sp', 'ds', 'es')})
        self._run(ptr)
        return self.reg('ax')

    def run_range(self, start, end, regs=None):
        """Run the code from `start` until execution reaches `end` (both as for call): for a
        fragment inside a big procedure, such as inline drawing code in a main loop. Imports
        and interrupts are handled as in call. It starts afresh: SS:SP, DS and ES are set to the
        start-up values, then `regs` (so set ss/sp/bp in `regs` for a fragment that uses its
        procedure's frame). To carry on a program that is already running (frame after frame
        of a headless run), use run_to: run_range would move the stack back to its start."""
        state = dict(self.regs)
        state.update(regs or {})
        for name, value in state.items():
            self.set(**{name: value})
        self._run_until(self.addr(start), end)

    def run_to(self, end):
        """Carry on from the current CS:IP, with every register as it is, until execution
        reaches `end`. For a running program: run_range(entry, a), then run_to(b), run_to(a)..."""
        self._run_until((self.reg('cs'), self.reg('ip')), end)

    def run_to_any(self, ends):
        """Like run_to, for several possible ends (a fragment that can finish in more than one
        place, or "the next fragment or the next frame, whichever comes first"): carry on until
        execution reaches one of `ends` and return its index in `ends`. An end at the current
        CS:IP is reached at once: step past it first (step) to search for the next one."""
        return self._run_until((self.reg('cs'), self.reg('ip')), *ends)

    def step(self):
        """Run one instruction from the current CS:IP (no interrupt delivery, no stubs)."""
        self.uc.emu_start(self.reg('ip'), 0, count=1)   # unicorn runs with IP as the address
        self.set(cs=self.reg('cs'))

    def _run_until(self, ptr, *ends):
        hit, hooks = [], []
        for i, end in enumerate(ends):
            at = self.linear(*self.addr(end))
            def on_end(uc, address, size, _, i=i):
                hit.append(i)
                self.pending = 'return'
                uc.emu_stop()
            hooks.append(self.uc.hook_add(UC_HOOK_CODE, on_end, begin=at, end=at))
        try:
            self._run(ptr)
        finally:
            for h in hooks:
                self.uc.hook_del(h)
        return hit[0] if hit else None

    def call_long(self, target, *args, **kw):
        """Like call, for a routine that returns a 32-bit value in DX:AX."""
        ax = self.call(target, *args, **kw)
        return (self.reg('dx') << 16) | ax

    def _run(self, ptr):
        self.error = None
        cs, ip = ptr
        budget = self.max_steps
        while True:
            self.pending = None
            self.set(cs=cs)
            count = min(budget, self.slice) if self.slice else budget
            try:
                self.uc.emu_start(ip, 0, count=count)
            except UcError as e:
                raise RuntimeError(self.error or f'{e} at {self.reg("cs"):04x}:{self.reg("ip"):04x}') from None
            if self.error:
                raise RuntimeError(self.error)
            if self.pending == 'stop':
                raise ProgramStopped(self.stop_value)
            if self.pending is None:
                budget -= count
                if self.slice and budget > 0:
                    # A slice ended: let the environment act (a timer interrupt), go on.
                    if self.on_slice:
                        self.on_slice(self)
                    cs, ip = self.reg('cs'), self.reg('ip')
                    continue
                raise RuntimeError(f'stopped after {self.max_steps} instructions at '
                                   f'{self.reg("cs"):04x}:{self.reg("ip"):04x} (an endless loop, '
                                   f'or raise max_steps)')
            if self.pending == 'return':
                return
            key, handler = self.pending
            ret_ip, ret_cs = self.pop(), self.pop()
            self.push(ret_cs); self.push(ret_ip)
            self.calls.append((key, f'{ret_cs:04x}:{ret_ip:04x}'))
            if self.verbose:
                print(f'  import {key} from {ret_cs:04x}:{ret_ip:04x}')
            pop = handler(self) or 0
            ip, cs = self.pop(), self.pop()
            self.set(sp=(self.reg('sp') + pop) & 0xFFFF)

    def interrupt(self, seg, off):
        """Deliver a hardware interrupt now, as the CPU does: push FLAGS, CS, IP, clear IF and
        TF, and continue at seg:off (the handler ends with IRET). Call it between slices
        (on_slice); it does nothing while interrupts are disabled (returns False)."""
        flags = self.reg('flags')
        if not flags & 0x200:
            return False
        self.push(flags)
        self.push(self.reg('cs'))
        self.push(self.reg('ip'))
        self.set(flags=flags & ~0x300)
        self.set(cs=seg, ip=off)
        return True

    def stop(self, value=None):
        """From a handler: end the current run (call or run_range raises ProgramStopped with
        `value`), for example when the program exits through INT 21h AH=4Ch."""
        self.pending = 'stop'
        self.stop_value = value
        self.uc.emu_stop()

    def _on_stub(self, uc, address, size, _):
        off = address - self.linear(self.stub_seg)
        if off == TRAP:
            self.pending = 'return'
            uc.emu_stop()
            return
        key = self.imports.get(off)
        handler = self.stubs.get(key)
        if handler is None and key:
            mod, _, what = key.partition('.')
            handler = self.stubs.get(f'{mod}.{what.upper()}')
        if handler is None:
            sp = self.reg('sp')
            ret = FarPtr(self.u16((self.reg('ss'), sp + 2)), self.u16((self.reg('ss'), sp)))
            self.error = f'unhandled import {key} called from {ret!r}: add stubs={{"{key}": handler}}'
            uc.emu_stop()
            return
        self.pending = (key, handler)
        uc.emu_stop()

    def _on_interrupt(self, uc, number, _):
        handler = self.interrupts.get(number)
        where = f'{self.reg("cs"):04x}:{self.reg("ip") - 2:04x}'
        if self.verbose:
            print(f'  int {number:02x}h ah={self.reg("ax") >> 8:02x}h at {where}')
        if handler is None:
            self.error = (f'unhandled interrupt {number:02X}h (AH={self.reg("ax") >> 8:02X}h) at {where}: '
                          f'add interrupts={{0x{number:02x}: handler}}')
            uc.emu_stop()
            return
        handler(self)

    def _on_bad_mem(self, uc, access, address, size, value, _):
        self.error = (f'bad memory access at linear {address:#x} ({size} bytes) from '
                      f'{self.reg("cs"):04x}:{self.reg("ip"):04x}')
        return False
