---
name: frontier-trace
description: Static reverse engineering of FRONTIER.EXE and the EL2*.OVL overlays with the local Ghidra export, headless one-off decompiles and the REA MCP tools. Use when you need to find or understand how the original game implements something (a routine, table, constant, file format reader, overlay entry) before porting it.
---

# Trace behaviour in the original binaries

Static evidence only: never claim the game "does" something at runtime unless it was emulated
(`frontier-emulate`) or observed in DOSBox. Label conclusions **observed**, **inferred** or
**unknown**.

## 0. Check what is already known

```bash
grep -rn "1000:2f3a\|EL2M07" docs/ | head
ls analysis/
```

## 1. Find an entry point

Work outward from something concrete: a string (ship names, overlay file names, error messages),
a file the game opens, a DOS/BIOS interrupt (`int 21h` file I/O, `int 10h` video, `int 33h` mouse),
or a known caller.

```bash
D=analysis/export/FRONTIER.EXE
grep -n 'EL2MCGA3' $D/strings.txt
awk '/^\/\/ ==== 1000:049c /{p=1;print;next} /^\/\/ ==== /{p=0} p' $D/decomp.c
```

Ghidra's default analysis misses most functions here (see `docs/FINDINGS.md` §3). If an address
is not in `functions.tsv`, decompile it on demand (step 2) rather than assuming it is data.

## 2. REA and one-off decompiles

REA (`reverse-engineer-anything` skill, `mcp__rea__*`) opens a DOS MZ directly with Ghidra:
`open_binary` with the absolute path under `game-data/original/`, then `analyze_function`,
`search_strings`, `xrefs`, `read_bytes`, `address_to_file_offset`. `inspect_native_load_image`
verifies the MZ mapping. Its output can be large: take the saved JSON with `jq`. Close with
`close_binary` when done.

For batches of addresses in the existing project (no re-analysis):

```bash
G=/opt/homebrew/opt/ghidra/libexec
JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
"$G/support/analyzeHeadless" analysis/ghidra-project frontier \
  -process FRONTIER.EXE -noanalysis -scriptPath tools/ghidra \
  -postScript ExportAddressBatch.java analysis/<topic>.txt 1000:049c 1000:0550
```
Ghidra locks a project per run; don't run two headless jobs on it at once.

## 3. Read the instructions when it matters

The pseudocode is weak on this code (16-bit segments, `0x66`-prefixed 32-bit arithmetic, flags
used across calls, translated 68000 idioms). Before porting arithmetic, read the disassembly:
REA `analyze_function` returns assembly, or `objdump -D -b binary -m i8086` over a byte range of
the load image (file offset = linear − `0x10000` + header size). Watch carry/overflow use, `sar`
vs `shr`, `imul` widths, and `0x66`/`0x67` prefixes.

## 4. Overlays

Each `EL2*.OVL` is analysed as its own program, so its addresses start at its own load segment.
Record overlay addresses as `EL2Mnn:offset` and note how the main program reaches them once the
loader is traced.

## 5. Record

Write findings into the topic doc (`frontier-land`): address → role tables, structure layouts with
offsets, constants with addresses, and **Open questions**. Throwaway dumps go in `analysis/`.
