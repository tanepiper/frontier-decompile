"""The PC around the CPU: ports (VGA status, the DAC, the PIT, the PIC, sound cards), the
timer interrupt, the BIOS tick, and screenshots.
Part of DosEnv (see __init__.py); the methods use its state."""
import struct

from dos_emu import FarPtr

class PcHardware:
    def save_screen(self, path, seg=None):
        """Write the 320 x 200 mode 13h screen (VGA memory at 0xA0000, or a 64 KB segment the
        program draws into) as a PNG, coloured with the DAC palette the program set."""
        import zlib
        pix = bytes(self.emu.uc.mem_read(0xA0000, 64000)) if seg is None else self.emu.read((seg, 0), 64000)
        rgb = bytes(c << 2 | c >> 4 for c in self.dac)
        raw = b''.join(b'\0' + b''.join(rgb[p * 3:p * 3 + 3] for p in pix[y * 320:(y + 1) * 320]) for y in range(200))
        def chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
        png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 320, 200, 8, 2, 0, 0, 0))
        png += chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
        with open(path, 'wb') as f:
            f.write(png)

    def vector(self, n):
        """Where interrupt n goes: the program's protected-mode handler, else one set with
        INT 21h AH=25h, else the BIOS's IRET."""
        return self.pm_vectors.get(n) or self.vectors.get(n) or FarPtr(self.bios, 4 * n)

    def set_timer_slice(self):
        hz = 1_193_182 / self.divisor
        self.emu.slice = max(1000, int(self.ips / hz))

    def on_slice(self, emu):
        """Between slices: deliver IRQ 0 (the timer) through INT 8's handler."""
        v = self.vector(8)
        if emu.interrupt(v.seg, v.off):
            self.irq0_count += 1

    def bios_int8(self, uc, address, size, _):
        self.ticks = (self.ticks + 1) & 0xFFFFFFFF
        uc.mem_write(0x46C, struct.pack('<I', self.ticks))

    def bios_tick_read(self, uc, access, address, size, value, _):
        """The BIOS tick count (0040:006C) advances one tick every 128 reads of it, so loops
        that wait for the next tick (Crt's Delay calibration) end."""
        self.tick_reads += 1
        if self.tick_reads % 128 == 0:
            self.ticks = (self.ticks + 1) & 0xFFFFFFFF
            uc.mem_write(0x46C, struct.pack('<I', self.ticks))

    def port_in(self, uc, port, size, _):
        if port == 0x40:
            # PIT channel 0: counts down a little at every read, low byte then high byte, so
            # delay loops that watch it (OPL write delays) end.
            if not self.pit_high:
                self.pit = (self.pit - 7) & 0xFFFF
            v = (self.pit >> 8) if self.pit_high else (self.pit & 0xFF)
            self.pit_high = not self.pit_high
            return v
        if port == 0x3DA:
            self.retrace ^= 0x09
            return self.retrace
        if port == 0x3C9:
            v = self.dac[self.dac_read % 768]
            self.dac_read += 1
            return v
        if port == 0x201:
            return 0xFF
        if port in (0x21, 0xA1):
            return self.ports.get(('mask', port), 0)
        if port == 0x388:
            return 0x06
        return 0xFF if 0x220 <= port <= 0x22F else 0

    def port_out(self, uc, port, size, value, _):
        self.ports[port] = self.ports.get(port, 0) + 1
        if port == 0x3C8:
            self.dac_index = (value & 0xFF) * 3
        elif port == 0x3C7:
            self.dac_read = (value & 0xFF) * 3
        elif port == 0x3C9:
            self.dac[self.dac_index % 768] = value & 0x3F
            self.dac_index += 1
        elif port in (0x21, 0xA1):
            self.ports[('mask', port)] = value & 0xFF
        elif port == 0x43:
            self.pit_high = False                     # a latch or mode command restarts the read
            self.pit_write_high = False
        elif port == 0x40:
            if not self.pit_write_high:
                self.pit_low = value & 0xFF
            else:
                self.divisor = (self.pit_low | (value & 0xFF) << 8) or 0x10000
                self.set_timer_slice()
                self.log.append(f'timer {1_193_182 / self.divisor:.2f} Hz')
            self.pit_write_high = not self.pit_write_high
