# frontier-decompile

Research toward a 1:1, cross-platform, browser-capable (WASM) reimplementation of
**Frontier: Elite II** (DOS, 1993) in Rust + wgpu, done in the same way as the
[Banished reimplementation](https://github.com/tanepiper/banished-decompile).

This repository contains **no game code and no game assets**. It holds format documentation, tooling
and, over time, an engine reimplementation. To use it you need your own copy of the game. The tools
copy the original files from your install into local, git-ignored folders.

## Layout

| Path | Contents | In git? |
|---|---|---|
| `docs/` | Reverse-engineering findings and format specs | yes |
| `tools/` | Asset copy script, DOSBox reference runner, Ghidra scripts | yes |
| `crates/`, `apps/` | The Rust reimplementation | yes |
| `game-data/original/` | Read-only copy of your `FRONTIER.EXE`, `EL2*.OVL` and data files | **no** |
| `game-data/run/` | Writable copy that DOSBox runs (saved games land here) | **no** |
| `game-data/derived/` | Captures, emulator dumps, exported images | **no** |
| `analysis/` | Ghidra project and decompiled output | **no** |

## Getting started

Requirements: Rust (stable, with `rustup target add wasm32-unknown-unknown` and `cargo install trunk`
for the web build later), and DOSBox Staging (`brew install dosbox-staging`) to run the original.
For analysis you also need Ghidra 12.1.x and JDK 21 (`brew install ghidra`). The REA MCP server
(the `reverse-engineer-anything` skill) uses the same Ghidra install.

```bash
# 1. Copy your game into game-data/ (read-only original + DOSBox working copy)
FRONTIER_DIR=/path/to/frontier tools/extract-assets.sh

# 2. Run the original game as the reference
tools/dosbox/run.sh
tools/dosbox/reset.sh            # throw away saves/state in game-data/run

# 3. Ghidra analysis of FRONTIER.EXE and every overlay into analysis/export (~20 s)
tools/ghidra/export.sh

# 4. The game (native window; runs your original files on the reference runtime) and tests
cargo run --release -p frontier-app
cargo test --release             # the game-data tests skip if game-data/ is missing

# 5. The game in the browser (WASM; your files are served only to 127.0.0.1)
tools/dev-web.sh                 # http://127.0.0.1:8743/?data=/game-data/
```

| Crate | Purpose |
|---|---|
| `crates/formats` | Dependency-free readers for the shipped files (MZ images so far) |
| `crates/hooks` | Installs native ports at the original routines' addresses; parity tests and shadow verification ([docs/MATH3.md](docs/MATH3.md)) |
| `crates/dos` | Reference runtime: 386 real-mode CPU + DOS/BIOS/EMS/VGA services that run the original as oracle and scaffold ([docs/RUNTIME.md](docs/RUNTIME.md)) |
| `crates/game` | Deterministic game logic, ported 1:1 and proven bit-exact against the original |
| `apps/game` | `frontier`: the game, native and browser |

## Status

See [docs/FINDINGS.md](docs/FINDINGS.md) for what is known about the binaries and the milestone plan.

- [x] M0: reference runtime (DOSBox Staging), repository layout, Ghidra export
- [x] Playable: the original runs on the in-process runtime, native and WASM, from boot through the
  intro and start menu into the cockpit ([docs/RUNTIME.md](docs/RUNTIME.md)); native ports replace it
  routine by routine
- [ ] M1: function discovery across `FRONTIER.EXE` and the overlays; overlay loader traced
  (loader and seeding done, see [docs/OVERLAYS.md](docs/OVERLAYS.md); indirect calls and names remain)
- [ ] M2: data formats (`.GRF` graphics, music, ship and planet models, saved commanders)
- [ ] M3: boot flow and intro rendered by the reimplementation
- [ ] M4: maths core (fixed point, RNG) and galaxy/system generation, bit-exact. 21 `EL2MATH3`
  float routines ported and verified, running natively in the game ([docs/MATH3.md](docs/MATH3.md))
- [ ] M5: flight, orbits and combat
- [ ] M6: stations, trade, missions, save/load

## Legal

Frontier: Elite II is © Frontier Developments / David Braben. This project is an independent,
non-commercial interoperability effort, not affiliated with or endorsed by the rights holders. Do not
commit or distribute game files, extracted assets or decompiled code.
