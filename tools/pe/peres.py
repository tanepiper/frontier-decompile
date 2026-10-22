#!/usr/bin/env python3
"""List and extract the resources inside a Windows executable or DLL: version information,
string tables, dialogs, menus, icons, bitmaps, sounds and the game's own data.

Usage: peres.py BINARY [--version] [--strings] [--extract DIR] [--json]

  (no option)     every resource: type, name, language, size, and the version information
  --version       only the version information (company, product, version, original name)
  --strings       every string in the string tables, with its id
  --extract DIR   write each resource to DIR/TYPE/NAME_LANG.EXT; bitmaps get a BMP file header
                  so they open as pictures

Windows games keep more in their resources than icons. The version information names the
product, the company and the build; the original file name and the version number date and
identify a release. String tables hold interface text and error messages, often with ids the
code passes to LoadString (search the export for the id). Dialogs and menus are the game's
standard windows: the launcher, the options. RCDATA and custom types hold whatever the
developer packed in: levels, palettes, scripts, sounds. A 16-bit NE program has resources too:
use tools/dos/dosinfo.py for those.

Extracted resources are game material: write them into derived/. Needs pefile (.venv).
"""
import json
import os
import struct
import sys

import pefile

TYPES = {1: 'CURSOR', 2: 'BITMAP', 3: 'ICON', 4: 'MENU', 5: 'DIALOG', 6: 'STRING', 7: 'FONTDIR',
         8: 'FONT', 9: 'ACCELERATOR', 10: 'RCDATA', 11: 'MESSAGETABLE', 12: 'GROUP_CURSOR',
         14: 'GROUP_ICON', 16: 'VERSION', 17: 'DLGINCLUDE', 19: 'PLUGPLAY', 20: 'VXD',
         21: 'ANICURSOR', 22: 'ANIICON', 23: 'HTML', 24: 'MANIFEST'}
EXT = {'BITMAP': '.bmp', 'ICON': '.ico.bin', 'HTML': '.html', 'MANIFEST': '.xml', 'VERSION': '.bin'}


def label(entry):
    return str(entry.name) if entry.name is not None else str(entry.id)


def walk(pe):
    """Yield (type, name, lang, rva, size) for every resource."""
    root = getattr(pe, 'DIRECTORY_ENTRY_RESOURCE', None)
    if root is None:
        return
    for t in root.entries:
        tname = TYPES.get(t.id, label(t)) if t.name is None else str(t.name)
        for n in getattr(t, 'directory', None) and t.directory.entries or []:
            for lang in getattr(n, 'directory', None) and n.directory.entries or []:
                d = lang.data.struct
                yield tname, label(n), lang.data.lang, d.OffsetToData, d.Size


def version_info(pe):
    out = {}
    for info in getattr(pe, 'FileInfo', []) or []:
        for entry in info:
            for table in getattr(entry, 'StringTable', []) or []:
                for k, v in table.entries.items():
                    out[k.decode('utf-8', 'replace')] = v.decode('utf-8', 'replace')
    fixed = getattr(pe, 'VS_FIXEDFILEINFO', None)
    if fixed:
        f = fixed[0]
        out['FileVersion (fixed)'] = (f'{f.FileVersionMS >> 16}.{f.FileVersionMS & 0xFFFF}.'
                                      f'{f.FileVersionLS >> 16}.{f.FileVersionLS & 0xFFFF}')
    return out


def string_tables(pe, items):
    """String table blocks hold 16 strings each: block id N covers ids (N-1)*16 to N*16-1."""
    out = []
    for tname, name, lang, rva, size in items:
        if tname != 'STRING' or not name.isdigit():
            continue
        data = pe.get_data(rva, size)
        at = 0
        for i in range(16):
            if at + 2 > len(data):
                break
            n = struct.unpack_from('<H', data, at)[0]
            text = data[at + 2:at + 2 + 2 * n].decode('utf-16-le', 'replace')
            at += 2 + 2 * n
            if text:
                out.append(((int(name) - 1) * 16 + i, text))
    return sorted(out)


def bmp_file(dib):
    """A BITMAP resource is a DIB without the 14-byte file header: add it."""
    header_size = struct.unpack_from('<I', dib, 0)[0]
    bits = struct.unpack_from('<H', dib, 14)[0] if header_size >= 16 else 8
    colours = struct.unpack_from('<I', dib, 32)[0] if header_size >= 36 else 0
    if not colours and bits <= 8:
        colours = 1 << bits
    offset = 14 + header_size + 4 * colours
    return b'BM' + struct.pack('<IHHI', 14 + len(dib), 0, 0, offset) + dib


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    pe = pefile.PE(argv[0], fast_load=True)
    pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_RESOURCE']])
    items = list(walk(pe))
    version = version_info(pe)
    if '--json' in argv:
        print(json.dumps({'version': version, 'resources': [
            {'type': t, 'name': n, 'lang': lang, 'size': s} for t, n, lang, _, s in items],
            'strings': string_tables(pe, items) if '--strings' in argv else []}, indent=2))
        return 0
    if '--version' in argv or len(argv) == 1:
        for k, v in version.items():
            print(f'{k:24s} {v}')
        if '--version' in argv:
            return 0
    if '--strings' in argv:
        for i, text in string_tables(pe, items):
            print(f'{i:6d}  {text!r}')
        return 0
    if '--extract' in argv:
        out = argv[argv.index('--extract') + 1]
        for tname, name, lang, rva, size in items:
            data = pe.get_data(rva, size)
            if tname == 'BITMAP':
                data = bmp_file(data)
            folder = os.path.join(out, tname.replace('/', '_'))
            os.makedirs(folder, exist_ok=True)
            with open(os.path.join(folder, f'{name}_{lang}{EXT.get(tname, ".bin")}'), 'wb') as f:
                f.write(data)
        print(f'{len(items)} resources written to {out}')
        return 0
    print()
    for tname, name, lang, _, size in items:
        print(f'{tname:14s} {name:24s} lang {lang:5d} {size:10,d} bytes')
    if not items:
        print('no resources')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
