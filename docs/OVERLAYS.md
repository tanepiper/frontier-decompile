# Overlays and the vector tables

This covers how `FRONTIER.EXE` loads the `EL2*.OVL` overlays, where their code ends up, how the main
program calls into them, and the text-mode set-up overlay (`EL2SETUP`) with its sound prompt.
Evidence is static: disassembly of the shipped files (capstone 5, 16-bit mode), Ghidra 12.1.4
analysis after `tools/ghidra/SeedFunctions.java`, and the MZ headers and relocation tables. Nothing
here was run under an emulator or a debugger. The DOSBox boot in `FINDINGS.md` §4 shows the prompt
appearing, but nothing about the loader was observed at runtime.

Addresses: main program `1000:xxxx`, with Ghidra's load segment `0x1000`. The main data segment is
paragraph `0x1615` in the file (set at `1000:071e`), which Ghidra shows as `2615:xxxx`; below it is
written `DS:xxxx`. A second data segment, file paragraph `0x1ea9` (`2ea9:xxxx` in Ghidra), holds the
overlay names and the EXEC parameter block; it is written `1ea9:xxxx`. Overlay addresses are
`EL2xxx:offset`, counted from the start of the overlay's load image (Ghidra `1000:offset`).

## 1. Every overlay is a TSR child program

An overlay is not read into memory by hand. The main program **runs it with DOS EXEC**
(`int 21h`, `AX=4B00h`, load and execute). The overlay's own start-up code makes itself resident and
hands its addresses back through a far pointer passed on its command line.

| Address | Role |
|---|---|
| `1000:d34c` | `load_overlay`: ES:DX = file name. Opens the file through `DS:00c0` (`1000:e29d`, open with retry) and closes it again, builds the EXEC parameter block, runs the overlay, copies its exports into the main vector table and returns the overlay's PSP segment in AX. EXEC failure → fatal error `0xF002` through `DS:002c` |
| `1000:d6be` | `load_module`: same, for the `EL2Mnn` modules (§4). Error 8 (not enough memory) → evicts every unlocked module and retries; other errors → fatal `0xF003` |
| `1000:e29d` | Open file read-only (`3D00h`), retrying through `1000:e29c` until it succeeds (the retry routine is not traced yet) |
| `1ea9:0089` | EXEC parameter block. Environment segment = the paragraph after `1ea9:0097` (an empty environment), command tail far pointer → `1ea9:00bb` |
| `1ea9:00b7` | Far pointer the overlay overwrites with the address of its export block |
| `1ea9:00bb` | Command tail: length 9, a space, 8 letters, CR |
| `1ea9:00c8` | Name buffer for modules (`EL2Mnn.OVL`) |

**Command-line handshake (observed in code).** Before the EXEC the loader writes the far address
`1ea9:00b7` into the command tail as 8 letters, one per nibble, `A` = 0 … `P` = 15, offset first, then
segment, most significant nibble first. The overlay reads them back from its PSP at `0082h`–`0089h`.

**The overlay stub, `EL2xxx:0000`–`005d`.** All 26 overlays start with the same 94-byte stub; only
two bytes change, the resident size. The stub:

1. saves its PSP segment at `EL2xxx:005e`;
2. decodes the far pointer from its command tail and stores `ovlseg:005e` there, which is the
   address of its export block;
3. frees its environment block (`49h`);
4. stays resident with `int 21h, AX=3100h` (keep process), DX = resident paragraphs. The paragraph
   count is relocated, so it covers the whole image.

The overlay's code therefore lives at **its own load segment, PSP + 10h**, wherever DOS put it.
Each overlay's relocations patch two words of the stub and every far pointer of the export block
(or module header), plus whatever its own code needs.
Overlays never call DOS EXEC themselves.

**Export block at `EL2xxx:005e`.**

| Offset | Size | Field |
|---|---|---|
| `005e` | word | PSP segment (filled in by the stub). `load_overlay` returns it, and it is used to free the overlay with `49h` |
| `0060` | word | First main-DS slot to fill |
| `0062` | 4 × n | Far pointers, all relocated. `load_overlay` copies them to `DS:dest`, `DS:dest+4`, … |
| after | dword | `0000:0000` terminator |

Ghidra's default analysis disassembles straight on from the stub into this block. That is why each
overlay used to show one large bogus "entry" function. `SeedFunctions.java` now defines the block as
data.

## 2. The main vector table, `DS:0000`–`0357`

Every `lcall [disp16]` in the main program and the overlays goes through one table of far pointers in
the main data segment. Overlays run with DS = the main data segment (*inferred*: `EL2SETUP` writes
`DS:8920` and other main variables directly). They therefore call main routines the same way.

| Range | Slots | Filled by |
|---|---|---|
| `DS:0000`–`01cf` | 116 | Static in the file: the main program's own routines, in segments `1000` and `1ee8` |
| `DS:01d0`–`026f` | 40 | `EL2MCGA3` video driver |
| `DS:0270`–`032b` | 47 | `EL2MATH3` maths |
| `DS:032c`–`0357` | 11 | The chosen sound driver: `EL2ADLIB`, `EL2ROL` or `EL2SPK`. All three export 11 entries to the same slots. With no sound the slots keep their file value `1000:ee5a`, a lone `retf` |
| `DS:0358`–`035b` | 1 | `EL2SETUP` (`EL2SETUP:0496`, the set-up checks) |

Of the 437 `lcall [disp16]` sites in the main program, 140 use the static slots, 126 the video
driver, 141 the maths overlay, 23 the sound driver and 7 the set-up/module slots (byte-pattern count,
*inferred* to be instructions). `SeedFunctions.java` turns the 134 static-slot calls that analysis
reaches into direct-call overrides, so the decompiler shows `v0144_1000_df72()` and not
`(*pcVar)()`. It also comments each overlay-slot call with the overlay routine that fills the slot.

Slot `DS:002c` (`1000:044c`) is the fatal-error exit, entered by `ljmp` with an error code in AX
(`0xF002`, `0xF003`, `0xF006` above).

## 3. Start-up order

`entry` (`1000:008f`) sets SS:SP and calls `startup_init` (`1000:06f1`):

| Step | Address | What |
|---|---|---|
| 1 | `1000:06f1` | Shrinks the program's memory block (`4Ah`). The largest free size is kept at `DS:09e0`, the PSP at `DS:09de` |
| 2 | `1000:0711` | Ctrl-Break (`23h`) and critical-error (`24h`) vectors → `1000:0432`, `1000:043a` |
| 3 | `1000:0734` | Loads `EL2SETUP.OVL` (`1ea9:001b`) and calls `DS:0358` (§5). It then frees the overlay again (`49h`). If set-up returned carry, the program exits with code 2 (`4C02h`) |
| 4 | `1000:0759` | `1000:cdf9`, `1000:ce47`, `1000:ccbb`, `1000:de08` (not traced) |
| 5 | `1000:076d` | Loads `EL2MATH3.OVL` (`1ea9:0028`). PSP → `DS:0836` |
| 6 | `1000:0773` | Reads the current video mode (`int 10h`, `0Fh`) into `1ea9:0035` |
| 7 | `1000:ee5b` | `load_sound_driver` (§5). PSP → `DS:0838`, then calls sound entry `DS:0334` |
| 8 | `1000:e4c2` | `load_video_driver`: `EL2MCGA3.OVL` (`1ea9:067a`). PSP → `DS:0834`, sets `DS:890e` = 4, calls `1000:d08d` and then video entry `DS:020c` |

Steps 1–8 are DOS set-up that the reimplementation skips. `startup_init` returns to `1000:009c`,
and `entry` then calls `modules_init_all` (`1000:0c13`, §4) at `1000:00a0`. That is where the game
start begins; the hand-off and the state to assume are in [DEVIATIONS.md](DEVIATIONS.md).

## 4. `EL2M00`–`EL2M19`: demand-loaded modules

The numbered overlays are loaded **on first use**, evicted under memory pressure and loaded again
later. The `EL2Mnn` names are not stored anywhere. `load_module` builds them in `1ea9:00c8` from
`EL2M`, two decimal digits of the slot number, and `.OVL`.

| Address | Role |
|---|---|
| `DS:0396` | Module table, 64 slots × 8 bytes: far pointer to the module header, then a dword (`module_word_nn`) |
| `DS:0596` | Lock counts, 1 byte per slot. Callers increment it around a call, and eviction skips locked modules |
| `DS:05d6` | One dword per slot, copied from the header at load |
| `DS:035c` | Stub header used for every unloaded slot. Its 12 handler pointers (`DS:0360`–`037f`) point to the thunks below |
| `1000:d5b4` + 7·k | 12 lazy thunks, one per handler. Each one calls `1000:d608` to save the registers and run `load_module` for slot SI/8. It then jumps to the same handler in the freshly loaded header |
| `1000:0be5` | `modules_reset`: unloads all 64 slots and clears the dwords and locks |
| `1000:0c13` | `modules_init_all`: resets, then calls handler `+04` for slots 0–17 (`DS:0394` = `0x88`). Through the thunks this loads every module |
| `1000:d85f` | `module_unload`: points the slot back at `DS:035c` and frees the module's PSP (`header − 0Ch`) |
| `1000:d7f0` | Evicts every unlocked module. Fatal `0xF006` if nothing could be freed |
| `1000:d816` | Evicts one unlocked module, round-robin from `cs:ccb6` |

**Calling convention (observed).** SI = slot × 8. `les bp,[si+0396h]` loads the header, then
`lcall es:[bp+N]` calls handler N. Most callers wrap this in `inc`/`dec byte [si+0596h]`.

**Module header.** In a module the export list at `005e` is empty (dest 0, terminator at `0062`).
The main program stores `ovlseg:006a` (export block + 0Ch) as the header pointer H.

| Offset | Overlay offset | Field |
|---|---|---|
| H−0Ch | `005e` | PSP segment (the stub's word) |
| H−04h | `0066` | Far pointer to a table of byte pairs at `00a2`, `0FFh`-terminated. When `DS:890e` bit 4 is set, `load_module` overwrites the first byte of each pair with the second. Purpose unknown |
| H+00h | `006a` | Dword copied into `module_word_nn` on the first load only |
| H+04h…H+30h | `006e`–`009a` | 12 far pointers: handlers `+04`, `+08`, … `+30`. Unused ones point at a `retf` |
| H+34h | `009e` | Dword copied into `DS:05d6 + slot·4` |

**Broadcast handlers.** These routines loop over slots 0–17 and call the same handler in each
module. Their roles are *unknown* until the callers are traced.

| Handler | Broadcast from |
|---|---|
| `+04` | `1000:0c48` (`modules_init_all`), `1000:0cc0` |
| `+08` | `1000:05c1` |
| `+0c` | `1000:0611` |
| `+10` | `1000:04b9` |
| `+14` | `1000:0568` |
| `+18` | `1000:0df2` |
| `+1c` | `1000:0e57` |
| `+24` | `1000:06df` |
| `+28` | `1000:e8d9` |
| `+2c` | `1000:0523` |
| `+30` | `1000:093c`, `1000:19a0` |

**Direct module calls** (slot from `mov si,imm`): `EL2M01 +18` at `1000:fca6`, `EL2M03 +08` at
`1000:2480`, `EL2M08 +20` at `1000:0165` and `+14` at `1000:01d4`. `EL2M12 +20` is called at
`1000:1b0c`, `1b76`, `1d0c`, `20f1`, `2315` and `25bf`, and `EL2M15 +20` at `1000:157b` and `2268`.
Handler `+20` looks like each module's main entry (*inferred*). Slots 18 and 19 are outside the
broadcast range, and no `mov si,imm` call reaches them, so how `EL2M18`/`EL2M19` are entered is
open.

**Content (inferred from strings).** `EL2M00` system descriptions, `EL2M02` star names, `EL2M03`
cockpit/flight labels, `EL2M07` commodity names, `EL2M08` title screen ("DAVID BRABEN'S", "GAME!TEK"),
`EL2M09` planet/colony names, `EL2M10` load/save commander, `EL2M14` equipment names, `EL2M15` crime
and police messages, `EL2M17` planet survey text. `EL2M18` is almost all text behind a word offset
table at `00a3` (traffic control and bulletin messages). `EL2M06`, `EL2M08` and `EL2M19` are large but
have very little reachable code, so they are probably mostly data (models or graphics, unknown).

## 5. `EL2SETUP` and the sound prompt

This is DOS set-up that the reimplementation **skips** ([DEVIATIONS.md](DEVIATIONS.md)). It is
documented so that the state it leaves behind is known, not so that it can be ported.

`EL2SETUP` exports one routine, `EL2SETUP:0496` → `DS:0358`. It runs five checks in order and stops
at the first failure, printing a `$` message (`09h`) and returning carry:

| Address | Check | Message on failure |
|---|---|---|
| `EL2SETUP:057c` | DOS version (`30h`) ≥ 2 | "Unable to run Frontier - DOS v2.00 or later required." |
| `EL2SETUP:0591` | CPU class from `EL2SETUP:04dc` (FLAGS-bit tests for 286/386/486 and an FPU probe; result word → `DS:891e`). Needs `DS:891f` ≥ 3, an 80386 | "...requires at least an 80386 processor..." |
| `EL2SETUP:05a5` | `DS:09e0` ≥ `9100h` paragraphs (594,944 bytes) | "...Requires 580K free base memory..." |
| `EL2SETUP:05b7` | Opens `EMMXXXX0`, checks with IOCTL (`4400h`, `4407h`) that it is a ready character device, EMS status (`int 67h`, `40h`), ≥ `30h` free pages (`42h`, 768 KB) and the page frame (`41h`) | "no expanded memory (EMS) driver installed" / "not enough free expanded (EMS) memory" |
| `EL2SETUP:04af` | Sound prompt (below) | — |

**The prompt** (`EL2SETUP:04af`) prints "Please select sound support required:-" and the four
options. It then reads keys with `int 21h AH=07h` (no echo) until it gets `1`–`4`, and records the
choice in the main program's `DS:8920`:

| Key | `DS:8920` | Driver loaded by `load_sound_driver` (`1000:ee5b`) |
|---|---|---|
| `1` Roland LAPC1 | 1 | `EL2ROL.OVL` (`1000:ee37`) |
| `2` AdLib / SoundBlaster | 2 | `EL2ADLIB.OVL` (`1000:ee42`) |
| `3` PC internal speaker | 4 | `EL2SPK.OVL` (`1000:ee4f`) |
| `4` No sound | unchanged (0 in the file) | none. The sound slots stay on the `retf` at `1000:ee5a` |

`load_sound_driver` tests bit 2 first, then bit 1, then bit 4, so the value works as a bit mask.
Nothing sets more than one bit (*inferred*). The prompt appears on every start; no setting is saved.

## 6. Ghidra function seeding

`tools/ghidra/SeedFunctions.java` runs in `export.sh` after auto-analysis and before the export. It:

- FRONTIER.EXE: defines and labels the main vector slots (`vec_xxxx`) and seeds a function at each
  target (`vXXXX_seg_off`). It labels the overlay slots (`MCGA3_v01d0` …) with every routine that
  can fill them, and labels the module table, thunks and loader routines above. It sets call
  overrides on `lcall [vec]` and comments the module-handler calls.
- Overlays: removes the bogus entry function and its listing, defines the export block (or module
  header) as data, and seeds the stub (`ovl_tsr_stub`) and every export (`MATH3_v0270`, `M07_h04`, …).
  It comments `lcall [vec]` sites with the main routine they reach.
- Both: repeatedly creates functions at call targets that have none, and follows `cs:[bx+disp]`
  near tables, until the count stops growing.

| Binary | Functions before | after | Bytes in functions after |
|---|---|---|---|
| `FRONTIER.EXE` | 29 | 342 | 82,156 |
| 26 overlays together | 36 | 741 | 108,594 |
| `EL2MCGA3` / `EL2MATH3` / `EL2ADLIB` | 1 / 1 / 1 | 48 / 54 / 36 | 16,377 / 8,972 / 5,446 |
| `EL2M03` / `EL2M11` / `EL2M12` | 1 / 1 / 1 | 100 / 88 / 100 | 14,469 / 8,541 / 10,777 |

Before seeding, the overlays' "functions" were mostly the stub running on into data.

## Open questions

- Roles of the 12 module handlers (only `+04`, the init broadcast, is clear) and of `module_word_nn`,
  `DS:05d6` and the byte-pair table at `00a2`.
- How `EL2M18` and `EL2M19` are reached, and whether slots 20–63 are ever used.
- Remaining unreached code in `FRONTIER.EXE`: about 82 KB of the 332 KB image is in functions now,
  and part of the rest is data. Calls that go through computed SI (`les bp,[si+0396h]`) and
  register-indirect calls are not resolved.
- Three functions in data segments (`2ea9:03f7`, `2ea9:1165`, `2fce:0134`) are probably false
  positives from analysis. They are not checked.
- What `1000:e29c` does when an open fails (a disk prompt?), and what steps 4 of §3 set up.
