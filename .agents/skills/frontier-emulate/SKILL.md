---
name: frontier-emulate
description: Get bit-exact ground truth from the original Frontier code, by running routines from FRONTIER.EXE or an overlay under an emulator (Unicorn in 16-bit mode, or the DOSBox debugger) and proving the Rust port matches with a parity test. Use for numeric or procedural logic (fixed-point maths, RNG, galaxy and system generation, orbits, model decoding).
---

# Emulate for ground truth, port until bit-exact

## Tools

- **Unicorn, 16-bit real mode** (`UC_ARCH_X86`, `UC_MODE_16`) for calling one routine with chosen
  inputs. A harness `tools/emu/mz_emu.py` does not exist yet: build it on the Banished
  `tools/emu/pe_emu.py` pattern (load the MZ image at a segment, apply the relocation table,
  set `SS:SP`/`DS`, hook `int` instructions, `call(seg, off, regs)` returning registers). Keep
  it dependency-light: `python3 -m venv .venv && .venv/bin/pip install unicorn`.
- **DOSBox** for whole-program state (`tools/dosbox/run.sh`). DOSBox Staging's debugger needs a
  debug build; DOSBox-X has a built-in debugger. Use it to dump memory or registers at a point
  you found statically.

## Steps

1. Trace first (`frontier-trace`): the routine's register/memory inputs, globals it reads, and
   callees that touch DOS, BIOS or hardware ports (those must be hooked).
2. Write a generator `tools/emu/<topic>.py` (docstring with `Usage:` and outputs). Cover edge cases.
   Write raw outputs to `game-data/derived/emu/<topic>/` (git-ignored), never to tracked paths.
3. Port into `crates/game`, mirroring the arithmetic exactly (16/32-bit widths, wrapping, signed
   shifts, carry-dependent sequences).
4. Parity test in `crates/game/tests/` that **skips with a message if `game-data/` is missing**, and
   compares exactly, reporting the first mismatch and a count.
5. Record in the doc which generator and inputs proved parity.
