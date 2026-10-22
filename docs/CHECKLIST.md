# Checklist: Frontier: Elite II

Updated: 2026-10-22

Each line names an item and its main original routine. Its state is traced (read and
documented), measured (run, ground truth captured), ported, or verified (identical to the
original on the runtime). Keep it short. Details belong in the topic documents.

## Inventory

- [x] Every binary identified: `FRONTIER.EXE` and 26 `EL2*.OVL` overlays ([FINDINGS.md](FINDINGS.md))
- [x] Ghidra export of every binary (`tools/ghidra/export.sh`), with functions seeded from the vector tables ([OVERLAYS.md](OVERLAYS.md))
- [x] The overlay loader and the start-up order traced ([OVERLAYS.md](OVERLAYS.md))
- [x] Code coverage on the runtime: 39 scripted runs (intro, start, every cockpit and station screen, save, load, launch, flight, hyperspace) compared with `coverdiff`; Ghidra seeded from them, `FRONTIER.EXE` 342 → 409 functions, about 95 % of its 90 KB of code ([COVERAGE.md](COVERAGE.md))

## Foundations

- [x] The original runs on an in-process 386 real-mode runtime, native and in the browser ([RUNTIME.md](RUNTIME.md))
- [x] Interrupts per instruction, and ported routines charged their instruction counts
- [x] `EL2MATH3`: every routine ported and verified, floats, 3D transforms, orbits and positions ([MATH3.md](MATH3.md))
- [x] `EL2ADLIB`: the AdLib sound driver ported, identical to the original, with an OPL2 model ([AUDIO.md](AUDIO.md))
- [ ] `EL2MCGA3`: the span filler, flat triangles, quadrilaterals and lines ported; the rest of the renderer
- [x] The random number generators: `random(n)` `1000:069b`, `random32` `1000:0645`, `mix` `1000:066a`, ported and verified ([RNG.md](RNG.md))
- [x] Galaxy: the density map (`EL2M19`), sector density `EL2M02:3b34`, sector seed `EL2M02:3abe` and sector star list `EL2M02:3c2f`, ported and verified ([GALAXY.md](GALAXY.md))
- [ ] Star systems: from a star in the list to its system, type, name and planets ([GALAXY.md](GALAXY.md); leads: the routines only a hyperspace jump runs, in `EL2M00` and `FRONTIER.EXE`, [COVERAGE.md](COVERAGE.md))
- [ ] The data files: `EL2G.GRF`, the models, saves

## Systems

- [ ] `EL2M00`–`EL2M19`, the demand-loaded modules ([OVERLAYS.md](OVERLAYS.md), section 4)
- [ ] The main program's game logic in `FRONTIER.EXE`
- [ ] Coverage runs still to add: combat, docking, buying and selling, missions, the system map
- [ ] `EL2ROL`: MT-32 music from the `.ROL` files ([AUDIO.md](AUDIO.md), section 4)

## Labels and evidence

- [x] Deviations: the policy ([DEVIATIONS.md](DEVIATIONS.md)) and the generated ledger ([DEVIATIONS-LEDGER.md](DEVIATIONS-LEDGER.md))
- [x] Labels from the port's citations, one file per binary in `labels/` (91 labels)
- [x] Hand labels for the routines only one action runs (`labels/names.tsv`, `labels/names-EL2M00.tsv`, `labels/names-EL2M10.tsv`), roles inferred
- [ ] The open questions in [FINDINGS.md](FINDINGS.md): the `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` and `LATIS._0n` files
