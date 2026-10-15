# Reference runtime (`crates/dos`)

This covers the in-process runtime that runs the user's own `FRONTIER.EXE` and `EL2*.OVL` files,
what it implements, and what was observed when the original ran on it. The evidence is runtime
observation on this runtime: boot logs from `crates/dos/examples/boot.rs`, frames captured to
`game-data/derived/captures/`, and the native app's window. These were compared by eye with the
original running in DOSBox Staging 0.83.0 (`tools/dosbox/frontier.conf`, sound option 2 fed
through stdin). No frame was compared pixel by pixel yet, and nothing was run past the
start-position menu.

## Why a runtime

The goal is a 1:1 native port. The runtime is how the project gets there while staying 1:1:

1. **Oracle.** Any original routine can be called with chosen inputs, and its registers and memory
   compared with the native port. This works in `cargo test`, natively and in the browser. It does
   the job the `frontier-emulate` skill planned for Unicorn, without Python.
2. **Scaffold.** The playable `frontier` binary (native and WASM) runs on the original code today.
   Native ports replace original routines address by address. A port is hooked in at the
   original's entry, and the runtime keeps everything not yet ported working. When nothing of
   the original runs any more, the runtime is only the oracle.
3. **Tracing.** Runtime facts (call order, which overlay is loaded when, memory contents) can be
   observed directly. Static analysis cannot provide them.

Nothing of the game ships with it. Every byte it executes is read from the user's files at run
time (`game-data/` natively, the file picker in the browser).

## What it implements

| Area | Implementation | Source |
|---|---|---|
| CPU | 80386 real-mode interpreter: `66h`/`67h` prefixes, 32-bit registers, `FS`/`GS`, SIB addressing, `0Fh` opcodes (`movzx/sx`, `shld/shrd`, `bt*`, `bsf/bsr`, `setcc`, long `jcc`, `lfs/lgs/lss`, `imul r,rm`). No protected mode (the byte scan below finds none). No x87 (the FPU opcodes decode and do nothing) | `cpu.rs` |
| Memory | 1 MB, wrapping at 1 MB (A20 off), in 16 KB pages. 4 MB of EMS pages that map into the frame at `E000:0000` | `mem.rs` |
| BIOS stubs | Every IVT vector points at `F000:n·8`, which runs `0F FF nn` (host service *nn*) and then `IRET`, so programs can hook vectors and chain to them as on hardware | `machine.rs` |
| DOS `21h` | Files on an in-memory, case-insensitive directory (`vfs.rs`), memory blocks (first fit, `48h`/`49h`/`4Ah`), `EXEC 4B00h` with a real PSP and command tail, TSR `31h` back to the parent, find first/next, vectors, version 5.0 | `machine.rs` |
| EMS `67h` | LIM 4.0 core: `40h`–`4Eh`, `50h`, `51h`, `58h`. The driver's segment carries `EMMXXXX0` at offset `0Ah` | `machine.rs` |
| Mouse `33h` | Reset, position, ranges, buttons, mickeys. The event handler (`0Ch`) is recorded but not yet called | `machine.rs` |
| Hardware | 8259 PIC (mask, EOI), 8253 PIT (reload, latch, modes) driving IRQ 0, keyboard controller (port `60h`/`64h`, IRQ 1, set-1 scan codes), VGA DAC (`3C7h`–`3C9h`), sequencer, CRTC and graphics registers (stored), `3DAh` retrace at 70 Hz | `devices.rs` |
| Time | Virtual: 25 M instructions = 1 s. The app runs `ips × frame time` instructions per host frame | `devices.rs` |

### Static facts behind the design

A byte scan of the shipped files (pattern counts, *inferred* to be instructions):

| File | `mov cr0` | `lgdt` | `fninit` | `int 67h` | `int 33h` | `int 10h` | `out 43h` | `in al,60h` | `int 21h` |
|---|---|---|---|---|---|---|---|---|---|
| `FRONTIER.EXE` | 0 | 0 | 1 | 9 | 9 | 4 | 3 | 1 | 53 |
| `EL2MCGA3.OVL` | 0 | 0 | 0 | 4 | 0 | 1 | 0 | 0 | 3 |
| `EL2MATH3.OVL` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 2 |

The program stays in real mode (this answers the open question in `FINDINGS.md`). It uses EMS
through `int 67h` from the main program and the video driver. It programs the PIT and reads the
keyboard port itself, and it uses the `int 33h` mouse driver.

## Observed boot on the runtime

The runtime's event log, virtual time in seconds. The set-up overlay's checks are replaced
([DEVIATIONS.md](DEVIATIONS.md)). Everything else is the original program running.

| t (s) | Event | Notes |
|---|---|---|
| 0.01 | `open`/`exec EL2SETUP.OVL` | Through `load_overlay` (`1000:d34c`): open, close, then `EXEC`. The overlay's TSR return hands its export block back, as described in [OVERLAYS.md](OVERLAYS.md) §1 |
| 0.01 | EMS: allocate 48 pages (768 KB) as handle 1 | Exactly the `30h` pages that `EL2SETUP:05b7` requires |
| 0.01 | `exec EL2MATH3.OVL`, then `EL2MCGA3.OVL` | Same order as `startup_init` steps 5 and 8. Both get the same PSP `5344` that `EL2SETUP` freed |
| 0.01 | `int 10h` mode 13h | Set by the video driver |
| 0.02 | `open EL2G.GRF` | Graphics file read at start-up |
| 0.09 | `int 21h AH=1Ch` (drive allocation info) | Before the module loads. Caller not traced |
| 0.10 | `exec EL2M00` … `EL2M17` | `modules_init_all` (`1000:0c13`) loading slots 0–17 through the lazy thunks, as traced statically |
| 0.10 | `exec EL2M08.OVL` again, at PSP `6b9b` | `EL2M08` (the title module, *inferred* from strings) was evicted under memory pressure and reloaded into the block `EL2M00` had. **Observed:** eviction does happen at start-up with 640 KB conventional memory |
| ~1–20 | Intro: "GAMETEK + KONAMI" card, "DAVID BRABEN'S", planets, the 3D "FRONTIER" title | Same sequence and colours as DOSBox, compared by eye (navy background, flat-shaded blue planets) |
| ~20 | **Select Start Position** menu: 1 Ross 154 (recommended), 2 Mars, 3 Lave, 4 Last Saved Position, 5 Select Position from disc. "Copyright © D.J.Braben 1993" and the Frontier logo in the bottom band | A key press (space at 20 s) loads `EL2M10` (load/save commander, *inferred*). This is when the menu appears. Whether the key or the intro's end brought up the menu is not separated yet |

Host speed: about 95 M instructions/s natively (Apple Silicon, release build), roughly 4× the
virtual 25 MIPS PC.

## Using it

```bash
# Headless: run N virtual seconds, frame every second to game-data/derived/captures/boot-NN.png,
# optional key presses T:SC (seconds:set-1 scancode hex)
cargo run --release -p frontier-dos --example boot -- game-data/original 40 20:39

# The game, native (reads game-data/run, else game-data/original; saves go to game-data/run)
cargo run --release -p frontier-app

# The game in the browser (serves your local game-data only to 127.0.0.1)
tools/dev-web.sh        # http://127.0.0.1:8743/?data=/game-data/
```

## Open questions

- The mouse event handler (`int 33h AX=000Ch`): does the game install one? Not seen in the logs
  up to the menu.
- Sound: the selection is 0 (no driver) for now, so the sound vectors keep their `retf`.
  Reproducing the AdLib output needs `EL2ADLIB` running against an OPL2 model.
- Timing: the virtual PC speed (25 MIPS) is a guess. Compare intro timing with DOSBox at
  matched cycles.
- Pixel-exact comparison with DOSBox frames (raw 320×200 captures, Ctrl+F5) is not done yet.
