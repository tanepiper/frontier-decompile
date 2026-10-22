# Code coverage: which code each action runs

This covers how the project finds the code behind one action in the game. The original runs on
the runtime ([RUNTIME.md](RUNTIME.md)) with a coverage recorder on. A script drives it through a
fixed timeline of keys and mouse clicks. Two runs that differ only in one action are compared with
GameFrame's `coverdiff`. Evidence: 39 runs of the user's `FRONTIER.EXE` and overlays on the
runtime (*measured*), the Ghidra export before and after seeding from them, and the game's manual
for the controls. Roles of the routines found are *inferred* from the action that ran them, and
none is traced yet.

## Results in brief

- **`FRONTIER.EXE` has about 90 KB of code, not 332 KB.** Every instruction the runs executed
  lies between `1000:008f` and `2614:000d` (flat `0x1008f`–`0x2614d`). The main data segment
  starts right after, at `2615:0000` (`mov ax,1615h` relocated, [OVERLAYS.md](OVERLAYS.md) §2), and
  runs to the end of the 332,492-byte image. The 250 KB that no function covered was mostly that
  data segment (*inferred*: no run executed anything there, and the program sets DS to it).
- **Known code in `FRONTIER.EXE` went from 342 functions (82,156 bytes) to 409 (86,189 bytes).**
  The code region from `1000:0000` to `2615:0000` is 90,448 bytes, so functions now cover about
  95 % of it. Across the 26 overlays the count went from 741 to 765 functions (108,594 to
  111,961 bytes).
- Of the 21,552 distinct instructions that ran in `FRONTIER.EXE`, 20,599 now lie inside a
  function. The other 953 are disassembled but sit in code Ghidra did not attach to a function
  body. In `EL2MCGA3` this is 1,690 of 7,702 instructions.
- 331 of the 409 main-program functions ran in at least one run. The 78 that never ran are
  candidates for code these runs did not reach (combat, docking, trading, missions) or for
  data seeded as code (`1000:0003` and `1000:0009` never ran and sit before the entry point).
- `EL2M00`, `EL2M02`, `EL2M07`, `EL2M09` and `EL2M11` run on a hyperspace jump, and most of
  `EL2M00`'s new routines (`EL2M00:4990`–`4e3f`) run only then. They are the first place to look
  for system generation (*inferred*).

## Method

### The recorder (`crates/dos/src/coverage.rs`)

`Machine::coverage_start()` gives the CPU two bitmaps of 1 Mbit, one bit per linear address.
Before each instruction the CPU sets the bit of its address. It sets the bit in the second map
when the instruction was reached by a call (`E8`, `9A`, `FF /2`, `FF /3`), a far jump (`EA`,
`FF /5`) or an interrupt, which marks routine entries. With recording off, the CPU's `cover`
field is `None` and the check is one branch per instruction. Over 60 s of virtual time the
`boot` example took 20.9–21.2 s of host time against 20.5–20.7 s without the recorder
(*measured*, two runs each), so about 1–3 %.

The bitmaps hold linear addresses, but overlays move. `EL2M08` is evicted and reloaded during
start-up, and module blocks are reused all game long ([OVERLAYS.md](OVERLAYS.md) §4). So
`Machine` moves the bits into per-image sets just before an image is loaded (`EXEC`) or its
block freed, while every resident image is still where its code ran. A bit maps to the image
whose block holds it, as an offset from the image's load segment (PSP + 10h). That offset plus
`0x10000` is Ghidra's flat address in that file, since every image is imported at segment
`1000`. Bits outside every image (the runtime's BIOS stubs at `F000`) go to `linear`.
`coverage_take()` flushes and returns the sets. `crates/dos/tests/coverage.rs` checks that a
recording run is identical to a plain one (registers, memory, log) and that every module's stub
is filed under its own name.

### The runner (`crates/dos/examples/coverage.rs`)

It boots the program, applies events at virtual times and writes, per image, `IMAGE.cov` (every
executed address, one hex flat address per line, the text format `coverdiff` reads) and
`IMAGE.entries` (routine entries). Input is applied at fixed 10 ms slices of virtual time and no
native hooks are installed, so a run is the original code only and the same events give the same
run. Recording two `idle` runs gave byte-identical logs and final frames (*measured*). An event
`T:cover` starts recording at T, so a run and its baseline record the same window.

What the controls needed, *measured* on the runtime:

- **Keys are polled, not queued.** A press and release in the same slice is never seen. The
  arrow keys on the galactic map need the key held for 0.08 s, which scrolls about 26 pixels.
  Function keys and Enter work as taps.
- **The pointer moves by mickeys.** The game reads the driver's motion counters
  (`int 33h AX=000Bh`) and moves its pointer half a pixel per mickey. The driver position
  (`AX=0003h`, range 0–319 × 0–198 as set by the game) is not what moves it. A click therefore
  pushes the pointer to the top-left corner, then moves it by twice the target, then holds the
  button for 0.1 s.
- The manual (Appendix Three, "An introductory journey") gives the rest. A star becomes the
  hyperspace target when the arrow keys scroll it to the middle of the galactic map. F8 in the
  view mode starts the jump. Launch needs F4, Launch Request, then F7. The right mouse button and
  backward motion pitch the nose up, and Enter raises the set speed.

### The runs (`tools/coverage/scenarios.tsv`)

`tools/coverage/run.sh` records every run in parallel (39 runs in about 75 s on 12 cores) into
`game-data/derived/coverage/` (git-ignored). The common prefix is space at 20 s (leave the
intro) and `1` at 24 s (start at Ross 154). The cockpit is up, landed at Sirocco starport on
Merlin, from about 32 s. Cockpit runs record from 33 s to 41 s and flight runs from 33 s to 80 s.

`tools/coverage/report.py` runs `coverdiff` for each run against its baseline and each image.
It then sorts the instructions only the action ran into the export's functions by start and
size, and writes `report.md` there. It also writes the seed lists below.

### Seeding Ghidra

`tools/ghidra/SeedFunctions.java` reads `game-data/derived/coverage/seeds/IMAGE.cov` and
`.entries` when they exist. It disassembles every executed address that was not yet an
instruction, clearing data that analysis had put there, since executed bytes are code. It then
makes a function at every entry. In `FRONTIER.EXE` that was 121 disassembly starts and 68 new
functions. `tools/ghidra/export.sh` takes `GAME_DATA` and `ANALYSIS` so a worktree can use the
main checkout's copies.

## What each action ran

Instructions that ran only with the action, against its baseline. The main-program column also
gives the number of functions they fall in. The module and driver columns give instruction
counts per overlay (`M03` is `EL2M03`). Every screen change runs `EL2M10` handler `+10`
(`M10_h10`, 16 instructions) and a few `EL2M03` handlers. They are probably reached through one
of the broadcast handlers that call the same handler in every module ([OVERLAYS.md](OVERLAYS.md)
§4), so they say little about the action.

| Run | Baseline | `FRONTIER.EXE` | Modules | Drivers |
|---|---|---|---|---|
| `intro` | `boot` | 2,261 (40) | M08 40 | ADLIB 256, MATH3 58, MCGA3 3,180 |
| `start_game` | `menu` | 8,996 (139) | M00 457, M01 147, M02 616, M03 909, M04 210, M05 47, M06 149, M07 748, M08 32, M09 112, M10 43, M11 675, M12 38, M13 32, M14 41, M15 32, M16 161, M17 65, M19 34 | ADLIB 224, MATH3 1,306, MCGA3 143 |
| `f1_view` (rear view) | `idle` | 1,670 (40) | M03 85, M04 35, M10 16 | ADLIB 26, MATH3 184, MCGA3 825 |
| `f2_galaxy` | `idle` | 1,289 (48) | M02 907, M03 8, M04 61, M06 60, M10 16 | ADLIB 26, MATH3 223, MCGA3 344 |
| `f2_select` (Barnard's Star) | `f2_galaxy` | 574 (12) | M02 76, M03 24, M04 288, M16 83 | MCGA3 69 |
| `f2_f5` | `f2_galaxy` | 74 (4) | M02 7, M03 104, M04 12 | – |
| `f2_f6` (system data) | `f2_galaxy` | 98 (8) | M00 177, M02 172, M03 22, M04 3 | MATH3 55, MCGA3 852 |
| `f2_f7`, `f2_f8`, `f2_f9` | `f2_galaxy` | 2 (1) | M02 14–31, M03 22 | – |
| `f2_f10` (whole galaxy) | `f2_galaxy` | 18 (3) | M02 335, M03 22, M04 3, M19 34 | MCGA3 919 |
| `f3_equipment` | `idle` | 1,567 (54) | M03 8, M04 35, M10 16, M12 6, M14 284, M16 80, M18 37 | ADLIB 26, MATH3 136, MCGA3 1,495 |
| `f3_profile` | `f3_equipment` | 12 (3) | M14 171 | – |
| `f3_cargo` | `f3_profile` | 19 (3) | M07 39, M14 88 | MCGA3 1 |
| `f4_comms` (station services) | `idle` | 451 (31) | M03 8, M04 64, M10 16, M12 328, M13 226, M18 37 | ADLIB 26, MCGA3 266 |
| `f4_launch_request` | `f4_comms` | 4,326 (69) | M01 12, M03 294, M04 287, M07 52, M12 189 | MATH3 256, MCGA3 101 |
| `f4_shipyard` | `f4_comms` | 64 (5) | M03 24, M04 2, M12 69, M13 87 | – |
| `f4_bbs` (bulletin board) | `f4_comms` | 104 (9) | M02 79, M03 24, M04 2, M07 46, M09 132, M12 116, M13 87 | – |
| `f4_market` (stockmarket) | `f4_comms` | 96 (11) | M03 24, M04 2, M07 46, M12 156, M13 91 | – |
| `f5` (weapons panel) | `idle` | 192 (12) | M03 104, M04 28, M10 16 | MCGA3 71 |
| `f6`, `f8`, `f9` | `idle` | 60 (4) | M03 31–35, M10 16, M16 6 (F8) | ADLIB 26 (F9) |
| `f7_takeoff` (no clearance) | `idle` | 3,648 (70) | M03 340, M04 354, M10 22, M11 37, M12 162, M15 90, M18 37 | ADLIB 277, MATH3 12, MCGA3 1,076 |
| `f10` | `idle` | 119 (10) | M03 93, M04 35, M10 16 | ADLIB 26, MCGA3 71 |
| `esc_pause` | `idle` | 72 (4) | M03 2, M10 31 | ADLIB 26, MCGA3 71 |
| `preferences` | `esc_pause` | 194 (14) | M03 6, M04 35, M10 77 | MCGA3 1 |
| `save` | `save_screen` | 30 (2) | M03 2, M10 285 | – |
| `load_list` | `save` | 0 | M10 14 | – |
| `load` | `load_list` | 448 (25) | M01 15, M03 125, M04 351, M10 196, M11 43 | ADLIB 229, MATH3 56, MCGA3 81 |
| `fly_launch` | `fly_idle` | 3,903 (93) | M01 12, M03 381, M04 357, M07 52, M10 16, M11 37, M12 517, M13 226, M18 37 | ADLIB 277, MATH3 14, MCGA3 1,249 |
| `fly_climb` | `fly_launch` | 732 (36) | M01 10, M03 180, M07 12 | ADLIB 9, MATH3 178, MCGA3 405 |
| `fly_target` | `fly_climb` | 560 (22) | M02 1,001, M03 21, M04 1, M06 60, M16 83 | MATH3 220, MCGA3 197 |
| `hyperspace` | `fly_target` | 2,210 (41) | M00 1,000, M01 18, M02 314, M03 399, M04 37, M06 87, M07 784, M09 172, M10 28, M11 924, M12 17, M14 32, M15 7, M16 174, M19 34 | ADLIB 167, MATH3 569, MCGA3 74 |

## What the modules do, from the actions

*Inferred* from which runs bring in a module's code, together with the strings
([OVERLAYS.md](OVERLAYS.md) §4).

| Module | Runs that execute it | Role |
|---|---|---|
| `EL2M00` | `start_game`, `f2_f6`, `hyperspace` (1,000) | System descriptions. Runs when a system is entered or shown: system generation or description |
| `EL2M01` | `start_game`, launch, `load`, `hyperspace` (small) | Small, called on state changes |
| `EL2M02` | every galactic-map run, `f4_bbs`, `hyperspace` | Galactic map and star names |
| `EL2M03` | almost every run | Cockpit and console: icons, labels, flight state |
| `EL2M04` | icon bars, launch, `load`, `f2_select` | Icon bar and screen changes (*inferred* from it following every screen change) |
| `EL2M05` | `start_game` only | Start-up only in these runs |
| `EL2M06` | `start_game`, galactic map, `hyperspace` | With the galactic map |
| `EL2M07` | `start_game`, `f3_cargo`, market, BBS, launch, `hyperspace` (784) | Commodities and the market, rebuilt on arrival in a system |
| `EL2M08` | `intro`, `start_game` | Title and intro |
| `EL2M09` | `start_game`, `f4_bbs`, `hyperspace` | Planet and colony names, used by the bulletin board |
| `EL2M10` | Esc twice, save, load | The preferences screen and commander save/load ("Save Current Commander as", "Select Commander File to Load") |
| `EL2M11` | `start_game`, take-off, `load`, `hyperspace` (924) | Runs when the player's situation changes; content not known |
| `EL2M12` | station services, take-off, launch | Station services and their sub-screens |
| `EL2M13` | station services | The station screens' common part (portrait, cargo and cash box) |
| `EL2M14` | the three F3 pages | Inventory: equipment, commander profile, cargo |
| `EL2M15` | `f7_takeoff` only | Crime and police messages: the fine for taking off without clearance |
| `EL2M16` | `f2_select`, `f3_equipment`, `hyperspace` | Target and range ("Fuel Required", "Out of Range" appear when it runs) |
| `EL2M17` | `start_game` only | Planet survey text |
| `EL2M18` | `f3_equipment`, `f4_comms`, take-off, launch | Message text behind its offset table |
| `EL2M19` | `start_game`, `f2_f10`, `hyperspace` | With the whole-galaxy map |

## Routines that only one action ran

A routine that ran in exactly one diff is the best lead for that action. The full lists, with
every routine and its instruction count, are in `game-data/derived/coverage/report.md` after
`report.py`. The leads below are named in `labels/names.tsv` (main program) and
`labels/names-EL2M00.tsv`, `labels/names-EL2M10.tsv` (modules):

| Action | Where | Routines |
|---|---|---|
| `hyperspace` | `FRONTIER.EXE` | `1000:0865` (slot `00bc`), `1ee8:0815` (slot `01b4`), `1ee8:2148` (slot `0184`, 208 instructions), `1ee8:2d25` (467), `1ee8:4ede` (179) |
| `hyperspace` | `EL2M00` | `468a`, `47d8`, `4990`, `4abb`, `4b69`, `4bda`, `4c32`, `4d10`, `4db8`, `4df5`, `4e29`, `4e3f`. Six of them (`4990`–`4d10`) were not functions before the coverage seeds |
| `hyperspace` | `EL2M11` | 17 routines, from `M11_h0c` (`069b`) on |
| `start_game` | `FRONTIER.EXE` | 37 routines, among them `1000:2f67`, `1000:c99b`, `1000:cc3f`, `1ee8:0007` (slot `0198`), `1ee8:4219` (slot `0130`), and the module calls `modules_reset` (`1000:0be5`), `modules_evict_unlocked` (`1000:d7f0`), `module_unload` (`1000:d85f`) |
| `intro` | `FRONTIER.EXE` | `1000:29b3`, `2a3c`, `2d0d`–`2e32`, `6dfa`, `b893`, and cases 1 and 3 of the switch at `1000:b275` |
| `save` | `EL2M10` | `0d8a`, `157b`, `1680`, `1751`, `18f3`, `1a33`, `1a56`, `1a87`, `1b92` |
| `load` | `EL2M10` | `M10_h24` (`0ded`), `123b`, `124f`, `1859`, `1af8`, `1b1a`, `1bc4`; and `1000:2422` (slot `0044`) in the main program |
| `f4_launch_request` | `FRONTIER.EXE` | `1000:64d3`, `1000:b286`, case 8 of the switch at `1000:8012` |
| `f3_equipment` | `EL2M14` | 10 routines |
| `f4_market` | `FRONTIER.EXE` | `1000:eb26` |
| `f4_bbs` | `FRONTIER.EXE` | `1000:eb90` |

The switch at `1000:b275` has cases that only the intro (1, 3), `start_game` (13h), `f1_view`
(10h) and `fly_climb` (0Eh) run. It looks like the main loop's dispatch on the game state
(*inferred*).

## Limits

- **Nothing seen is traced yet.** A routine that only ran with an action is a lead, not a role.
- **The runs do not cover combat, docking, trading a unit, missions, other ships, the system
  map's zoom keys or the F6–F10 cockpit icons in space.** F6, F8 (landed), F9 and F10 changed
  nothing visible in the cockpit.
- **Coverage of the drivers is not separated from the action's rendering.** The `EL2MCGA3`
  counts mostly follow how much the screen changes.
- Code reached only by a jump and never called stays outside any function (953 instructions in
  `FRONTIER.EXE`). Seeding functions there needs a look at each place.
- `EL2M17` has one observed entry that Ghidra did not turn into a function. It was not checked.

## GameFrame gaps found

- `coverdiff` skips `functions.tsv` rows whose address contains `:`, which is every row of a
  16-bit export (`1000:0003`). `report.py` writes a copy with flat addresses.
- `coverdiff` credits an address to the nearest function start before it, whatever the
  function's size, so code past a function's end looks known. `report.py` checks the size.
- GameFrame's `coverage` tool reads `EL2MCGA3:1cf5`-style citations as nothing, so it reports
  `EL2MCGA3` as 0 % ported. Every overlay is numbered from `1000:0000`, and a per-overlay
  citation needs a module-aware lookup.

## Reproducing

```bash
tools/coverage/run.sh                          # all runs, about 75 s on 12 cores
./gameframe python tools/coverage/report.py    # report.md and the seed lists
tools/ghidra/export.sh                         # re-export with the coverage seeds
```

One run by hand, for exploring a screen. `T:shot` saves the frame:

```bash
cargo run --release -p frontier-dos --example coverage -- --name try --secs 40 \
  20:39 24:02 33:3c 34:48+ 34.08:48- 36:shot
```
