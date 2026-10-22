"""Record which instructions read and write chosen memory while an emulated function runs.

This is taint analysis, done on the emulator instead of a live process with Pin or DynamoRIO:
mark some bytes (a buffer read from the save file, a global, a table), call the function, and
list each instruction that touched them.

    from mem_trace import MemTrace
    trace = MemTrace(emu, [(buf, len(data), 'save')], follow_copies=True)
    emu.call(load, buf, len(data), out)
    trace.write('derived/load.memtrace')   # "label<TAB>r|w<TAB>offset<TAB>size<TAB>module+offset"
    trace.readers()                           # {module+offset: count}, the code that read the bytes

With `follow_copies`, a write made by the same instruction that just read marked bytes (a
`rep movs`, an unrolled copy) marks the destination too, with the same label.

With `registers`, values are also followed through registers, as Pin-based taint tools do.
Capstone gives the registers each instruction reads and writes. An instruction that reads a
marked register or marked memory marks every register and every byte it writes. One that
reads nothing marked clears the registers it writes. Registers used only to form an address
do not pass their mark on, and `xor r, r` clears r. Memory written by unmarked values keeps
its mark, so the result can mark too much, never too little. `reg_log()` lists each
instruction that passed a mark on.
"""
import re

from unicorn import UC_HOOK_CODE, UC_HOOK_MEM_READ, UC_HOOK_MEM_WRITE
from unicorn.x86_const import UC_X86_REG_RIP

X86_OP_REG, X86_OP_MEM = 1, 3


def family(name):
    """The 64-bit register a register name belongs to: eax, ax, al and ah are all rax."""
    m = re.fullmatch(r'r(\d+)[dwb]?', name)
    if m:
        return 'r' + m.group(1)
    for base in ('ax', 'bx', 'cx', 'dx'):
        if name in (base, 'e' + base, 'r' + base, base[0] + 'l', base[0] + 'h'):
            return 'r' + base
    for base in ('si', 'di', 'bp', 'sp'):
        if name in (base, 'e' + base, 'r' + base, base + 'l'):
            return 'r' + base
    return name


class MemTrace:
    def __init__(self, emu, regions, follow_copies=False, registers=False):
        """regions: [(address, size, label)]."""
        self.emu = emu
        self.regions = [(a, a + n, label) for a, n, label in regions]
        self.follow = follow_copies
        self.log = []
        self._last_read = None  # (pc, label) of the latest read of marked bytes
        self.marked = {}        # register family -> label
        self._reg_log = []
        self._insn = None       # (pc, registers read, registers written) of the running instruction
        self._reads, self._writes = [], []
        emu.uc.hook_add(UC_HOOK_MEM_READ, self._on_access, 'r')
        emu.uc.hook_add(UC_HOOK_MEM_WRITE, self._on_access, 'w')
        if registers:
            import capstone
            self._cs = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
            self._cs.detail = True
            self._decoded = {}
            emu.uc.hook_add(UC_HOOK_CODE, self._on_code)

    def _where(self, addr):
        for m in self.emu.modules.values():
            if m.contains(addr):
                return f'{m.name}+{addr - m.base:#x}'
        return f'{addr:#x}'

    def _region(self, addr):
        for start, end, label in self.regions:
            if start <= addr < end:
                return start, label
        return None

    def _decode(self, uc, pc, size):
        if pc in self._decoded:
            return self._decoded[pc]
        insn = next(self._cs.disasm(bytes(uc.mem_read(pc, size)), pc), None)
        if insn is None:
            self._decoded[pc] = (set(), set())
            return self._decoded[pc]
        read, written = insn.regs_access()
        read = {family(insn.reg_name(r)) for r in read}
        written = {family(insn.reg_name(r)) for r in written}
        regs = [o for o in insn.operands if o.type == X86_OP_REG]
        addressing = set()
        for op in insn.operands:
            if op.type == X86_OP_MEM:
                addressing |= {family(insn.reg_name(r)) for r in (op.mem.base, op.mem.index) if r}
        # A register that only forms an address is not a value read, unless it is also an operand.
        read = (read - addressing) | (read & {family(insn.reg_name(o.reg)) for o in regs})
        if insn.mnemonic in ('xor', 'sub', 'pxor', 'xorps') and len(regs) == 2 and regs[0].reg == regs[1].reg:
            read = set()
        read.discard('rflags')
        written.discard('rflags')
        self._decoded[pc] = (read, written)
        return self._decoded[pc]

    def _finish(self):
        """Pass marks on for the instruction that just ran."""
        if self._insn is None:
            return
        pc, read, written = self._insn
        label = next((self.marked[r] for r in sorted(read) if r in self.marked), None)
        if label is None and self._reads:
            label = self._reads[0]
        for r in written:
            if label is None:
                self.marked.pop(r, None)
            else:
                self.marked[r] = label
        if label is not None:
            for address, size in self._writes:
                if not self._region(address):
                    self.regions.append((address, address + size, label))
            self._reg_log.append((label, self._where(pc), sorted(written),
                                  [f'{a:#x}' for a, _ in self._writes]))
        self._reads, self._writes = [], []

    def _on_code(self, uc, pc, size, _):
        self._finish()
        self._insn = (pc,) + self._decode(uc, pc, size)

    def _on_access(self, uc, access, address, size, value, kind):
        pc = uc.reg_read(UC_X86_REG_RIP)
        hit = self._region(address)
        if kind == 'w':
            self._writes.append((address, size))
        elif hit:
            self._reads.append(hit[1])
        if hit:
            start, label = hit
            self.log.append((label, kind, address - start, size, self._where(pc)))
            if kind == 'r':
                self._last_read = (pc, label)
        elif kind == 'w' and self.follow and self._last_read and self._last_read[0] == pc:
            label = self._last_read[1]
            self.regions.append((address, address + size, label))
            self.log.append((label, 'copy', 0, size, self._where(pc)))

    def reg_log(self):
        """[(label, module+offset, registers written, memory written)] for each instruction that passed a mark on."""
        self._finish()
        self._insn = None
        return self._reg_log

    def readers(self):
        out = {}
        for label, kind, _, _, where in self.log:
            if kind == 'r':
                out[where] = out.get(where, 0) + 1
        return out

    def write(self, path):
        with open(path, 'w') as f:
            for label, kind, off, size, where in self.log:
                f.write(f'{label}\t{kind}\t{off:#x}\t{size}\t{where}\n')
