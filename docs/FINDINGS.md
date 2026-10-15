# Findings

What is known about the shipped DOS build of Frontier: Elite II. Evidence is static: MZ headers read
with `crates/formats::mz`, Ghidra 12.1.4 headless analysis (`tools/ghidra/export.sh`), REA load-image
verification, and one observed boot under DOSBox Staging 0.83.0. Nothing here was traced through
execution yet.

## 1. The build

`game-data/MANIFEST.sha256` records the exact files. `FRONTIER.EXE` SHA-256
`0b7c477eb68f43785194f1d489277c7bb10bdeb743a3b9a0a4092b1692456982`, 334,028 bytes, dated
29 Aug 1998 (a re-release; `EL2M12.OVL` is dated 30 Dec 2000 and has a different header size, so it
may be patched — *inferred*).

| File(s) | What | Evidence |
|---|---|---|
| `FRONTIER.EXE` | Main program, DOS MZ | header |
| `EL2M00`–`EL2M19.OVL` | 20 overlays, all MZ images with entry `0000:0000` and SP `0x200` | header |
| `EL2MCGA3.OVL` | MCGA/VGA 320×200 video driver (*inferred* from name) | name, string in EXE |
| `EL2MATH3.OVL` | Maths routines (*inferred*) | name, string in EXE |
| `EL2ADLIB.OVL`, `EL2ROL.OVL`, `EL2SPK.OVL` | AdLib, ROL music, PC speaker sound drivers (*inferred*) | names, strings in EXE |
| `EL2SETUP.OVL` | Setup/config overlay (*inferred*) | name, string in EXE |
| `*.ROL` / `*.rol` | Standard MIDI files (`MThd`), despite the extension | magic |
| `EL2G.GRF`, `PIC000.GRF` | Graphics. `PIC000.GRF` opens with 6-bit VGA palette bytes | bytes |
| `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` | Starting positions or scenarios (*inferred*); unknown format | names |
| `LATIS._01`–`._04`, `COMMANDR/` | 2010-dated additions (`COMMANDR/` is empty) | dates |

## 2. `FRONTIER.EXE` load image

REA (`inspect_native_load_image`, status **verified**) confirms Ghidra's import: 1,536-byte header,
332,492-byte module, 268 relocations, entry `0000:008f` (linear `0x1008f` at load segment `0x1000`),
initial `SS:SP = 512c:000c`, no appended overlay bytes. Ghidra splits it into code segments
`CODE_0`–`CODE_8`.

The code is real-mode x86 that heavily uses 32-bit registers through the `0x66` operand-size prefix
(`push eax; push ebx; …` runs are visible as `fPfS` strings). The DOS original is known to be a
machine translation of the 68000 Amiga/ST code (*inferred* from the register usage pattern, not yet
shown).

Strings include the full ship list (Missile … Long-range Cruiser), station signage, and the overlay
names the loader opens. `"Not enough free memory$"` is a DOS `$`-terminated message.

## 3. Ghidra coverage (the first problem)

Default auto-analysis finds only 29 functions in `FRONTIER.EXE` and about one per overlay. The
decompiler also treats the overlays' image start as code, which it is not reliably. Reasons
(*inferred*): the code is reached through tables and register-indirect calls, and overlays are loaded
by the program's own loader, so Ghidra sees no call graph into them.

Next steps (M1):
1. Trace the overlay loader from the strings `EL2*.OVL`, recover how an overlay's entry table is
   laid out, and where it is placed in memory (segment) so overlay addresses can be related to
   main-program calls.
2. Write a Ghidra script that seeds functions from call targets, jump tables and the overlay
   entry tables, then re-run the export.
3. Record function roles in a names table as they are identified.

## 4. Observed in the original (DOSBox)

DOSBox Staging 0.83.0, `tools/dosbox/frontier.conf` (S3 SVGA machine, 16 MB, 25000 cycles, SB16).

- From a fresh `game-data/run`, the program first prints a text-mode prompt, "Please select sound
  support required", with options 1 Roland LAPC1, 2 AdLib / SoundBlaster, 3 PC Internal speaker,
  4 No sound. This presumably selects between `EL2ROL`, `EL2ADLIB` and `EL2SPK` (*inferred*).
- In the very first session it instead switched to VGA mode 13h (320×200, 256 colours) about 6 s
  after launch and showed the cockpit landed at Merlin, 12:00:01 1-Jan-3200, with no intro and no
  menu. Whether that run got past the prompt through stray input is unknown. Re-check this by
  answering the prompt deliberately.

## Open questions

- What are `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` and the `LATIS._0n` files?
- After the sound prompt, does the game play the intro and offer the start-position menu (`LAVE`, `MARS`, …)?
- How does the program choose between `EL2ADLIB`, `EL2ROL` and `EL2SPK`?
- Does the main program run entirely in real mode, or does it switch to protected or unreal mode?
