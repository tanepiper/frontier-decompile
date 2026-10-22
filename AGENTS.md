# AGENTS.md: reverse engineering Frontier: Elite II

Rules for every agent (Claude Code, Codex, Cursor and others) and every person working here.
The method and the tools come from [GameFrame](https://github.com/tanepiper/game-frame), an
agentic framework for reverse engineering games. Its skills, its stage and task agents, its gates
and its MCP servers come from GameFrame's Claude Code plugin, enabled in `.claude/settings.json`.
Each agent does one step and hands off to the next (`re-start`, "Stage agents"). The gates stop
a commit or a written file that would add game material or decompiled code. `./gameframe` runs
GameFrame's toolkit on this project (`./gameframe root` prints where GameFrame is). The `frontier-*` skills are this game's own versions of the
steps: use them first, and GameFrame's `re-*` skills for anything they do not cover.

## The goal

A 1:1 reimplementation of **Frontier: Elite II** (DOS, 1993; the build recorded in
`game-data/MANIFEST.sha256`) in Rust + wgpu, native and in the browser (WebAssembly), reading
the player's own game files. "1:1" means that behaviour, data formats, constants and visuals
match the original program. The original runs on an in-process 386 real-mode runtime
(`crates/dos`), and routines are replaced by native ports one overlay at a time, each checked
against the original running beside it.

## Working rules

1. **Keep game material out of git.** `game-data/` (the original files, the DOSBox run copy,
   captures, emulator dumps) and `analysis/` (the Ghidra project, `decomp.c`, disassembly) are
   ignored by git; never force-add past `.gitignore`. Do not paste decompiled code, disassembly
   or file dumps into tracked files: describe findings in prose, tables and addresses
   (`1000:2f3a`, `EL2M07:0412`).
2. **`game-data/original/` is read-only.** Analysis and tests read it. DOSBox runs
   `game-data/run/` (recreate it with `tools/dosbox/reset.sh`).
3. **Evidence before code.** Every ported routine cites the original address in a doc comment
   (`/// Port of `EL2MATH3:1cf5` (...)`, or `1000:2f3a` in `FRONTIER.EXE`), and says how it is known:
   *observed* (read in code or data), *measured* (run), *inferred* or *assumed*.
4. **Measure before claiming exactness.** A ported routine is checked against the original on the
   runtime, register for register and write for write (`frontier-verify`, [MATH3.md](docs/MATH3.md)).
5. **Mark every intentional difference** in a code comment containing `Deviation:` (and a piece
   left out with `not ported`), then regenerate the ledger:
   `./gameframe run deviations --out docs/DEVIATIONS-LEDGER.md crates apps`. The policy for
   differences is [DEVIATIONS.md](docs/DEVIATIONS.md).
6. **Game, not hardware.** Legacy DOS set-up (sound card selection, driver overlays, memory checks)
   is not reproduced; the rewrite goes straight into the game.
7. **Write it down.** Findings go into `docs/`, function names into `labels/`, progress into
   [docs/CHECKLIST.md](docs/CHECKLIST.md). An undocumented finding is re-derived by the next session.
8. **Several sessions may work at once.** Stage only your own files (`git add path`), never
   `git add -A`. Never revert work you did not do. Commit and push to `origin main` after each
   coherent, verified piece of work.
9. **Ask before downloading** anything; say the file, the source and the size.

## Where things are

| Path | What |
|---|---|
| `game-data/original/` | Your copy of `FRONTIER.EXE`, the `EL2*.OVL` overlays and the data files (ignored) |
| `game-data/run/`, `game-data/derived/` | The DOSBox run copy; captures and emulator dumps (ignored) |
| `analysis/export/<binary>/` | Ghidra's text export of every binary (ignored): `tools/ghidra/export.sh` |
| `crates/dos` | The 386 real-mode runtime with the DOS, BIOS, EMS and mouse services the program uses ([RUNTIME.md](docs/RUNTIME.md)) |
| `crates/game`, `crates/hooks`, `crates/opl`, `crates/formats` | Native ports of the original's routines, the hooks that run them in place of the original, the OPL2 model, the file readers |
| `apps/game` (bin `frontier`) | The game, native and in the browser |
| `docs/FINDINGS.md` | The overview: the build, the load image, the overlays, what was observed |
| `docs/CHECKLIST.md` | What is traced, ported and verified, and what is next |
| `docs/DEVIATIONS.md`, `docs/DEVIATIONS-LEDGER.md` | The policy for differences, and the generated ledger of every marked place |
| `labels/` | Names of original functions, one file per binary because every overlay is numbered from `1000:0000`: `labels/FRONTIER.tsv`, `labels/EL2MATH3.tsv` and so on, generated from the port's citations by `tools/triage/cite_labels.py`; `labels/names.tsv` and `labels/fields.tsv` by hand |
| `tools/extract-assets.sh`, `tools/dosbox/`, `tools/ghidra/`, `tools/coverage/` | This project's own scripts: copy the game, run the original in DOSBox, export from Ghidra, record and compare code coverage ([COVERAGE.md](docs/COVERAGE.md)) |
| `./gameframe` | Runs GameFrame's toolkit here: `run NAME ARGS`, `python FILE`, `doctor`, and help (`./gameframe help`). GameFrame itself comes from its plugin |
| GameFrame's guides | The reference documents the skills name (`docs/DOS.md` first, for 16-bit DOS programs), in GameFrame's `docs/` and on https://tanepiper.github.io/game-frame/ |
| `docs/games/`, `docs/observe/`, `docs/working/` and the other guides | GameFrame's guides: the family of games (vehicle and space simulations), observing the original, working on a project |

Binaries: `FRONTIER.EXE` (the main program, 16-bit real-mode MZ using 32-bit registers, Ghidra
load segment `0x1000`) and 26 `EL2*.OVL` overlays, each its own MZ image loaded by the program's
own overlay loader ([OVERLAYS.md](docs/OVERLAYS.md)).

## The loop

inventory → observe → decompile → trace → (format) → emulate → reimplement → verify → land. Start
each session by reading `docs/CHECKLIST.md` and searching `docs/` and `labels/` for the addresses
you are about to work on. `./gameframe find WORD` shows which skills and tools cover a subject.

| Task | Skill |
|---|---|
| How does the original do X? Find and understand a routine, table, constant or string | `frontier-trace` |
| Exact numbers from an original routine (maths, random numbers, generation) | `frontier-emulate` |
| Turn traced behaviour into Rust | `frontier-port` |
| Prove a change works (tests, captures against DOSBox) | `frontier-verify` |
| Write up findings and commit safely | `frontier-land` |

After porting, regenerate the labels so the decompile tools name the new routines:

```bash
for m in EL2MATH3 EL2MCGA3 EL2ADLIB EL2SETUP; do
  ./gameframe run cite_labels . --module $m --functions analysis/export/$m.OVL/functions.tsv --no-lines --out labels/$m.tsv
done
./gameframe run cite_labels . --any --functions analysis/export/FRONTIER.EXE/functions.tsv --no-lines --out labels/FRONTIER.tsv
```
