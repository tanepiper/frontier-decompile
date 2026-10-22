# Checklist: Frontier: Elite II

Updated: 2026-10-22

Each line names an item and its main original routine. Its state is traced (read and
documented), measured (run, ground truth captured), ported, or verified (identical to the
original on the runtime). Keep it short. Details belong in the topic documents.

## Inventory

- [x] Every binary identified: `FRONTIER.EXE` and 26 `EL2*.OVL` overlays ([FINDINGS.md](FINDINGS.md))
- [x] Ghidra export of every binary (`tools/ghidra/export.sh`), with functions seeded from the vector tables ([OVERLAYS.md](OVERLAYS.md))
- [x] The overlay loader and the start-up order traced ([OVERLAYS.md](OVERLAYS.md))

## Foundations

- [x] The original runs on an in-process 386 real-mode runtime, native and in the browser ([RUNTIME.md](RUNTIME.md))
- [x] Interrupts per instruction, and ported routines charged their instruction counts
- [x] `EL2MATH3`: every routine ported and verified, floats, 3D transforms, orbits and positions ([MATH3.md](MATH3.md))
- [x] `EL2ADLIB`: the AdLib sound driver ported, identical to the original, with an OPL2 model ([AUDIO.md](AUDIO.md))
- [ ] `EL2MCGA3`: the span filler, flat triangles, quadrilaterals and lines ported; the rest of the renderer
- [ ] The random number generator and the galaxy generation
- [ ] The data files: `EL2G.GRF`, the models, saves

## Systems

- [ ] `EL2M00`–`EL2M19`, the demand-loaded modules ([OVERLAYS.md](OVERLAYS.md), section 4)
- [ ] The main program's game logic in `FRONTIER.EXE`
- [ ] `EL2ROL`: MT-32 music from the `.ROL` files ([AUDIO.md](AUDIO.md), section 4)

## Labels and evidence

- [x] Deviations: the policy ([DEVIATIONS.md](DEVIATIONS.md)) and the generated ledger ([DEVIATIONS-LEDGER.md](DEVIATIONS-LEDGER.md))
- [x] Labels from the port's citations, one file per binary in `labels/` (91 labels)
- [ ] The open questions in [FINDINGS.md](FINDINGS.md): the `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` and `LATIS._0n` files
