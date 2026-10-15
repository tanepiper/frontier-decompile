---
name: frontier
description: Entry point for any work on the Frontier: Elite II 1:1 Rust/wgpu/WASM reimplementation in this repo. Use first whenever a task touches Frontier, FRONTIER.EXE or the EL2*.OVL overlays, game-data, crates/, apps/game or docs/ here; it gives the ground rules and routes to the specialised frontier-* skills.
---

# Frontier reimplementation: ground rules and routing

Goal: a **1:1** reimplementation of Frontier: Elite II (DOS, the build recorded in
`game-data/MANIFEST.sha256`) in Rust + wgpu, native and in the browser (WASM), reading the user's own
game files. "1:1" means behaviour, data formats, constants and visuals match the original program —
not "inspired by". Structure and rules mirror the Banished project (`banished-*` skills).

## Non-negotiable rules

1. **Never commit game material.** `game-data/` (original files, the DOSBox run copy, captures,
   emulator dumps) and `analysis/` (Ghidra project, `decomp.c`, disassembly) are git-ignored and
   must stay that way. Never paste decompiled C, disassembly, or file dumps into tracked files.
   Docs describe findings in prose, tables and addresses (`1000:2f3a`, `EL2M07:0x0412`).
2. **`game-data/original/` is read-only.** Analysis and tests read it; DOSBox runs
   `game-data/run/` (recreate with `tools/dosbox/reset.sh`).
3. **Evidence before code.** Every ported routine cites the original address in a doc comment
   (`/// Port of 1000:2f3a (fixed-point multiply): …`). Guesses say *inferred* / *assumed*.
4. **Other sessions may edit this repo concurrently.** Stage only your own files; never
   `git add -A`. See `frontier-land`.
5. **Downloads need user permission** (state filename, source, size).

## Map of the repo

| Path | What |
|---|---|
| `crates/formats` | Dependency-free readers (`mz` so far; `.GRF`, models, saves to come) |
| `crates/game` | Bit-exact game logic (maths, RNG, galaxy, orbits, ships, trade) |
| `apps/game` (bin `frontier`) | The game, native and WASM |
| `tools/extract-assets.sh` | Copies the user's game into `game-data/original` + manifest |
| `tools/dosbox/` | `run.sh`, `reset.sh`, `frontier.conf`: the original as a reference |
| `tools/ghidra/` | `export.sh` (all binaries → `analysis/export/<FILE>/`), export scripts |
| `docs/` | `FINDINGS.md` (overview, milestones, open questions) and topic docs |

Binaries: `FRONTIER.EXE` (main program, 16-bit real-mode MZ using 32-bit registers, Ghidra load
segment `0x1000`), 26 `EL2*.OVL` overlays (each its own MZ image, loaded by the program's own
overlay loader). Ghidra addresses are `segment:offset` in its analysis image.

Setup if missing: `FRONTIER_DIR=… tools/extract-assets.sh`, then `tools/ghidra/export.sh`
(~20 s). Ghidra: `/opt/homebrew/opt/ghidra/libexec`, JDK 21 at
`/opt/homebrew/opt/openjdk@21`. Original game: `tools/dosbox/run.sh` (DOSBox Staging).

## Which skill for which task

| Task | Skill |
|---|---|
| "How does the original do X?" — find and understand a routine, table, constant, string | `frontier-trace` |
| Need exact numbers from an original routine (maths, RNG, generation) | `frontier-emulate` |
| Turn traced behaviour into Rust | `frontier-port` |
| Prove a change works (tests, captures vs DOSBox) | `frontier-verify` |
| Write up findings, commit safely | `frontier-land` |

A feature runs **trace → (emulate) → port → verify → land**. Read `docs/FINDINGS.md` first and grep
`docs/` for the addresses you are about to work on.
