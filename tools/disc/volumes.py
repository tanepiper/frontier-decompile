#!/usr/bin/env python3
"""Find and read the file systems in a CD data track: ISO 9660 with its Joliet
names, the older High Sierra format, an El Torito boot record, and an Apple HFS or HFS+
volume (hybrid Mac and PC discs).

    from image import open_image
    from volumes import scan
    for vol in scan(open_image('GAME.CUE').tracks[0]): print(vol.kind, vol.info)

ISO 9660 (ECMA-119) keeps its volume descriptors from sector 16 of the session: a primary
descriptor, supplementary ones (Joliet is a supplementary descriptor whose names are UCS-2),
an optional boot record, and a terminator. Numbers are stored twice, little-endian then
big-endian. The directory tree starts at the root record inside each descriptor. HFS keeps
its master directory block 1024 bytes into the volume, so a hybrid disc can carry both. Only
the HFS header is read here: listing an HFS tree needs its B-tree (hfsutils or HFSExplorer).
Standard library only.
"""
import datetime
import struct


def text(b):
    return b.decode('latin-1').rstrip(' \x00')


def dec_date17(b):
    """The 17-byte descriptor date ('YYYYMMDDhhmmsscc' and a zone in 15-minute steps)."""
    s = b[:16].decode('latin-1')
    if not s.strip('0 \x00'):
        return None
    zone = struct.unpack('b', b[16:17])[0] * 15
    return f'{s[0:4]}-{s[4:6]}-{s[6:8]} {s[8:10]}:{s[10:12]}:{s[12:14]} UTC{zone // 60:+03d}:{abs(zone) % 60:02d}'


def dec_date7(b):
    """The 7-byte directory record date (years since 1900, month, day, h, m, s, zone)."""
    if not any(b[:6]):
        return None
    zone = struct.unpack('b', b[6:7])[0] * 15
    return f'{1900 + b[0]:04d}-{b[1]:02d}-{b[2]:02d} {b[3]:02d}:{b[4]:02d}:{b[5]:02d} UTC{zone // 60:+03d}:{abs(zone) % 60:02d}'


class Entry:
    def __init__(self, path, lba, size, date, directory, hidden, xa=0):
        self.path, self.lba, self.size, self.date = path, lba, size, date
        self.directory, self.hidden = directory, hidden
        self.xa = xa                    # CD-ROM XA attributes from the system use area, or 0

    @property
    def form2(self):
        """Mode 2 Form 2 or interleaved sectors (streamed video and sound): a copy of 2048 bytes
        per sector loses part of each sector."""
        return bool(self.xa & 0x3000)

    @property
    def sectors(self):
        return (self.size + 2047) // 2048


class Volume:
    def __init__(self, kind, info, track=None, root=None, joliet=False):
        self.kind, self.info = kind, info
        self.track, self.root, self.joliet = track, root, joliet
        self.base = 0                   # disc sector number of the track's sector 0

    def entries(self):
        """Every file and directory, depth first, as Entry objects."""
        if self.root is None:
            return []
        out, seen = [], set()
        self._walk(self.root, '', out, seen)
        return out

    def _walk(self, record, prefix, out, seen):
        lba, size = record
        if lba in seen:                 # a loop in a damaged or protected tree
            return
        seen.add(lba)
        data = self.read(lba, (size + 2047) // 2048)
        i = 0
        while i < len(data):
            n = data[i]
            if n == 0:                  # records never cross a sector: skip to the next one
                i = (i // 2048 + 1) * 2048
                continue
            r = data[i:i + n]
            i += n
            name_len = r[32]
            raw = r[33:33 + name_len]
            if raw in (b'\x00', b'\x01'):
                continue                # '.' and '..'
            name = raw.decode('utf-16-be', 'replace') if self.joliet else raw.decode('latin-1')
            name = name.split(';')[0]
            if name.endswith('.') and not self.joliet:
                name = name[:-1]
            flags = r[25]
            use = r[33 + name_len + (1 - name_len % 2):]   # the system use area, after padding
            xa = struct.unpack_from('>H', use, 4)[0] if len(use) >= 8 and use[6:8] == b'XA' else 0
            e = Entry(prefix + name, struct.unpack_from('<I', r, 2)[0],
                      struct.unpack_from('<I', r, 10)[0], dec_date7(r[18:25]),
                      bool(flags & 2), bool(flags & 1), xa)
            out.append(e)
            if e.directory:
                self._walk((e.lba, e.size), e.path + '/', out, seen)

    def used(self):
        """The sectors this volume's own structures and files occupy: the system area, the
        descriptors, the path tables, the directories and the files."""
        used = set(range(0, 16))
        if self.root is None:
            return used
        i = 16
        while True:
            used.add(i)
            try:
                d = self.read(i)
            except EOFError:
                break
            if d[1:6] != b'CD001' or d[0] == 255:
                break
            if d[0] in (1, 2):
                size = struct.unpack_from('<I', d, 132)[0]
                for at in (140, 144):
                    p = struct.unpack_from('<I', d, at)[0]
                    if p:
                        used.update(range(p, p + (size + 2047) // 2048))
                for at in (148, 152):
                    p = struct.unpack_from('>I', d, at)[0]
                    if p:
                        used.update(range(p, p + (size + 2047) // 2048))
            i += 1
        used.update(range(self.root[0], self.root[0] + (self.root[1] + 2047) // 2048))
        for e in self.entries():
            used.update(range(e.lba, e.lba + max(1, e.sectors)))
        return used

    def read(self, lba, count=1):
        return self.track.read(lba - self.base, count)

    def file(self, entry):
        """The bytes of one file."""
        return self.read(entry.lba, (entry.size + 2047) // 2048)[:entry.size]


def _iso_info(d, wide=False):
    # Joliet text is UCS-2. Some mastering tools pad it with single-byte or little-endian
    # spaces, which decode as other blank characters: strip any of them.
    text = (lambda b: b[:len(b) // 2 * 2].decode('utf-16-be', 'replace').strip('\x00\u2020\ufffd \u2000').strip()) \
        if wide else globals()['text']
    return {
        'system': text(d[8:40]), 'volume': text(d[40:72]),
        'sectors': struct.unpack_from('<I', d, 80)[0],
        'block_size': struct.unpack_from('<H', d, 128)[0],
        'volume_set': text(d[190:318]), 'publisher': text(d[318:446]),
        'preparer': text(d[446:574]), 'application': text(d[574:702]),
        'copyright_file': text(d[702:739]), 'abstract_file': text(d[739:776]),
        'created': dec_date17(d[813:830]), 'modified': dec_date17(d[830:847]),
        'expires': dec_date17(d[847:864]), 'effective': dec_date17(d[864:881]),
    }


def _root(d):
    return struct.unpack_from('<I', d, 156 + 2)[0], struct.unpack_from('<I', d, 156 + 10)[0]


def _find_base(track, root_lba):
    """Which disc sector the track's sector 0 is. The root's '.' record names its own sector,
    so try the likely bases, then search the start of the track."""
    def is_root(index):
        try:
            s = track.read(index)
        except EOFError:
            return False
        return s[0] >= 34 and s[32] == 1 and s[33] == 0 and struct.unpack_from('<I', s, 2)[0] == root_lba
    for base in (0, track.lba):
        if 0 <= root_lba - base < track.frames and is_root(root_lba - base):
            return base
    for index in range(min(track.frames, 6000)):
        if is_root(index):
            return root_lba - index
    return 0


def _el_torito(track, d, base):
    catalog = struct.unpack_from('<I', d, 71)[0]
    info = {'catalog_sector': catalog}
    try:
        c = track.read(catalog - base)
    except EOFError:
        return info
    platforms = {0: 'x86 PC', 1: 'PowerPC', 2: 'Macintosh', 0xEF: 'UEFI'}
    media = {0: 'no emulation', 1: '1.2 MB floppy', 2: '1.44 MB floppy', 3: '2.88 MB floppy', 4: 'hard disk'}
    info['platform'] = platforms.get(c[1], hex(c[1]))
    info['bootable'] = c[32] == 0x88
    info['emulation'] = media.get(c[33] & 0x0F, hex(c[33]))
    info['load_segment'] = hex(struct.unpack_from('<H', c, 34)[0] or 0x7C0)
    info['sectors_loaded'] = struct.unpack_from('<H', c, 38)[0]
    info['image_sector'] = struct.unpack_from('<I', c, 40)[0]
    return info


def _hfs(track):
    try:
        blocks = track.read(0, 2)       # 4096 bytes: 512-byte blocks 0 to 7
    except EOFError:
        return []
    out = []
    starts = [0]
    if blocks[512:514] == b'PM':        # Apple partition map: find the HFS partition
        count = struct.unpack_from('>I', blocks, 512 + 4)[0]
        for i in range(1, min(count, 7) + 1):
            p = blocks[512 * i:512 * (i + 1)]
            if p[:2] == b'PM' and b'Apple_HFS' in p[48:80]:
                starts.append(struct.unpack_from('>I', p, 8)[0] * 512)
    for start in starts:
        try:
            mdb = track.read(start // 2048, 2)[start % 2048 + 1024:]
        except EOFError:
            continue
        sig = mdb[:2]
        epoch = datetime.datetime(1904, 1, 1)
        if sig == b'BD':
            created = struct.unpack_from('>I', mdb, 2)[0]
            name = mdb[37:37 + mdb[36]].decode('mac_roman', 'replace')
            out.append(Volume('HFS', {'volume': name, 'byte_offset': start,
                                      'created': str(epoch + datetime.timedelta(seconds=created)),
                                      'files': struct.unpack_from('>I', mdb, 84)[0],
                                      'directories': struct.unpack_from('>I', mdb, 88)[0],
                                      'embedded_hfs_plus': mdb[124:126] == b'H+'}))
        elif sig in (b'H+', b'HX'):
            created = struct.unpack_from('>I', mdb, 16)[0]
            out.append(Volume('HFS+', {'byte_offset': start,
                                       'created': str(epoch + datetime.timedelta(seconds=created)),
                                       'files': struct.unpack_from('>I', mdb, 32)[0],
                                       'directories': struct.unpack_from('>I', mdb, 36)[0]}))
    return out


def scan(track):
    """Every file system found at the start of a data track."""
    if track.audio:
        return []
    out = _hfs(track)
    primary_root, base = None, 0
    for i in range(16, 16 + 64):
        try:
            d = track.read(i)
        except EOFError:
            break
        if d[9:14] == b'CDROM':         # High Sierra: the identifier sits 8 bytes later
            out.append(Volume('High Sierra', {'volume': text(d[48:80])}))
            break
        if d[1:6] != b'CD001':
            break
        kind = d[0]
        if kind == 255:
            break
        if kind == 1:
            primary_root = _root(d)
            base = _find_base(track, primary_root[0])
            v = Volume('ISO 9660', _iso_info(d), track, primary_root)
            v.base = base
            out.append(v)
        elif kind == 2:
            joliet = d[88:91] in (b'%/@', b'%/C', b'%/E')
            v = Volume('Joliet' if joliet else 'ISO 9660 supplementary', _iso_info(d, joliet), track, _root(d), joliet)
            v.base = base
            out.append(v)
        elif kind == 0 and d[7:30] == b'EL TORITO SPECIFICATION':
            out.append(Volume('El Torito boot record', _el_torito(track, d, base)))
    # Rock Ridge (long Unix names) lives in each record's system use area: say when it is there.
    for v in out:
        if v.kind == 'ISO 9660' and v.root:
            try:
                s = v.read(v.root[0])
            except EOFError:
                continue
            use = s[33 + s[32]:s[0]]      # the '.' record's system use area
            if b'SP' in use or b'RR' in use or b'PX' in use:
                v.info['rock_ridge'] = True
    return out
