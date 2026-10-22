"""The BIOS: video (INT 10h), keyboard (INT 16h), mouse (INT 33h), multiplex (INT 2Fh),
time of day (INT 1Ah), the BIOS data area and the PSP.
Part of DosEnv (see __init__.py); the methods use its state."""
import struct

class BiosServices:
    def int10(self, emu):
        """Video BIOS: modes, the text cursor and text output on the 0xB8000 screen."""
        ax, bx, cx, dx = emu.reg('ax'), emu.reg('bx'), emu.reg('cx'), emu.reg('dx')
        ah, al = ax >> 8, ax & 0xFF
        uc = emu.uc
        if ah == 0x00:
            self.video_mode = al & 0x7F
            uc.mem_write(0x449, bytes([self.video_mode]))
            if self.video_mode == 3 and not al & 0x80:
                uc.mem_write(0xB8000, b'\x20\x07' * 2000)
            self.cursor = (0, 0)
        elif ah == 0x01:
            self.cursor_shape = cx
        elif ah == 0x02:
            self.cursor = (dx >> 8, dx & 0xFF)
        elif ah == 0x03:
            row, col = getattr(self, 'cursor', (0, 0))
            emu.set(dx=(row << 8) | col, cx=getattr(self, 'cursor_shape', 0x0607))
        elif ah in (0x09, 0x0A):
            row, col = getattr(self, 'cursor', (0, 0))
            at = 0xB8000 + (row * 80 + col) * 2
            for i in range(cx):
                uc.mem_write(at + 2 * i, bytes([al]) if ah == 0x0A else bytes([al, bx & 0xFF]))
        elif ah == 0x0E:
            row, col = getattr(self, 'cursor', (0, 0))
            if al == 13:
                col = 0
            elif al == 10:
                row += 1
            elif al == 8:
                col = max(0, col - 1)
            else:
                uc.mem_write(0xB8000 + (row * 80 + col) * 2, bytes([al]))
                col += 1
            if col >= 80:
                row, col = row + 1, 0
            if row >= 25:
                screen = bytes(uc.mem_read(0xB8000 + 160, 3840))
                uc.mem_write(0xB8000, screen + b'\x20\x07' * 80)
                row = 24
            self.cursor = (row, col)
        elif ah == 0x08:
            row, col = getattr(self, 'cursor', (0, 0))
            emu.set(ax=int.from_bytes(uc.mem_read(0xB8000 + (row * 80 + col) * 2, 2), 'little'))
        elif ah == 0x0F:
            emu.set(ax=(80 << 8) | (self.video_mode if self.video_mode is not None else 3), bx=bx & 0xFF)
        elif ah == 0x1A:
            emu.set(ax=0x001A, bx=0x0008)                 # VGA with colour display
        elif ah == 0x12 and (bx & 0xFF) == 0x10:
            emu.set(bx=0x0003, cx=0x0009)                 # EGA/VGA, 256 KB, colour
        elif ah == 0x11 and al == 0x30:
            emu.set(cx=16, dx=24)                         # character height, rows - 1
        self.log.append(f'int 10h ah={ah:02x}h al={al:02x} dx={dx:04x} cx={cx:04x} bx={bx:04x}')

    def screen_text(self):
        """The 80 x 25 text screen as lines (what a text-mode message says)."""
        raw = bytes(self.emu.uc.mem_read(0xB8000, 4000))[::2].decode('cp437', 'replace')
        return [raw[i:i + 80].rstrip() for i in range(0, 2000, 80)]

    def int16(self, emu):
        """Keyboard BIOS with a queue: env.keys holds (scan << 8 | ascii) words. Reading a key
        when the queue is empty stops the run (a real BIOS would wait), so a script decides what
        is typed; AX = 0 would be Ctrl-Break."""
        ah = emu.reg('ax') >> 8
        flags = emu.reg('flags')
        if ah in (0x01, 0x11):
            if self.keys:
                emu.set(ax=self.keys[0], flags=flags & ~0x40)
            else:
                emu.set(flags=flags | 0x40)            # ZF: no key waiting
        elif ah in (0x00, 0x10):
            if not self.keys:
                emu.error = 'the program waits for a key (INT 16h): put keys in env.keys'
                emu.uc.emu_stop()
                return
            emu.set(ax=self.keys.pop(0))
        elif ah in (0x02, 0x12):
            emu.set(ax=0)                              # shift states: none

    def int2f(self, emu):
        """Multiplex: nothing installed (registers left as they are, which callers read as
        "not present": no Windows, no RTM extensions, no XMS driver)."""
        self.log.append(f'int 2fh ax={emu.reg("ax"):04x}h: not present')

    def int33(self, emu):
        emu.set(ax=0)                                 # no mouse driver

    def int1a(self, emu):
        if emu.reg('ax') >> 8 == 0:
            emu.set(cx=self.ticks >> 16, dx=self.ticks & 0xFFFF, ax=0)

    def make_psp(self, args, program, environment):
        """A program segment prefix: INT 20h at 0, the environment's selector at 0x2C, the
        command line at 0x80. The environment block ends with the program's path (ParamStr(0))."""
        emu = self.emu
        env_block = b''.join(e.encode('latin-1') + b'\0' for e in environment) + b'\0'
        env_block += b'\x01\x00' + program.encode('latin-1') + b'\0'
        env_sel = emu.new_segment(env_block)
        tail = args.encode('latin-1')[:126]
        psp = bytearray(256)
        psp[0:2] = b'\xCD\x20'
        struct.pack_into('<H', psp, 2, 0xA000)            # top of memory, as a paragraph
        struct.pack_into('<H', psp, 0x2C, env_sel)
        psp[0x80] = len(tail)
        psp[0x81:0x81 + len(tail) + 1] = tail + b'\r'
        return emu.new_segment(bytes(psp))

    def init_bios_data(self):
        """The BIOS data area (0040:0000) as a PC with colour VGA in text mode 3 has it."""
        uc = self.emu.uc
        uc.mem_write(0x410, struct.pack('<H', 0x0027))   # equipment: colour 80x25, FPU, floppy
        uc.mem_write(0x413, struct.pack('<H', 640))      # KB of conventional memory
        uc.mem_write(0x449, bytes([3]))                  # video mode
        uc.mem_write(0x44A, struct.pack('<H', 80))       # columns
        uc.mem_write(0x44C, struct.pack('<H', 4096))     # bytes per text page
        uc.mem_write(0x463, struct.pack('<H', 0x3D4))    # CRT controller port (status at +6)
        uc.mem_write(0x484, bytes([24, 16, 0]))          # rows - 1, character height
        uc.mem_write(0x487, bytes([0x60, 0x09]))         # EGA/VGA flags
