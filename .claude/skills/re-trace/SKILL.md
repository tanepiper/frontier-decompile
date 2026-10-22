---
name: re-trace
description: Find and understand how the original game implements something - from a string, a resource name, an import, an export, a C++ class name or a known caller to the function, its inputs, the fields it reads and writes, the constants it uses and the loop that calls it - and write the result down as evidence. Use for any "how does the game do X?" question before reimplementing or documenting it.
---

# Trace a behaviour

Tracing means following the program from something concrete to the code you need, and leaving a
written trail. Static reading only: label conclusions *observed*, *inferred* or *unknown*
(`re-start`). When the exact numbers matter, finish with `re-emulate`.

## 0. Has it been done?

```bash
grep -rn "1400be770\|SaveGame" docs/ labels/ | head
```

## 1. Find an entry point

For a DOS game, look for debug symbols first (`tools/dos/tdinfo.py BINARY summary`): with
them, the entry point is a name (`tdinfo.py BINARY procs enemy`), with its parameters, locals
and the source line of each address (`tdinfo.py BINARY lines UNIT.PAS`).


Work outward from something you can see. In rough order of usefulness:

1. **Strings.** File paths (`"Saves/"`, `"data/units.xml"`), format strings (`"%d citizens"`),
   UI text, error messages, debug and log text, script function names. `decomp_query.py strings
   WORD` lists them with the functions that use them. Developers leave log messages that name
   the function they are in.
2. **Exports with real names.** An engine DLL that exports C++ symbols
   (`?Serialize@Stream@IO@@...`) names hundreds of functions for free: `re-ghidra` `named`.
3. **Imports.** The OS and library calls show what a function does: `CreateFileW`/`ReadFile`
   (loading), `QueryPerformanceCounter` (timing), `XInputGetState` (input), `send`/`recv`
   (network), `D3D11CreateDevice` (rendering). Find their callers.
4. **RTTI.** MSVC binaries with RTTI contain `.?AVClassName@@` strings; each points to a type
   descriptor, which leads to the class's vtable and so to all its methods.
5. **Type registries and hashes.** Engines often register classes or resources under a hash of
   their name (CRC32, FNV-1a, murmur). If a table pairs constants with factories, hash candidate
   names and look the constants up.
6. **Data.** A value you can see in the game (a price, a duration) may appear as a constant
   (`readdata.py`) or in a data file (`re-format`). Its readers lead to the rules. A value
   that changes during play (gold, health) can be found in memory dumps with
   `tools/triage/memscan.py`. The code that writes that address is the rule
   (`docs/observe/memory.md`).
7. **Known constants.** `tools/triage/constants.py BINARY` names the random generators,
   checksums, hashes and decompressors by their constants and gives each address.
   `tools/triage/tables.py BINARY` finds sine and cosine tables. `tools/triage/insns.py
   BINARY '^(in|out) ' '^int '` lists every port access and interrupt: the hardware layer.
   `tools/triage/hashnames.py BINARY` finds the hash function behind names stored as numbers
   and names each hash constant (`docs/engines/engine-runtime.md`).
8. **Calls with their arguments.** `scripts/ghidra-run.sh BINARY callargs OUT FUNCTION...`
   lists every call to a function with its constant arguments: the sound ids, the file names,
   the random ranges. A log of the original's system calls shows the same from outside
   (`docs/observe/system-calls.md`).
9. **The main loop.** From `WinMain`/`main` (or the entry point) find the per-frame update; the
   systems hang off it in a fixed order, which you will need anyway (see `docs/GAME-PATTERNS.md`).

## 2. Read the function

Optimised code hides its source behind compiler forms: division by a constant becomes a
multiply and a shift, a `switch` becomes a jump table, small functions are copied into their
callers. `docs/COMPILED-CODE.md` lists these forms and how to find the game's own code past the
C runtime's start-up.

- Note its inputs: what `param_1` is (often `this`), the globals it reads (`DAT_...` pointers to
  singletons), and the callees that touch the OS, threads, files or the GPU.
- Name what you understand: a row in `labels/names.tsv` for the function and its callees, a row
  in `labels/fields.tsv` per field (`Citizen  0x44  hunger  f32  written by FUN_1400be770`).
- Follow the callers up until you reach a loop, a vtable slot or an event handler: a code path
  is not proof that it runs.

## 3. Reconstruct structures

- **Serializers first.** A `Save`/`Load`/`Serialize` function reads or writes every field in
  order with its type: the cleanest field list you will get.
- **Who writes a field:** `decomp_query.py writers 0x44`, then skip constructors and copies.
- **All the fields at once:** `decomp_query.py layout CTOR SERIALIZER USER:param_2` merges every
  access to one pointer across functions into a table and a draft struct. The allocation size
  (`operator new(0x148)`) gives the whole size. `docs/TYPES.md` has the shapes of lists, trees,
  hash tables and the C++ containers. Send `re-typer` for many structures at once.
- **Names stored as hashes:** a constant compared where a name would be is often a hash.
  `tools/triage/hashnames.py BINARY` finds the function and names the constants
  (`docs/engines/engine-runtime.md`).
- **Nested offsets** (`array + i * 0x1f0 + 0x90`): find getters that return sub-objects, subtract,
  and continue inside the sub-object.
- **vtables:** a constructor's first store is `*this = &vtable`. List it (`re-ghidra` `vtable`);
  slot offsets are what call sites use: `(**(code **)(*obj + 0x18))(obj)` calls slot +0x18.
  Base classes share early slots; compare tables to find the hierarchy.
- **Which argument is which:** a call's name does not tell you its argument order. Read the
  callee before trusting a guess written in a document.

## 4. Constants

Read every `DAT_` constant and table the function uses with `tools/pe/readdata.py`, and record
each with its address. Never round or "tidy" them: `0.33333331` is not `1/3` in a 32-bit float
simulation.

## 5. Write it down (`re-land`)

A topic document in `docs/` (extend one if it exists): an opening paragraph (what, which binary
and version, which evidence), then prose and tables keyed by address and offset: function →
role, fields, constants, the call order, and **Open questions**. Never paste pseudocode or
disassembly: describe it. Update `docs/CHECKLIST.md`.

## Pitfalls

- Pseudocode drops and mis-types things (see `re-ghidra` section 5). For arithmetic, read the
  instructions; for exact values, emulate.
- A function may be inlined into each caller; a "missing" function is often inside its caller.
- Functions reached only through tables are often absent from the bulk export: create them with
  `scripts/ghidra-run.sh BINARY decompile OUT 0xADDRESS`.
- Two subsystems may share a helper with different meanings for its arguments. Name helpers by
  what they do, not by the first place you saw them.
