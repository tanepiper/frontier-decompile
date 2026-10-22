#!/usr/bin/env python3
"""What each file of a game is, from its bytes: the format its first bytes name, its size and
its entropy. It finds the code, the containers, the pictures and sounds, and the files that are
compressed or encrypted, before anything is decoded.

Usage: filetypes.py PATH [PATH ...] [--summary] [--blocks N] [--json]

  PATH         files or folders (folders are walked)
  --summary    one line per kind: how many files and how many bytes
  --blocks N   for each file, also the entropy of every N-byte block (find the compressed
               parts inside a container, or where an executable's packed data starts)
  --json       print the result as JSON

Entropy is in bits per byte, from 0 (one byte value repeated) to 8 (every value equally
likely). Text and code sit between about 4 and 6.5. Above about 7.5, the bytes are compressed
or encrypted: a decoder needs the game's decompressor or key (re-format, xorscan.py). A known
compressed format (ZIP, PNG, Ogg) is expected to be high. An unknown file that is high is the
one to look at. The kind is a guess from signatures. It is not proof: confirm it in the code
that reads the file (re-format). Standard library only.
"""
import json
import math
import os
import sys

# (offset, signature, kind). The first match wins, so longer and more specific ones go first.
SIGNATURES = [
    (0, b'MZ', 'executable (MZ)'), (0, b'\x7fELF', 'executable (ELF)'),
    (0, b'\xcf\xfa\xed\xfe', 'executable (Mach-O)'), (0, b'\xca\xfe\xba\xbe', 'Mach-O universal or Java class'),
    (0, b'PK\x03\x04', 'ZIP archive'), (0, b'Rar!', 'RAR archive'), (0, b'7z\xbc\xaf', '7-Zip archive'),
    (0, b'MSCF', 'Microsoft cabinet'), (0, b'ISc(', 'InstallShield cabinet'), (0, b'\x1f\x8b', 'gzip'),
    (0, b'BZh', 'bzip2'), (0, b'\xfd7zXZ', 'xz'), (0, b'\x60\xea', 'ARJ archive'),
    (2, b'-lh', 'LHA archive'), (2, b'-lz', 'LHA archive'),
    (0, b'IWAD', 'Doom WAD (IWAD)'), (0, b'PWAD', 'Doom WAD (PWAD)'), (0, b'WAD2', 'Quake WAD2'),
    (0, b'WAD3', 'Half-Life WAD3'), (0, b'PACK', 'Quake PAK'), (0, b'KenSilverman', 'Build engine GRP'),
    (0, b'MPQ\x1a', 'Blizzard MPQ'), (0, b'BIFFV1', 'BioWare BIF'), (0, b'KEY V1', 'BioWare KEY'),
    (0, b'\xc1\x83\x2a\x9e', 'Unreal package'), (0, b'UnityFS', 'Unity asset bundle'),
    (0, b'RIFF', 'RIFF container'), (0, b'FORM', 'IFF container'), (0, b'LIST', 'IFF list'),
    (0, b'Creative Voice File', 'Creative VOC sound'), (0, b'CTMF', 'Creative Music File (CMF)'),
    (0, b'MThd', 'Standard MIDI'), (0, b'HMIMIDIP', 'HMI MIDI (HMP)'), (0, b'HMI-MIDISONG', 'HMI MIDI (HMI)'),
    (0, b'OggS', 'Ogg'), (0, b'fLaC', 'FLAC'), (0, b'ID3', 'MP3 (ID3 tag)'), (0, b'\xff\xfb', 'MP3'),
    (0, b'Extended Module:', 'FastTracker XM'), (0, b'IMPM', 'Impulse Tracker'), (44, b'SCRM', 'Scream Tracker S3M'),
    (1080, b'M.K.', 'ProTracker MOD'), (1080, b'4CHN', 'MOD'), (1080, b'8CHN', 'MOD'),
    (0, b'\x89PNG', 'PNG'), (0, b'GIF8', 'GIF'), (0, b'\xff\xd8\xff', 'JPEG'), (0, b'BM', 'BMP'),
    (0, b'DDS ', 'DirectDraw surface'), (0, b'#?RADIANCE', 'Radiance HDR'),
    (4, b'\x11\xaf', 'Autodesk FLI animation'), (4, b'\x12\xaf', 'Autodesk FLC animation'),
    (0, b'SMK2', 'Smacker video'), (0, b'SMK4', 'Smacker video'), (0, b'BIK', 'Bink video'),
    (0, b'KB2', 'Bink 2 video'), (0, b'\x1aE\xdf\xa3', 'Matroska or WebM'),
    (0, b'Microsoft C/C++ MSF 7.00', 'Microsoft debug symbols (PDB)'), (0, b'!<arch>\n', 'library (ar or COFF .lib)'),
    (0, b'FWS', 'Flash movie (SWF, uncompressed)'), (0, b'CWS', 'Flash movie (SWF, zlib)'),
    (0, b'ZWS', 'Flash movie (SWF, LZMA)'), (0, b'RIFX', 'Shockwave Director (big-endian)'),
    (0, b'XFIR', 'Shockwave Director (little-endian)'),
    (0, b'SQLite format 3', 'SQLite database'), (0, b'<?xml', 'XML'),
    (0, b'\xef\xbb\xbf', 'text (UTF-8 with BOM)'), (0, b'\xff\xfe', 'text (UTF-16LE with BOM)'),
    (0x8001, b'CD001', 'ISO 9660 image'), (0, b'\x00\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\x00', 'raw CD image (BIN)'),
]
RIFF_KINDS = {b'WAVE': 'WAVE sound', b'AVI ': 'AVI video', b'RMID': 'RIFF MIDI', b'ACON': 'animated cursor'}
IFF_KINDS = {b'ILBM': 'IFF picture (ILBM)', b'PBM ': 'IFF picture (PBM)', b'8SVX': 'IFF sound (8SVX)',
             b'XDIR': 'XMIDI music', b'XMID': 'XMIDI music', b'AIFF': 'AIFF sound', b'AIFC': 'AIFF-C sound'}
HEAD = 0x8010


def entropy(data):
    if not data:
        return 0.0
    counts = [0] * 256
    for b in data:
        counts[b] += 1
    n = len(data)
    return max(0.0, -sum(c / n * math.log2(c / n) for c in counts if c))


def sample(path, size, limit=4 << 20):
    """All of a small file. For a big one, 64 pieces spread over it, 4 MB in all."""
    with open(path, 'rb') as f:
        if size <= limit:
            return f.read()
        step, piece, out = size // 64, limit // 64, bytearray()
        for i in range(64):
            f.seek(i * step)
            out += f.read(piece)
        return bytes(out)


def kind_of(head, size):
    for off, sig, kind in SIGNATURES:
        if head[off:off + len(sig)] == sig:
            if kind == 'RIFF container':
                return RIFF_KINDS.get(head[8:12], f'RIFF ({head[8:12].decode("latin-1")})')
            if kind == 'IFF container':
                return IFF_KINDS.get(head[8:12], f'IFF ({head[8:12].decode("latin-1")})')
            if kind == 'executable (MZ)':
                return mz_kind(head)
            if kind == 'BMP' and not (size > 26 and int.from_bytes(head[2:6], 'little') in (0, size)):
                continue
            return kind
    if head[:1] == b'\x0a' and head[1:2] in b'\x00\x02\x03\x04\x05' and head[2:3] == b'\x01':
        return 'PCX picture'
    text = head[:4096]
    if text and sum(32 <= b < 127 or b in (9, 10, 13) for b in text) / len(text) > 0.97:
        return 'text'
    return 'unknown'


def mz_kind(head):
    at = int.from_bytes(head[0x3c:0x40], 'little')
    sig = head[at:at + 2] if at + 2 <= len(head) else b''
    kind = {b'PE': 'executable (PE, Windows)', b'NE': 'executable (NE, Windows 3.x or OS/2)',
            b'LE': 'executable (LE, DOS extender or VxD)', b'LX': 'executable (LX, OS/2 or DOS extender)'}.get(
        sig, 'executable (MZ, DOS)')
    for marker, packer in ((b'LZ09', 'LZEXE'), (b'LZ91', 'LZEXE'), (b'PKLITE', 'PKLITE'), (b'UPX!', 'UPX'),
                           (b'diet', 'DIET'), (b'RJSX', 'ARJ self-extractor'), (b'PK\x03\x04', 'ZIP self-extractor')):
        if marker in head[:0x400]:
            kind += f', packed? ({packer})'
            break
    return kind


def verdict(kind, h):
    if h < 1.0:
        return 'mostly one value (padding, empty or sparse)'
    packed = any(k in kind for k in ('ZIP', 'RAR', '7-Zip', 'cabinet', 'gzip', 'bzip2', 'xz', 'PNG', 'JPEG',
                                     'Ogg', 'MP3', 'FLAC', 'video', 'GIF', 'LHA', 'ARJ', 'MPQ', 'zlib', 'LZMA'))
    if packed and h > 6.5:
        return 'compressed (expected)'
    if h > 7.5:
        return 'compressed or encrypted'
    if h > 6.8:
        return 'dense: partly compressed, or packed numbers'
    return ''


def walk(paths):
    for p in paths:
        if os.path.isdir(p):
            for base, dirs, files in os.walk(p):
                dirs.sort()
                for f in sorted(files):
                    yield os.path.join(base, f), p
        else:
            yield p, os.path.dirname(p) or '.'


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    blocks = int(argv[argv.index('--blocks') + 1]) if '--blocks' in argv else 0
    paths = [a for i, a in enumerate(argv) if not a.startswith('--') and (i == 0 or argv[i - 1] != '--blocks')]
    rows = []
    for path, root in walk(paths):
        size = os.path.getsize(path)
        with open(path, 'rb') as f:
            head = f.read(HEAD)
        data = sample(path, size)
        h = entropy(data)
        k = kind_of(head, size)
        row = {'path': os.path.relpath(path, root), 'size': size, 'kind': k,
               'entropy': round(h, 2), 'note': verdict(k, h)}
        if blocks:
            with open(path, 'rb') as f:
                row['blocks'] = [round(entropy(f.read(blocks)), 2) for _ in range((size + blocks - 1) // blocks)]
        rows.append(row)
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    if '--summary' in argv:
        by = {}
        for r in rows:
            n, b = by.get(r['kind'], (0, 0))
            by[r['kind']] = (n + 1, b + r['size'])
        for k, (n, b) in sorted(by.items(), key=lambda x: -x[1][1]):
            print(f'{n:6d} files {b:15,d} bytes  {k}')
        return 0
    for r in rows:
        print(f'{r["size"]:13,d}  {r["entropy"]:4.2f}  {r["kind"]:34s} {r["path"]}'
              + (f'  [{r["note"]}]' if r['note'] else ''))
        if blocks:
            print('               ' + ' '.join(f'{x:.1f}' for x in r['blocks']))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
