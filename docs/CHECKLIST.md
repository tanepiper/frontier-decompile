# Checklist: Frontier: Elite II

Updated: 2026-10-23

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
- [x] `EL2MCGA3`: all 40 exports ported and verified: spans, flat and textured polygons, lines, discs, scanline and mask fills, text, bitmaps and their EMS cache, the palette, the flip ([VIDEO.md](VIDEO.md))
- [ ] The draw-command layer and a WebGPU backend that renders it at higher resolution ([VIDEO.md](VIDEO.md) §10)
- [x] The random number generators: `random(n)` `1000:069b`, `random32` `1000:0645`, `mix` `1000:066a`, ported and verified ([RNG.md](RNG.md))
- [x] Galaxy: the density map (`EL2M19`), sector density `EL2M02:3b34`, sector seed `EL2M02:3abe` and sector star list `EL2M02:3c2f`, ported and verified ([GALAXY.md](GALAXY.md))
- [x] Names: the `EL2M09` name generator `EL2M09:0f92` and the text expander `1000:e969`, ported and verified, with the nested sub-name string ([NAMES.md](NAMES.md))
- [ ] Star systems: from a star in the list to its system, type, name and planets ([GALAXY.md](GALAXY.md); leads: the routines only a hyperspace jump runs, in `EL2M00` and `FRONTIER.EXE`, [COVERAGE.md](COVERAGE.md))
  - [x] Bodies: seed step, classifiers, small- and large-body adders, naming (`EL2M00:4990`–`4e29`), ported and verified ([SYSTEM.md](SYSTEM.md))
  - [x] Body set-up and finishing `EL2M00:468a`, `47d8` and every classifier in the table ([SYSTEM.md](SYSTEM.md))
  - [ ] The orbit builder `EL2M00:4e3f` ([SYSTEM.md](SYSTEM.md))
- [x] `EL2G.GRF` (`1000:e2ba`, `EL2MCGA3:02d3`) and the `PICnnn.GRF` screenshots (`1000:db9f`) decoded, every byte of the shipped files ([FORMATS.md](FORMATS.md))
- [x] The `.ROL` songs read as Standard MIDI Files ([FORMATS.md](FORMATS.md) §3)
- [ ] The models: tables (`1000:18b5`), header, vertices; every command's length measured, every model walks to its end; ships named from their data blocks (four checked in the game), stations and starports grouped (*inferred*); what opcodes `0A`, `0F`–`12`, `15`–`1C`, `1E` draw, not traced ([FORMATS.md](FORMATS.md) §4)
- [ ] Saves and the position files (`11 00 C6`, packed) ([FORMATS.md](FORMATS.md) §6)
- [x] Resource viewer (`apps/viewer`): every file, native and browser

## Systems

- [ ] `EL2M00`–`EL2M19`, the demand-loaded modules ([OVERLAYS.md](OVERLAYS.md), section 4)
- [ ] The main program's game logic in `FRONTIER.EXE`
- [ ] Coverage runs still to add: combat, docking, buying and selling, missions, the system map
- [ ] `EL2ROL`: MT-32 music from the `.ROL` files ([AUDIO.md](AUDIO.md), section 4)

## Labels and evidence

- [x] Deviations: the policy ([DEVIATIONS.md](DEVIATIONS.md)) and the generated ledger ([DEVIATIONS-LEDGER.md](DEVIATIONS-LEDGER.md))
- [x] Labels from the port's citations, one file per binary in `labels/` (221 labels)
- [x] Hand labels for the routines only one action runs (`labels/names.tsv`, `labels/names-EL2M00.tsv`, `labels/names-EL2M10.tsv`), roles inferred
- [ ] The open questions in [FINDINGS.md](FINDINGS.md): the `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE` and `LATIS._0n` files (all packed, same 3-byte header; [FORMATS.md](FORMATS.md) §6)
