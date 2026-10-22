"""DOS services (INT 21h): files from a folder, the overlay for writes, find first/next,
the DOS keyboard functions, vectors, date and time, exit.
Part of DosEnv (see __init__.py); the methods use its state."""
import os
import struct

from dos_emu import FarPtr

CF = 1

class DosServices:
    def carry(self, on):
        f = self.emu.reg('flags')
        self.emu.set(flags=(f | CF) if on else (f & ~CF))

    def fail(self, code):
        self.emu.set(ax=code)
        self.carry(True)

    def asciiz(self, seg, off):
        out = bytearray()
        while len(out) < 260:
            b = self.emu.read((seg, (off + len(out)) & 0xFFFF), 1)[0]
            if b == 0:
                break
            out.append(b)
        return out.decode('latin-1')

    def host_path(self, dos_name, for_write=False):
        """Find a DOS name under root (or overlay), ignoring case; None if missing."""
        name = dos_name.replace('\\', '/').split(':')[-1].lstrip('/')
        for base in ([self.overlay] if self.overlay else []) + [self.root]:
            if base is None:
                continue
            path = base
            for part in [p for p in name.split('/') if p and p != '.']:
                if not os.path.isdir(path):
                    path = None
                    break
                match = [e for e in os.listdir(path) if e.lower() == part.lower()]
                path = os.path.join(path, match[0]) if match else None
                if path is None:
                    break
            if path and os.path.exists(path):
                return path
        if for_write and self.overlay:
            os.makedirs(self.overlay, exist_ok=True)
            return os.path.join(self.overlay, os.path.basename(name).upper())
        return None

    def int21(self, emu):
        ah, al = emu.reg('ax') >> 8, emu.reg('ax') & 0xFF
        ds, dx, bx, cx = emu.reg('ds'), emu.reg('dx'), emu.reg('bx'), emu.reg('cx')
        self.carry(False)
        if ah in (0x02, 0x06):
            self.stdout.append(dx & 0xFF)
        elif ah == 0x09:
            s = bytearray()
            while len(s) < 4096:
                b = emu.read((ds, (dx + len(s)) & 0xFFFF), 1)[0]
                if b == ord('$'):
                    break
                s.append(b)
            self.stdout += s
        elif ah == 0x0B:
            emu.set(ax=(emu.reg('ax') & 0xFF00) | (0xFF if self.keys else 0))
        elif ah in (0x01, 0x07, 0x08, 0x0C):
            if ah == 0x0C:                                  # flush, then the function in AL
                if al not in (0x01, 0x07, 0x08):
                    return
            if not self.keys:
                emu.error = 'the program waits for a key (INT 21h): put keys in env.keys'
                emu.uc.emu_stop()
                return
            emu.set(ax=(emu.reg('ax') & 0xFF00) | (self.keys.pop(0) & 0xFF))
        elif ah == 0x0E:
            emu.set(ax=(emu.reg('ax') & 0xFF00) | 26)
        elif ah == 0x19:
            emu.set(ax=(emu.reg('ax') & 0xFF00) | 2)             # drive C:
        elif ah == 0x1A:
            self.dta = FarPtr(ds, dx)
        elif ah == 0x25:
            self.vectors[al] = FarPtr(ds, dx)
        elif ah == 0x35:
            v = self.vector(al)
            emu.set(es=v.seg, bx=v.off)
        elif ah == 0x2A:
            y, m, d = self.clock[:3]
            emu.set(cx=y, dx=(m << 8) | d, ax=(emu.reg('ax') & 0xFF00) | 2)
        elif ah == 0x2C:
            h, mi, sec, hs = self.clock[3:]
            emu.set(cx=(h << 8) | mi, dx=(sec << 8) | hs)
        elif ah == 0x30:
            emu.set(ax=0x0005, bx=0, cx=0)
        elif ah == 0x3B:
            pass                                                # chdir: one folder only
        elif ah in (0x3C, 0x3D, 0x5B):
            name = self.asciiz(ds, dx)
            if ah == 0x3D:
                path = self.host_path(name)
                if path is None or not os.path.isfile(path):
                    self.log.append(f'open {name}: not found')
                    return self.fail(2)
                data = bytearray(open(path, 'rb').read())
            else:
                data = bytearray()                      # create: an empty file
            # Writes never reach root: a file is saved (to overlay, or nowhere) only when the
            # program writes to it. Pascal's Reset opens files read/write (FileMode 2), so the
            # open mode alone says nothing.
            h = self.next_handle
            self.next_handle += 1
            self.files[h] = [name, data, 0, ah != 0x3D]   # name, bytes, position, dirty
            self.log.append(f'{"create" if ah != 0x3D else "open"} {name} -> {h}')
            emu.set(ax=h)
        elif ah == 0x3E:
            f = self.files.pop(bx, None)
            if f and f[3]:
                self.save(f[0], f[1])
        elif ah == 0x3F:
            if bx == 0:
                return emu.set(ax=0)
            f = self.files.get(bx)
            if f is None:
                return self.fail(6)
            chunk = bytes(f[1][f[2]:f[2] + cx])
            emu.write((ds, dx), chunk)
            f[2] += len(chunk)
            emu.set(ax=len(chunk))
        elif ah == 0x40:
            data = emu.read((ds, dx), cx)
            if bx in (1, 2):
                self.stdout += data
            else:
                f = self.files.get(bx)
                if f is None:
                    return self.fail(6)
                f[1][f[2]:f[2] + cx] = data
                f[2] += cx
                f[3] = True                                 # dirty: saved on close
            emu.set(ax=cx)
        elif ah == 0x41:
            pass
        elif ah == 0x42:
            f = self.files.get(bx)
            if f is None:
                return self.fail(6)
            off = struct.unpack('<i', struct.pack('<HH', dx, cx))[0]
            base = (0, f[2], len(f[1]))[al] if al < 3 else 0
            f[2] = max(0, base + off)
            emu.set(ax=f[2] & 0xFFFF, dx=f[2] >> 16)
        elif ah == 0x43:
            emu.set(cx=0x20)
        elif ah == 0x44:
            if al == 0:
                emu.set(dx=0x80 | 0x03 if bx < 5 else 0x02)
            else:
                emu.set(ax=0)
        elif ah == 0x47:
            emu.write((ds, emu.reg('si')), b'\0')
        elif ah == 0x4C:
            self.log.append(f'exit {al}')
            emu.stop(al)
        elif ah == 0x4E:
            self.find_first(self.asciiz(ds, dx), cx)
        elif ah == 0x4F:
            self.find_next()
        elif ah in (0x51, 0x62):
            emu.set(bx=self.psp)
        else:
            emu.error = f'INT 21h AH={ah:02X}h (AL={al:02X}h) is not provided by dos_env'
            emu.uc.emu_stop()

    def save(self, name, data):
        """A written file goes to overlay (never to root); without an overlay it is kept only
        in memory (env.written)."""
        self.written[os.path.basename(name.replace('\\', '/')).upper()] = bytes(data)
        if self.overlay:
            os.makedirs(self.overlay, exist_ok=True)
            with open(os.path.join(self.overlay, os.path.basename(name.replace('\\', '/')).upper()), 'wb') as out:
                out.write(data)
        self.log.append(f'save {name} ({len(data)} bytes)')

    def find_first(self, pattern, attrs):
        import fnmatch
        pat = pattern.replace('\\', '/').split('/')[-1].lower().replace('*.*', '*')
        names = sorted(set(os.listdir(self.root)) | (set(os.listdir(self.overlay)) if self.overlay and os.path.isdir(self.overlay) else set()))
        self.find = [n for n in names if fnmatch.fnmatch(n.lower(), pat) and os.path.isfile(self.host_path(n) or '')]
        self.log.append(f'find {pattern}: {len(self.find)}')
        self.find_next()

    def find_next(self):
        if not self.find:
            return self.fail(18)
        name = self.find.pop(0)
        size = os.path.getsize(self.host_path(name))
        dta = self.dta or FarPtr(self.emu.regs['ds'], 0x80)
        rec = bytearray(43)
        rec[21] = 0x20
        struct.pack_into('<I', rec, 26, size)
        rec[30:30 + 13] = name.upper().encode('latin-1')[:12].ljust(13, b'\0')
        self.emu.write(dta, rec)
        self.emu.set(ax=0)
