# Deviations from the original

This lists the places where the reimplementation deliberately differs from the DOS original. Everything
not listed here should match the original 1:1. In code, each deviation is marked with a
`// Deviation:` comment that names the original address it replaces.

## Policy: no legacy hardware setup

The reimplementation reproduces the **game**, not the 1993 PC configuration around it. It skips any
step that only exists to set up DOS hardware or memory and goes straight into the game.

| Original behaviour | Reimplementation | Status |
|---|---|---|
| Text-mode prompt "Please select sound support required" (1 Roland LAPC1, 2 AdLib / SoundBlaster, 3 PC speaker, 4 No sound) before the game starts | Not shown. Boot goes straight to the game's own start-up (intro/menu) | Decided; prompt traced: `EL2SETUP:04af` sets `DS:8920`, see [OVERLAYS.md](OVERLAYS.md) §5 |
| Choosing a sound driver overlay (`EL2ROL`, `EL2ADLIB`, `EL2SPK`) | No driver choice. Audio reproduces one reference output (AdLib / SoundBlaster, option 2, which the DOSBox reference config also uses) through the platform's audio API | Decided; reference driver may be revisited when audio is traced |
| Video driver overlay (`EL2MCGA3`) and VGA mode 13h set-up | The 320×200 palettised frame is produced in software and presented with wgpu | Decided |
| DOS memory checks ("Not enough free memory") and overlay loading | Not reproduced. Overlay code is ported like the rest of the game | Decided |

### Where set-up hands off to the game

All of the skipped set-up runs inside `startup_init` (`1000:06f1`), which `entry` calls at
`1000:0099`. When it returns, `entry` continues at **`1000:009c`**, which calls `modules_init_all`
(`1000:0c13`) at `1000:00a0`. That is where the game proper starts, and where the reimplementation's
boot begins. Static evidence only ([OVERLAYS.md](OVERLAYS.md) §3, §5). The hand-off was not observed
at runtime.

State the set-up leaves for the game. The reimplementation supplies it directly, without running the
set-up:

| State | Set by | Value to assume |
|---|---|---|
| `DS:8920` sound selection (bit mask 1 ROL, 2 AdLib, 4 speaker, 0 none) | `EL2SETUP:04af` | 2 (AdLib, the reference) |
| `DS:891e` CPU/FPU class word (`891f` ≥ 3 = 386) | `EL2SETUP:04dc` | *unknown* until a reader is traced; DOSBox reference gives a 386+ with FPU |
| `DS:09de` PSP, `DS:09e0` largest free block (paragraphs) | `1000:06f1` | not needed once DOS memory is gone (*inferred*) |
| `DS:0834`/`0836`/`0838` PSPs of `EL2MCGA3`/`EL2MATH3`/sound driver | `load_overlay` | not needed (driver code is ported in place) |
| `1ea9:0035` BIOS video mode at start | `1000:0773` | only used to restore the mode on exit (*inferred*) |
| `DS:890e` = 4 | `1000:e4c2` | 4. Bit 4 clear, so module byte-pair patch tables are not applied |
| Vector slots `DS:01d0`–`0357` | the driver overlays | the ported driver routines, called directly |

Driver initialisation calls that happen inside set-up and may set game-visible state: sound entry
`DS:0334` (from `1000:ee82`), `1000:d08d` and video entry `DS:020c` (from `1000:e4c2`), and
`1000:cdf9`, `1000:ce47`, `1000:ccbb`, `1000:de08` (from `1000:0759`). None is traced yet. Any game
state they set must be reproduced when they are.

Anything the player sees as part of the game itself stays 1:1, including the intro, menus, the
in-game options screens, gameplay and the save format. To compare boot captures with DOSBox, answer
the sound prompt with **2** so the reference uses the same driver.
