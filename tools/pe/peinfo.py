#!/usr/bin/env python3
"""First look at a Windows executable or DLL: what it is, how it was built, what it uses.

Usage: peinfo.py BINARY [--imports] [--exports] [--strings MIN]

Prints the machine (x86 or x64), image base, entry point, sections, the PDB path and build
date when present, the linker and compiler hints in the Rich header, every imported DLL with
its function count, the export count, and *engine fingerprints*: names in the imports and
strings that point to a known engine, middleware or protection (Unity, Unreal, FMOD, Steam,
Denuvo...). Run it on every binary in the game folder before anything else; it tells you which
binaries hold the game's own code and which tools apply (see the re-inventory skill).
"""
import re
import sys
from datetime import datetime, timezone

import pefile

FINGERPRINTS = {
    'Unity (Mono or IL2CPP)': [b'UnityPlayer', b'il2cpp', b'mono_runtime', b'GameAssembly'],
    'Unreal Engine': [b'UE4', b'UnrealEngine', b'FEngineLoop', b'UObject'],
    'Godot': [b'Godot Engine', b'godot'],
    'Source / Source 2': [b'tier0.dll', b'vstdlib', b'engine2.dll'],
    'id Tech / Quake family': [b'idTech', b'quake'],
    'GameMaker': [b'GameMaker', b'YoYo Games'],
    'RPG Maker': [b'RGSS', b'RPG Maker'],
    '.NET (managed code)': [b'mscoree.dll', b'_CorExeMain'],
    'Lua scripting': [b'lua_pcall', b'luaL_', b'LuaJIT'],
    'Python scripting': [b'Py_Initialize', b'python3'],
    'FMOD audio': [b'fmod.dll', b'fmod64.dll', b'fmodstudio', b'FMOD_System'],
    'Wwise audio': [b'AK::SoundEngine', b'Wwise'],
    'Bink video': [b'bink', b'Bink'],
    'Havok physics': [b'Havok', b'hkBase'],
    'PhysX': [b'PhysX'],
    'Scaleform UI': [b'Scaleform', b'GFx'],
    'Steamworks': [b'steam_api', b'SteamAPI_'],
    'GOG Galaxy': [b'Galaxy64.dll', b'galaxy.dll'],
    'Direct3D 9': [b'd3d9.dll'],
    'Direct3D 11': [b'd3d11.dll'],
    'Direct3D 12': [b'd3d12.dll'],
    'OpenGL': [b'opengl32.dll'],
    'Vulkan': [b'vulkan-1.dll'],
    'SDL': [b'SDL2.dll', b'SDL_Init'],
    'Denuvo (anti-tamper)': [b'denuvo', b'Denuvo'],
    'VMProtect / Themida (packer)': [b'.vmp0', b'.themida', b'.winlice'],
    'UPX (packer)': [b'UPX0', b'UPX1'],
}


def main(path, *flags):
    pe = pefile.PE(path)
    data = open(path, 'rb').read()
    machine = {0x14c: 'x86 (32-bit)', 0x8664: 'x64 (64-bit)', 0xaa64: 'ARM64'}.get(
        pe.FILE_HEADER.Machine, hex(pe.FILE_HEADER.Machine))
    kind = 'DLL' if pe.FILE_HEADER.Characteristics & 0x2000 else 'EXE'
    base = pe.OPTIONAL_HEADER.ImageBase
    print(f'{path}: {kind}, {machine}, image base {base:#x}, entry {base + pe.OPTIONAL_HEADER.AddressOfEntryPoint:#x}')
    stamp = pe.FILE_HEADER.TimeDateStamp
    print(f'  linked: {datetime.fromtimestamp(stamp, timezone.utc):%Y-%m-%d %H:%M} UTC (may be a fake or hash)')
    for d in getattr(pe, 'DIRECTORY_ENTRY_DEBUG', []):
        if getattr(d.entry, 'PdbFileName', None):
            print(f'  PDB: {d.entry.PdbFileName.decode(errors="replace").rstrip(chr(0))}')
    rich = pe.parse_rich_header()
    if rich:
        ids = sorted({v >> 16 for v in rich['values'][::2]})
        print(f'  Rich header: {len(rich["values"]) // 2} tool entries (MSVC-built; product ids {ids[:8]}...)')
    print('  sections:')
    for s in pe.sections:
        name = s.Name.rstrip(b'\0').decode(errors='replace')
        flags = ''.join(c for c, bit in (('r', 0x40000000), ('w', 0x80000000), ('x', 0x20000000))
                        if s.Characteristics & bit)
        print(f'    {name:<9} {base + s.VirtualAddress:#x}  size {s.Misc_VirtualSize:#9x}  {flags}  entropy {s.get_entropy():.2f}')
    imports = getattr(pe, 'DIRECTORY_ENTRY_IMPORT', [])
    print(f'  imports: {len(imports)} DLLs')
    for d in imports:
        print(f'    {d.dll.decode():<28} {len(d.imports)} functions')
        if '--imports' in flags:
            for i in d.imports:
                print(f'        {i.name.decode() if i.name else "#" + str(i.ordinal)}')
    delayed = getattr(pe, 'DIRECTORY_ENTRY_DELAY_IMPORT', [])
    if delayed:
        print(f'  delay-loaded imports: {len(delayed)} DLLs (loaded on the first call, not in the import table)')
        for d in delayed:
            print(f'    {d.dll.decode():<28} {len(d.imports)} functions')
            if '--imports' in flags:
                for i in d.imports:
                    print(f'        {i.name.decode() if i.name else "#" + str(i.ordinal)}')
    relocs = sum(len(b.entries) for b in getattr(pe, 'DIRECTORY_ENTRY_BASERELOC', []))
    print(f'  base relocations: {relocs}' + (' (tools/pe/relocs.py lists the pointers in data)' if relocs
                                             else ' (none: linked to load only at its base, /FIXED)'))
    exports = getattr(pe, 'DIRECTORY_ENTRY_EXPORT', None)
    n = len(exports.symbols) if exports else 0
    mangled = sum(1 for e in (exports.symbols if exports else []) if e.name and e.name.startswith(b'?'))
    print(f'  exports: {n}' + (f' ({mangled} MSVC C++ names: demangle them for class and method names)' if mangled else ''))
    if '--exports' in flags and exports:
        for e in exports.symbols:
            print(f'    {base + e.address:#x}  {e.name.decode() if e.name else "#" + str(e.ordinal)}')
    rtti = len(re.findall(rb'\.\?AV[\w@]+@@', data))
    if rtti:
        print(f'  RTTI type descriptors: {rtti} (C++ class names are recoverable)')
    found = [name for name, keys in FINGERPRINTS.items() if any(k in data for k in keys)]
    print('  fingerprints: ' + (', '.join(found) if found else 'none'))
    high = [s for s in pe.sections if s.get_entropy() > 7.2 and s.Characteristics & 0x20000000]
    if high:
        print('  warning: high-entropy executable section: packed or encrypted code; '
              'static analysis will see little until it is unpacked')
    if '--strings' in flags:
        min_len = int(flags[flags.index('--strings') + 1])
        for m in re.finditer(rb'[\x20-\x7e]{%d,}' % min_len, data):
            print(f'    {m.start():#x}  {m.group().decode()}')


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    main(*sys.argv[1:])
