#!/usr/bin/env python3
"""List the pointers in a PE file's data, found from its base relocations.

Usage: relocs.py BINARY [--min N] [--all]

The loader must change every absolute address in the image when it loads the image at another
base. The base relocation table (.reloc) lists each of those places. In code, they are
instructions that use an absolute address. In data, each one is a pointer. Runs of pointers
next to each other are tables: vtables, tables of functions, tables of strings, tables of
records. A recursive disassembler misses functions that are reached only through such tables.

The tool prints each run of at least N pointers (default 2) outside the code sections, with
the kind of each target: code, a string, or other data. --all also prints single pointers.
DLLs keep their relocations. Many 32-bit EXEs were linked without them (/FIXED); the tool says
so. 64-bit code uses relative addresses, so its code has few relocations, while its data
pointers are still listed.
"""
import sys

import pefile

KINDS = {pefile.RELOCATION_TYPE['IMAGE_REL_BASED_HIGHLOW']: 4, pefile.RELOCATION_TYPE['IMAGE_REL_BASED_DIR64']: 8}


def section_of(pe, rva):
    for s in pe.sections:
        if s.VirtualAddress <= rva < s.VirtualAddress + max(s.Misc_VirtualSize, s.SizeOfRawData):
            return s
    return None


def is_code(s):
    return bool(s and s.Characteristics & 0x20000000)  # IMAGE_SCN_MEM_EXECUTE


def looks_like_text(pe, rva):
    try:
        data = pe.get_data(rva, 64)
    except pefile.PEFormatError:
        return False
    end = data.find(b'\0')
    return end >= 3 and all(32 <= b < 127 or b in (9, 10, 13) for b in data[:end])


def kind_of(pe, target_rva):
    s = section_of(pe, target_rva)
    if s is None:
        return 'header' if 0 <= target_rva < pe.OPTIONAL_HEADER.SizeOfHeaders else 'outside'
    if is_code(s):
        return 'code'
    return 'string' if looks_like_text(pe, target_rva) else 'data'


def pointers(pe):
    """Yield (rva, size, target_rva) for each relocated pointer outside the code sections."""
    base = pe.OPTIONAL_HEADER.ImageBase
    for block in getattr(pe, 'DIRECTORY_ENTRY_BASERELOC', []):
        for e in block.entries:
            size = KINDS.get(e.type)
            if not size or is_code(section_of(pe, e.rva)):
                continue
            raw = pe.get_data(e.rva, size)
            target = int.from_bytes(raw, 'little') - base
            yield e.rva, size, target


def runs(items):
    """Group pointers that sit next to each other into runs."""
    run = []
    for item in sorted(items):
        if run and item[0] != run[-1][0] + run[-1][1]:
            yield run
            run = []
        run.append(item)
    if run:
        yield run


def main(argv):
    min_run = 2
    if '--min' in argv:
        i = argv.index('--min')
        min_run = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    if '--all' in argv:
        argv.remove('--all')
        min_run = 1
    if len(argv) != 1:
        sys.exit(__doc__)
    pe = pefile.PE(argv[0], fast_load=True)
    pe.parse_data_directories([pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_BASERELOC']])
    if not hasattr(pe, 'DIRECTORY_ENTRY_BASERELOC'):
        print('no base relocations: the image was linked to load only at its preferred base (/FIXED)')
        return
    base = pe.OPTIONAL_HEADER.ImageBase
    total = shown = 0
    for run in runs(pointers(pe)):
        total += len(run)
        if len(run) < min_run:
            continue
        shown += 1
        kinds = [kind_of(pe, t) for _, _, t in run]
        summary = ', '.join(f'{kinds.count(k)} {k}' for k in ('code', 'string', 'data', 'header', 'outside') if k in kinds)
        sec = section_of(pe, run[0][0])
        name = sec.Name.rstrip(b'\0').decode(errors='replace') if sec else '?'
        print(f'{base + run[0][0]:#x}  {name:8} {len(run):4} pointers  ({summary})')
        for rva, _, target in run[:8]:
            print(f'    +{rva - run[0][0]:#06x}  -> {base + target:#x}  {kind_of(pe, target)}')
        if len(run) > 8:
            print(f'    ... {len(run) - 8} more')
    print(f'{total} pointers in data, {shown} runs shown')


if __name__ == '__main__':
    main(sys.argv[1:])
