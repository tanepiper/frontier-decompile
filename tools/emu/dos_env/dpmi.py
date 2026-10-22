"""The DPMI host (INT 31h) and the KERNEL and Borland RTM imports a DPMI16 program calls.
The meanings of RTM.5, RTM.6 and RTM.21 are inferred from how Borland Pascal 7's heap manager
calls them (Tyrian 2000, 1040:0433, 055d, 0381).
Part of DosEnv (see __init__.py); the methods use its state."""
import struct

from dos_emu import FarPtr

class DpmiServices:
    def int31(self, emu):
        ax, bx, cx, dx = emu.reg('ax'), emu.reg('bx'), emu.reg('cx'), emu.reg('dx')
        self.carry(False)
        if ax == 0x0000:                                  # allocate CX descriptors
            first = emu.new_selector()
            for _ in range(max(cx, 1) - 1):
                emu.new_selector()
            emu.set(ax=first)
        elif ax in (0x0001, 0x0101, 0x0502, 0x0600, 0x0601, 0x0009, 0x0008, 0x0603, 0x0604):
            pass                                          # free, unlock, limits, access rights
        elif ax == 0x0002:                                # real-mode segment -> selector
            emu.set(ax=emu.new_selector(base=bx << 4))
        elif ax == 0x0003:
            emu.set(ax=8)                                 # selector increment
        elif ax == 0x0006:
            base = emu.linear(bx, 0)
            emu.set(cx=base >> 16, dx=base & 0xFFFF)
        elif ax == 0x0007:
            emu.set_descriptor(bx, (cx << 16) | dx)
        elif ax == 0x000A:                                # alias: a data selector, same base
            emu.set(ax=emu.new_selector(base=emu.linear(bx, 0)))
        elif ax == 0x0100:                                # DOS memory: BX paragraphs
            seg = self.dos_mem
            self.dos_mem += bx
            emu.set(ax=seg, dx=emu.new_selector(base=seg << 4))
        elif ax == 0x0200:
            v = self.rm_vectors.get(bx & 0xFF, FarPtr(0, 0))
            emu.set(cx=v.seg, dx=v.off)
        elif ax == 0x0201:
            self.rm_vectors[bx & 0xFF] = FarPtr(cx, dx)
        elif ax == 0x0204:
            v = self.vector(bx & 0xFF)
            emu.set(cx=v.seg, dx=v.off)
        elif ax == 0x0205:
            self.pm_vectors[bx & 0xFF] = FarPtr(cx, dx)
        elif ax == 0x0202:
            v = self.exc_vectors.get(bx & 0xFF, FarPtr(0, 0))
            emu.set(cx=v.seg, dx=v.off)
        elif ax == 0x0203:
            self.exc_vectors[bx & 0xFF] = FarPtr(cx, dx)
        elif ax in (0x0300, 0x0301, 0x0302):              # call real mode: nothing to call
            self.log.append(f'dpmi {ax:04x}: real-mode call skipped')
        elif ax == 0x0303:                                # real-mode callback address
            emu.set(cx=0xF000, dx=0x0100 + 4 * len(self.callbacks))
            self.callbacks.append(FarPtr(emu.reg('ds'), emu.reg('si')))
        elif ax in (0x0304, 0x0E01, 0x0900, 0x0901):
            if ax in (0x0900, 0x0901):
                emu.set(ax=(ax & 0xFF00) | 1)
        elif ax == 0x0902:
            emu.set(ax=1)
        elif ax in (0x0305, 0x0306):
            emu.set(ax=0, bx=0xF000, cx=0x0200, si=self.stub_selector(), di=0x0200)
        elif ax == 0x0507:
            pass
        elif ax == 0x0800:                                # physical address: identity map
            emu.set(bx=bx, cx=cx)
        elif ax == 0x0E00:
            emu.set(ax=0x0045)                            # an FPU is present (387)
        elif ax == 0x0A00:
            return self.fail(0x8001)                      # no vendor extensions
        elif ax == 0x0400:
            emu.set(ax=0x005A, bx=0x0005, cx=0x0003, dx=0x0870)   # DPMI 0.90, 286 host, 386
        elif ax == 0x0500:
            info = struct.pack('<12I', 0x00F00000, 0xF00, 0xF00, 0xF00, 0xF00, 0xF00, 0xF00, 0xF00, 0, 0, 0, 0)
            emu.write((emu.reg('es'), emu.reg('di')), info[:48])
        else:
            emu.error = f'INT 31h (DPMI) AX={ax:04X}h is not provided by dos_env'
            emu.uc.emu_stop()
        self.log.append(f'dpmi {ax:04x}')

    def stub_selector(self):
        return self.emu.stub_seg

    def global_alloc(self, emu):
        """GlobalAlloc(flags: Word; size: Longint): THandle. The handle is the selector."""
        sp, ss = emu.reg('sp'), emu.reg('ss')
        size = emu.u32((ss, sp + 4))
        count = max(1, (size + 0xFFFF) // 0x10000)
        first = emu.new_segment()
        for _ in range(count - 1):
            emu.new_segment()
        self.log.append(f'GlobalAlloc {size} -> {first:04x}')
        emu.set(ax=first, dx=0)
        return 6

    def global_free(self, emu):
        emu.set(ax=0)
        return 2

    def global_lock(self, emu):
        sel = emu.u16((emu.reg('ss'), emu.reg('sp') + 4))
        emu.set(ax=0, dx=sel)
        return 2

    def get_free_space(self, emu):
        emu.set(ax=0, dx=0x00F0)                      # about 15 MB
        return 2

    def rtm_alloc(self, emu):
        """RTM.5(var Selector: Word; Owner: Longint; Flags: Word; Size: Longint): 0 = done."""
        sp, ss = emu.reg('sp'), emu.reg('ss')
        result = FarPtr(emu.u16((ss, sp + 6)), emu.u16((ss, sp + 4)))
        size = emu.u32((ss, sp + 14))
        sel = emu.new_segment()
        for _ in range(max(1, (size + 0xFFFF) // 0x10000) - 1):
            emu.new_segment()
        emu.write(result, struct.pack('<H', sel))
        self.log.append(f'RTM alloc {size} -> {sel:04x}')
        emu.set(ax=0)
        return 14

    def rtm_free(self, emu):
        """RTM.6(Selector: Word): free a block."""
        emu.set(ax=0)
        return 2

    def rtm_free_mem(self, emu):
        """RTM.21(Flags: Word; Owner: Longint?): free memory in bytes, DX:AX."""
        emu.set(ax=0, dx=0x00F0)
        return 6

    def init_task(self, emu):
        """InitTask: register-based. AX=1 (success), CX = stack size, DI = instance (DS)."""
        emu.set(ax=1, cx=0x2000, di=emu.reg('ds'), si=0, dx=1, bx=0x81, es=self.psp)
        return 0
