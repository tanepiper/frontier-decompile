---
name: re-ghidra
description: Decompile a game's binaries with Ghidra in headless mode and work with the result as text - the full export (decomp.c, functions, strings, imports, exports), queries with decomp_query.py, one-off decompiles of functions Ghidra missed, vtable and reference listings, constants read from the binary, and checks against the disassembly. Use whenever you need pseudocode for a native binary, including 16-bit DOS (MZ) and NE programs.
---

# Decompile with Ghidra

Ghidra turns machine code into C-like *pseudocode*. It has no original names: a function is
`FUN_1400be770` (the function at address 0x1400be770), a field is `*(int *)(param_1 + 0x44)`
(the int 0x44 bytes into the first argument). Pseudocode helps you read the code, but it can
be wrong. Check anything exact against the disassembly or the emulator.

## 1. Export once

Import the game's own binaries **together**, so calls between the EXE and its DLLs resolve:

```bash
scripts/ghidra-export.sh target/Game.exe target/GameEngine.dll
```

If you have names for functions and globals (debug symbols, a map file, earlier labels), give
them to the export: Ghidra names everything and creates the functions it missed.

```bash
python3 tools/dos/tdinfo.py target/GAME.EXE labels > analysis/td-labels.tsv   # DOS: Borland symbols
scripts/ghidra-export.sh --labels analysis/td-labels.tsv target/GAME.EXE
scripts/ghidra-run.sh GAME.EXE labels analysis/notes/labels.txt labels/names.tsv  # an existing project
```

It writes, per binary, into `analysis/export/<binary>/` (ignored by git):

| File | Contents |
|---|---|
| `decomp.c` | Every function's pseudocode, each headed `// ==== <address> <name>` |
| `functions.tsv` | address, size in bytes, name, `thunk` for thunks |
| `strings.txt` | address and value of every defined string |
| `imports.txt`, `exports.txt` | library!function; address and name of each export |

Time: about a minute for a small program, 10 to 30 minutes for a large game. Run it in the
background and keep working. It needs memory: `MAXMEM=16G` for games over 50 MB of code.

## 2. Query the export

```bash
T=tools/triage/decomp_query.py; P=.venv/bin/python
$P $T stats                      # functions, decompiled, labelled share
$P $T fn 1400be770               # one function, with your labels applied
$P $T callers FUN_1400be770      # who calls it (or refers to it through a table)
$P $T callees 1400be770          # what it calls, labelled and largest first
$P $T find 'Saves/'              # functions whose pseudocode contains a string or constant
$P $T strings savegame           # strings containing a word, and the functions that use them
$P $T writers 0x44               # functions that store to +0x44 (who sets this field?)
$P $T layout 140010a0 140011f0   # one structure's fields from several functions, and a draft struct
$P $T biggest 40                 # the largest unlabelled functions (where the work is)
```

Plain `grep` and `awk` on `decomp.c` work too: `awk '/^\/\/ ==== 1400be770 /{p=1;print;next}
/^\/\/ ==== /{p=0} p' analysis/export/Game.exe/decomp.c` prints one function.

Every time you identify a function, add `address<TAB>label<TAB>evidence` to `labels/names.tsv`;
`decomp_query.py` and `tools/pe/vtable.py` show labels everywhere from then on. Fields go in
`labels/fields.tsv`. This shared vocabulary is what makes later sessions fast.

## 3. One-off work in the analysed project

Reuse the project the export made (no re-analysis, about a minute of start-up, so batch):

```bash
scripts/ghidra-run.sh Game.exe decompile analysis/notes/ai.txt 0x1400be770 0x1400be680
scripts/ghidra-run.sh Game.exe named     analysis/notes/rand.txt Random Rand
scripts/ghidra-run.sh Game.exe refs      analysis/notes/refs.txt 0x140178000
scripts/ghidra-run.sh Game.exe vtable    analysis/notes/vt.txt Citizen 0x140165120 40
scripts/ghidra-run.sh Game.exe callargs  analysis/notes/sfx.tsv PlaySound play_sfx
scripts/ghidra-run.sh boot.bin blocks    analysis/notes/blocks.txt derived/loadmap.tsv
```

- `decompile` also **creates** functions Ghidra missed: callbacks reached only through tables
  of pointers are often not functions in the bulk export.
- `named` searches by name: best for a DLL that exports C++ names (`Random`, `Serialize`).
- `refs` lists every reference to an address (a global, a table, a function) with the
  referring function decompiled: the fastest way to find who uses a constant or a singleton.
- `vtable` lists a table of function pointers. `tools/pe/vtable.py BINARY 0xTABLE --names
  analysis/export/BINARY/functions.tsv` does the same without Ghidra.

- `callargs` lists every call to the named functions with the constant, string and global
  arguments at each call, from P-Code, for any processor: every sound and its id, every file
  name, every random range in one pass.
- `blocks` adds raw pieces of memory from a load map (a booter, a memory dump, ROM banks) and
  analyses them.

Ghidra locks a project: run one job per project at a time.

Signatures carry names without an export: `tools/triage/sigscan.py make BINARY --names
labels/names.tsv > SIGS`, then `sigscan.py scan SIGS NEW_BINARY --labels OUT.tsv`.

A new build of the game? `tools/triage/match_functions.py OLD_EXPORT NEW_EXPORT --labels
labels/names.tsv --out labels/names-new.tsv` matches the functions and carries your labels to
the new addresses. `docs/working/ghidra.md` covers this, Version Tracking, BSim, Function ID
and processors Ghidra lacks.

## 4. Constants and data

Pseudocode often shows a float constant as a global (`DAT_14016a2b0`). Read the real value:

```bash
.venv/bin/python tools/pe/readdata.py target/Game.exe f32 DAT_14016a2b0
.venv/bin/python tools/pe/readdata.py target/Game.exe f32 0x14016a2c0 --count 12   # a table
.venv/bin/python tools/pe/readdata.py target/Game.exe cstr 0x140180010
```

## 5. Check against the disassembly

Before you reimplement arithmetic, loops or bit tricks, read the instructions; Ghidra drops loop
bounds, mis-types floats, merges stack slots and loses register arguments:

```bash
objdump -d --no-show-raw-insn --start-address=0x1400be680 --stop-address=0x1400be770 \
  target/Game.exe > analysis/notes/be680.s
```

What to look for: `cvttss2si` (float to int, truncating) against `cvtss2si` (rounding by the
current mode); `minss`/`maxss` operand order (it decides which value wins with NaN); `movss`
(32-bit float) against `movsd` (64-bit double); x87 instructions (`fld`, `fstp`) in 32-bit
games, which compute in 80-bit precision until stored; `imul`/`idiv` (signed) against
`mul`/`div` (unsigned); `sar` (keeps the sign) against `shr`.

## Pitfalls

- **Inlining.** A small function may exist only inside its callers. If `callers` finds none
  but the code clearly runs, search for its body's distinctive constants instead.
- **Magic numbers as integers.** A check for the bytes "SMPL" compiles to `== 0x4c504d53`
  (little-endian). Search for the hex value as well as the text.
- **Decimal offsets.** Ghidra prints some offsets in decimal: `param_1 + 100` is `+0x64`.
  `writers` matches both.
- **Thunks.** A `FUN_` that only jumps elsewhere is a thunk: follow it (functions.tsv marks them).
- **Wrong signatures.** Ghidra guesses argument counts and types; a call through a register it
  did not track shows fewer arguments than the callee uses. The callee's own pseudocode and the
  disassembly at the call site settle it.
- **32-bit games** pass arguments on the stack (cdecl, stdcall) and `this` in ecx (thiscall);
  see `docs/ABI.md`.
- **Paths with a dot folder.** Ghidra refuses a project path with a folder that starts with
  "." (a git worktree in `.claude/worktrees/`). The scripts handle it with a link in `$TMPDIR`
  (`ghidra_project_dir` in `scripts/lib.sh`); call `analyzeHeadless` yourself the same way.

## 16-bit programs (DOS MZ, NE)

The same export works. Addresses are `seg:off`: NE segment N is `1000 + 8 * (N - 1)`
(`1000:873a`, `FUN_1000_873a`, `DAT_1048_12de`); an MZ image starts at `1000:0000`.
`decomp_query.py` accepts these addresses and labels. Pitfalls (details in docs/DOS.md):

- The export passes `-loader-libraryLoadDepth 0`: Ghidra's library search crashed on an NE
  program whose imported module (`RTM`) had a plain DOS file of that name next to it.
- Ghidra does not know `INT 21h AH=4Ch` or Pascal's `Halt` never return: the next function is
  glued onto the caller. Create the function at the right address (labels, or `decompile`).
- Pascal procedures pop their own arguments and take them left to right: Ghidra's `param_1` is
  the *last* source argument. Count from `[bp+6]` upwards, the first argument highest.
- Pascal strings (length byte first) are missing from `strings.txt`: use
  `tools/dos/dosinfo.py BINARY --pstrings 6` and `tools/dos/readseg.py BINARY pstr ADDR`.
- Disassembling the file directly shows far calls as `lcall 0:ffff` (unrelocated). Use
  `tools/dos/nedis.py BINARY SEG:OFF LEN`: relocations applied, calls and globals named.
- A Pascal string is referenced by its length byte (one before the text). A procedure's strings sit just before its code. A constant equal to a DS offset may print as `&global`.
- `INT 34h`-`3Dh` in Borland code are floating-point instructions (the FPU emulator).
- Constants: `tools/dos/readseg.py BINARY TYPE ADDR` reads typed values at `seg:off`
  (`u16`, `i32`, `ptr`, `real48` for Turbo Pascal's 6-byte Real, `pstr`, `hex`).

