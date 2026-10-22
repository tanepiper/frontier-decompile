#!/usr/bin/env python3
"""First look at a DOS or Windows 3.x executable (MZ, NE): what it is, how it was built, what it uses.

Usage: dosinfo.py BINARY [--imports] [--exports] [--strings MIN] [--pstrings MIN]

The 16-bit counterpart of tools/pe/peinfo.py. It prints:
  - the format: plain real-mode MZ, NE (16-bit protected mode: Windows 3.x, OS/2 1.x, Borland
    Pascal/C++ DPMI16), LE/LX (32-bit DOS extenders such as DOS/4GW) or PE (use peinfo.py);
  - MZ: load size, entry CS:IP and SS:SP, relocations, and data appended after the image
    (overlays, Borland .OVR, packed payloads);
  - NE: module name, description, entry point and stack, every segment with the selector
    Ghidra gives it (0x1000 + 8 * (N - 1)), its size, CODE/DATA and relocation count, the
    imported modules with how many places use each function, and the exports;
  - fingerprints: compiler and runtime (Turbo/Borland Pascal, Borland C++, Microsoft C, Watcom,
    DJGPP...), DOS extenders, packers (LZEXE, PKLITE, EXEPACK...) and middleware (sound and
    video libraries) found in the bytes;
  - `--strings MIN`: C-style strings; `--pstrings MIN`: Pascal strings (a length byte, then the
    text). Ghidra does not find Pascal strings by itself, so this is how to see a Pascal
    program's messages and file names with their addresses.

  - debug symbols: Borland Turbo Debugger symbols (read them with tdinfo.py) or a CodeView trailer.

It does not unpack packed programs, read resources or parse LE/LX images.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ne import FIRST_SELECTOR, MZ_LOAD, load  # noqa: E402
from tdinfo import find as find_tdinfo  # noqa: E402

FINGERPRINTS = {
    'Turbo Pascal / Borland Pascal runtime': [b'Runtime error ', b'Portions Copyright (c) 1983,9'],
    'Borland Pascal 7 DPMI16 (RTM.EXE, DPMI16BI.OVL)': [b'\x03RTM', b'DPMI16BI.OVL', b'RTM.EXE'],
    'Borland C++ / Turbo C runtime': [b'Borland C++', b'Turbo C', b'Turbo-C', b'xalloc'],
    'Microsoft C runtime': [b'MS Run-Time Library', b'R6001', b'Microsoft C'],
    'Watcom C/C++': [b'WATCOM C', b'Watcom C', b'WATCOM'],
    'DJGPP (32-bit GNU)': [b'go32', b'DJGPP', b'CWSDPMI'],
    'Free Pascal': [b'FPC ', b'Free Pascal'],
    'QuickBASIC / BASIC PDS': [b'QuickBASIC', b'BC.EXE', b'BRUN4'],
    'DOS/4GW or DOS/4G extender': [b'DOS/4G', b'DOS4GW'],
    'PMODE/W or PMODE extender': [b'PMODE/W', b'PMODE'],
    'Phar Lap extender': [b'Phar Lap'],
    'Ergo / Rational DOS extender': [b'Rational Systems', b'Ergo'],
    'LZEXE (packer)': [b'LZ91', b'LZ09'],
    'PKLITE (packer)': [b'PKLITE', b'PKlite'],
    'EXEPACK (packer)': [b'Packed file is corrupt'],
    'DIET (packer)': [b'diet', b'dlz'],
    'UPX (packer)': [b'UPX!'],
    'Borland overlay (.OVR, FBOV)': [b'FBOV'],
    'Miles Audio Interface Library': [b'Miles Design', b'AIL_', b'Audio Interface Library'],
    'HMI Sound Operating System': [b'Human Machine Interfaces', b'HMIDRV', b'sosDIG'],
    'DigPak / MidPak': [b'DIGPAK', b'MIDPAK'],
    'Sound Blaster (BLASTER variable)': [b'BLASTER'],
    'Gravis UltraSound (ULTRASND variable)': [b'ULTRASND'],
    'AdLib / OPL FM music': [b'AdLib', b'ADLIB'],
    'Fastgraph graphics': [b'Fastgraph'],
    'Smacker video': [b'SMK2', b'SMK4'],
    'FLIC animation (.FLI/.FLC)': [b'.FLI', b'.FLC'],
    'VESA BIOS (SVGA modes)': [b'VESA'],
    'Mouse driver (INT 33h)': [b'MOUSE', b'mouse driver'],
}


def fingerprint(key, data):
    """True when `key` appears as a whole word (ADLIB inside LOADLIBRARY does not count)."""
    pattern = re.escape(key)
    if key[:1].isalnum():
        pattern = rb'(?<![A-Za-z0-9])' + pattern
    if key[-1:].isalnum():
        pattern += rb'(?![A-Za-z0-9])'
    return re.search(pattern, data) is not None


def strings(data, minimum):
    for m in re.finditer(rb'[\x20-\x7e]{%d,}' % minimum, data):
        yield m.start(), m.group().decode()


def pascal_strings(data, minimum):
    """Length-prefixed strings: a byte n, then n printable bytes (spaces and \\r\\n allowed)."""
    i, n = 0, len(data)
    while i < n:
        length = data[i]
        inside_text = 32 <= length < 127 and i and 32 <= data[i - 1] < 127   # a letter of a longer text
        if length >= minimum and i + 1 + length <= n and not inside_text:
            text = data[i + 1:i + 1 + length]
            if all(32 <= c < 127 or c in (9, 10, 13) for c in text) and sum(c > 32 for c in text) * 2 >= length:
                yield i, text.decode('latin-1')
                i += 1 + length
                continue
        i += 1


def where(exe, file_off):
    """The Ghidra address of a file offset, or the bare file offset."""
    if exe.kind == 'NE':
        for s in exe.segments:
            if s.offset <= file_off < s.offset + s.length:
                return f'{s.selector:04x}:{file_off - s.offset:04x}'
    elif exe.kind == 'MZ':
        linear = file_off - exe.hdr_paras * 16
        if 0 <= linear < exe.image_end - exe.hdr_paras * 16:
            seg = MZ_LOAD + (linear >> 4)
            return f'{seg:04x}:{linear & 15:04x}'
    return f'file+{file_off:#x}'


def main(path, *flags):
    exe = load(path)
    data = exe.data
    print(f'{path}: {exe.kind}, {len(data):,} bytes')
    load_size = exe.image_end - exe.hdr_paras * 16
    print(f'  MZ: image {load_size:,} bytes, entry {exe.cs:04x}:{exe.ip:04x}, stack {exe.ss:04x}:{exe.sp:04x}, '
          f'{exe.nrel} relocations, min/max alloc {exe.minalloc:#x}/{exe.maxalloc:#x} paragraphs')
    if exe.kind == 'MZ' and len(data) > exe.image_end:
        extra = len(data) - exe.image_end
        print(f'  appended data: {extra:,} bytes after the load image (an overlay, or a packed payload)')
    if exe.kind == 'PE':
        print('  this is a Windows PE: use tools/pe/peinfo.py')
    elif exe.kind in ('LE', 'LX'):
        print(f'  {exe.kind} at {exe.lfanew:#x}: a 32-bit DOS-extender program (often DOS/4GW). Ghidra '
              'needs an LE/LX loader extension; the stub in front is the extender')
    elif exe.kind == 'NE':
        os_name = {1: 'OS/2', 2: 'Windows', 3: 'European DOS 4', 4: 'Windows 386', 5: 'BOSS'}.get(exe.target_os, f'{exe.target_os} (unknown)')
        print(f'  NE: module {exe.module_name!r}, description {getattr(exe, "description", "")!r}')
        print(f'      linker {exe.linker[0]}.{exe.linker[1]}, target OS byte {os_name}, flags {exe.flags:#06x}')
        print(f'      entry {FIRST_SELECTOR + 8 * (exe.ne_cs - 1):04x}:{exe.ne_ip:04x} (segment {exe.ne_cs}), '
              f'auto data segment {exe.autodata}, heap {exe.heap:#x}, stack {exe.stack:#x}')
        code = sum(s.length for s in exe.segments if not s.is_data)
        print(f'  segments: {len(exe.segments)} ({code:,} bytes of code, '
              f'{sum(s.length for s in exe.segments if s.is_data):,} bytes of data); '
              f'Ghidra selector, file offset, size in file, size in memory, kind, relocations:')
        for s in exe.segments:
            print(f'    {s.number:3d}  {s.selector:04x}  file {s.offset:#8x}  {s.length:#7x}  {s.minalloc:#7x}  '
                  f'{s.describe():<26} {s.relocs} relocs' + ('  (uninitialised)' if not s.offset else ''))
        print(f'  imported modules: {", ".join(exe.modules) or "none"}')
        for mod in dict.fromkeys(exe.modules):   # a module can be listed twice
            used = {k: n for k, n in exe.import_refs.items() if k.startswith(mod + '.')}
            print(f'    {mod:<10} {len(used)} functions, {sum(used.values())} references')
            if '--imports' in flags:
                for k, n in sorted(used.items(), key=lambda kv: (len(kv[0]), kv[0])):
                    print(f'        {k:<28} {n}')
        print(f'  exports: {len(exe.exports)}')
        if '--exports' in flags:
            for o, sel, off, name in exe.exports:
                print(f'    #{o:<4} {sel:04x}:{off:04x}  {name}')
    td = find_tdinfo(data)
    if td is not None:
        print(f'  debug symbols: Turbo Debugger (TDINFO) at file+{td:#x}: original names of procedures, '
              f'globals and types. Run tools/dos/tdinfo.py {path} summary')
    elif data[-8:-6] == b'NB':
        print(f'  debug symbols: a CodeView {data[-8:-4].decode()} trailer (not read by these tools)')
    found = [name for name, keys in FINGERPRINTS.items() if any(fingerprint(k, data) for k in keys)]
    print('  fingerprints: ' + (', '.join(found) if found else 'none'))
    if '--strings' in flags:
        for off, text in strings(data, int(flags[flags.index('--strings') + 1])):
            print(f'    {where(exe, off)}  {text}')
    if '--pstrings' in flags:
        for off, text in pascal_strings(data, int(flags[flags.index('--pstrings') + 1])):
            print(f'    {where(exe, off + 1)}  [{len(text)}] {text!r}')


if __name__ == '__main__':
    if len(sys.argv) < 2 or sys.argv[1] in ('-h', '--help'):
        print(__doc__)
        sys.exit(2)
    main(*sys.argv[1:])
