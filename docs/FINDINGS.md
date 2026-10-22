# Findings

What is known about the shipped DOS build of Frontier: Elite II. Evidence: MZ headers read with
`crates/formats::mz`, Ghidra 12.1.4 headless analysis (`tools/ghidra/export.sh`), REA load-image
verification, disassembly of the loader paths, boots under DOSBox Staging 0.83.0, and runs of the
original on the project's own reference runtime (`crates/dos`, [RUNTIME.md](RUNTIME.md)) from boot
to the cockpit.

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

## 3. Ghidra coverage and the overlay loader

Default auto-analysis found only 29 functions in `FRONTIER.EXE` and about one per overlay, and it
decompiled each overlay's image start as code. The cause is now known (see
[OVERLAYS.md](OVERLAYS.md)):

- Nearly every call in the main program is `lcall [disp16]` through a far-pointer vector table at
  `DS:0000`–`0357` (main data segment, file paragraph `0x1615`). Slots `0000`–`01cf` hold the
  program's own routines. The rest are filled at run time by the video, maths, sound and set-up
  overlays.
- Overlays are separate DOS programs. `1000:d34c` runs them with DOS EXEC (`4B00h`). Their common
  94-byte stub at `0000` stays resident (`3100h`) at its own load segment and returns the address of
  an export block at `005e` through a far pointer encoded as letters on its command line. Ghidra had
  run the stub on into that block.
- `EL2M00`–`EL2M19` are demand-loaded modules in a 64-slot table at `DS:0396`. Lazy thunks
  (`1000:d5b4`…) build the name `EL2Mnn.OVL` from the slot number, load the module and jump to one of
  its 12 handlers. Unlocked modules are evicted when DOS runs out of memory.

`tools/ghidra/SeedFunctions.java` (run by `export.sh`) seeds functions from these tables, the
overlay exports and module headers, call targets and near jump tables. It also sets call overrides
so `lcall [vec]` decompiles as a direct call. Coverage went from 29 to 342 functions in
`FRONTIER.EXE` (82 KB of code in functions) and from 36 to 741 across the overlays.

M1 status:
1. ~~Trace the overlay loader and the overlay entry tables~~: done, [OVERLAYS.md](OVERLAYS.md).
2. ~~Seed functions from call targets, jump tables and the overlay entry tables~~: done.
3. ~~Find the code that runs but is in no function~~: done by recording coverage on the runtime
   ([COVERAGE.md](COVERAGE.md)). `FRONTIER.EXE` executes only between `1000:008f` and `2614:000d`,
   about 90 KB. The rest of the 332 KB image is the main data segment from `2615:0000`
   (*inferred*). Seeding from 39 runs took it to 409 functions and 86 KB, about 95 % of the code.
4. Resolve the remaining indirect calls (computed module slots, register calls) and record function
   roles in a names table as they are identified.

## 4. Observed in the original (DOSBox)

DOSBox Staging 0.83.0, `tools/dosbox/frontier.conf` (S3 SVGA machine, 16 MB, 25000 cycles, SB16).

- From a fresh `game-data/run`, the program first prints a text-mode prompt, "Please select sound
  support required", with options 1 Roland LAPC1, 2 AdLib / SoundBlaster, 3 PC Internal speaker,
  4 No sound. Statically, `EL2SETUP:04af` stores 1, 2 or 4 in `DS:8920`, and `1000:ee5b` loads
  `EL2ROL`, `EL2ADLIB` or `EL2SPK`. Option 4 loads no driver ([OVERLAYS.md](OVERLAYS.md) §5).
- In the very first session it instead switched to VGA mode 13h (320×200, 256 colours) about 6 s
  after launch and showed the cockpit landed at Merlin, 12:00:01 1-Jan-3200, with no intro and no
  menu. Whether that run got past the prompt through stray input is unknown. Re-check this by
  answering the prompt deliberately.

## 5. The original running on the reference runtime

`crates/dos` runs the user's `FRONTIER.EXE` and overlays on an in-process 386 real-mode
interpreter with the DOS/BIOS/EMS/mouse services the program uses ([RUNTIME.md](RUNTIME.md)). The
`frontier` app shows it natively and in the browser. **Observed** on it:

- The overlay loader runs as traced statically: `EL2SETUP`, `EL2MATH3`, `EL2MCGA3`, then
  `EL2M00`–`EL2M17` through `modules_init_all`. `EL2M08` is evicted under memory pressure and
  reloaded at start-up.
- The program allocates 48 EMS pages (768 KB), sets mode 13h and reads `EL2G.GRF`.
- The intro plays: the GameTek + Konami card, "DAVID BRABEN'S", planets and the 3D title. Then the
  **Select Start Position** menu appears: 1 Ross 154 (recommended), 2 Mars, 3 Lave, 4 Last Saved
  Position, 5 Select Position from disc. DOSBox shows the same screens, compared by eye.
- Choosing 1 lands the player at **Merlin, Ross 154, 12:00:01 1-Jan-3200**, cockpit view, with the
  clock running. This is the screen the first DOSBox session reached, so that session had most
  likely been sent through the prompt and menu by stray input.
- Scripted past the cockpit ([COVERAGE.md](COVERAGE.md)): every F1–F4 screen, the station's
  launch request, shipyard, bulletin board and stockmarket, the preferences screen (Esc twice),
  saving `COMMANDR\JAMESON._01` and loading it back, launch with clearance, climbing out of
  the atmosphere, and a hyperspace jump to Barnard's Star (5.42 light years, 1 t of fuel).
- The game reads keys by polling, so a key must be held across a frame. It moves its pointer by
  the mouse driver's motion counters at half a pixel per mickey.
- The program never leaves real mode (no `mov cr0`/`lgdt` anywhere) and uses the PIT, the
  keyboard port and `int 33h` directly ([RUNTIME.md](RUNTIME.md)).

## Open questions

- What are `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` and the `LATIS._0n` files?
- Which start option reads which of `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE`? Watch the `open`
  log on the runtime for each menu choice.
