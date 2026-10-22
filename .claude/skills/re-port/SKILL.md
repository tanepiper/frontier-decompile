---
name: re-port
description: Reimplement traced game behaviour in new code that behaves exactly like the original - mirroring its structure, numeric types and order of operations, random draw order, iteration order and integer semantics, citing the original function on every item and marking every intentional difference as a Deviation. Also how to shape the reimplementation (an exact core, commands, digests, a headless runner) and keep additions such as multiplayer or mod code apart from it. Use when writing the reimplementation of anything traced (re-trace) and measured (re-emulate).
---

# Reimplement, exactly

Precondition: the behaviour is traced and documented. Anything numeric has ground truth. If
you find yourself inventing a rule while writing code, stop and trace it.

## Write new code

Read the pseudocode to understand the behaviour, then write the new code from your notes, with
clear names and the target language's own types and structure. By default, do not paste
decompiled code into the new code base: it is the original author's work. It is also unreadable. This is the line between a reimplementation and a copy; keep it visible (see
`docs/LEGAL.md`). If the user chooses otherwise, say this once and follow their decision.

Before you write a sound chip, a codec or a language runtime yourself, check `docs/crates/`:
GameFrame may already have a checked implementation (OPL2, WMA v2, the MSVC maths functions,
Turbo Pascal's `Real`). Compare it with your game's version before you rely on it.

## Mirror the original's shape

- One module or type per original class or system, named after it where the name is known
  (from exports, RTTI or strings) and after its role otherwise.
- Keep the data flow: if the original builds a menu from a data file, build it from the same
  data file; do not hand-code what the game ships as data.
- Keep constants with their source address, exactly as read: `0.33333331f`, not `1.0 / 3.0`.
- Every reimplemented item names its original in a comment or doc comment, in one consistent
  form the tools can search, for example ``Original: `FUN_1400be680` ``. Mark guesses
  *inferred*.

## Build it so it can be checked

Decide the shape of the reimplementation early: these are cheap on day one and expensive to add
after a year.

- **An exact core with nothing else in it.** Put everything that decides what happens (the
  world, its rules, the clock, the random generators, loading and writing saves) in one module or
  library that does not draw, play sound, open windows or use the network. It then runs headless,
  in tests, on a server and on every platform. Add a test that fails if the core starts to depend
  on a drawing, window, sound or network library.
- **Every player action is a command.** Input becomes a small value ("place building X at Y",
  "set speed 3") that the core carries out at the exact point where the original reacts to the
  input, so the random draws stay in order. One command type gives you recording and replay,
  scripted tests and lockstep multiplayer from the same code.
- **A digest of the state.** A function that hashes the core's whole state (or the bytes of a
  save) lets two runs, two builds or two computers compare themselves cheaply at every
  checkpoint, and find the first step where they differ.
- **A headless runner.** A command-line mode that loads a save or starts a seeded game, runs a
  fixed number of updates or seconds with scripted input at fixed times, then prints digests or
  writes a save and a picture. Every whole-system check in `re-verify` is built on it.
- **A query layer for tests.** A way to ask the running program what it shows (interface
  elements by name or text, the world's counts) and to press things by name. Tests that find a
  button by its name survive layout changes; tests that click pixel positions or wait fixed
  times do not.

The platform layer around the core (time, input, drawing, sound, the player's files, the
browser build) is in `docs/working/building-the-port.md`. Shaders and fixed-function state are
part of what the player sees: port them exactly (`docs/hardware/shaders.md`).

## Additions the original does not have

Most reimplementations add things: multiplayer, a server, mod support, a debug probe, a new
camera, a browser version. Keep each one apart from the core:

- it lives in its own files, behind a build option, and the program builds and plays exactly
  the same without it (a test builds with every addition off and compares results with the full
  build);
- it reaches the core only through a few small, named entry points (commands, events after each
  step, read-only queries), never by editing the core's code paths;
- a run-time switch (for example `--pure`) turns every addition off for one run, so a player
  can always get the original game;
- anything that changes results is marked `Deviation:` and happens only when that addition is in
  use.

**Mod code that changes rules** (scripts or plugins run inside the simulation) must give the
same result on every computer, or replays and multiplayer break. Run it in a sandbox: integers
only (no floating point), no access to files, the clock or the network, random numbers only
from the game's own generator, a fixed instruction budget per call (a plugin that runs out
stops at the same instruction everywhere), a fixed order between plugins, and a fingerprint of
the loaded plugins that multiplayer players must share. A WebAssembly interpreter with fuel
metering and floats switched off does all of this.

## Generated record types (Borland programs with debug symbols)

`python3 tools/dos/tdinfo.py BINARY rust TYPE ...` writes Rust structs and readers for Pascal
record types in the original's field order (with `rust-reader` for the `PascalReader` they
use). Generate the code instead of copying it by hand. Keep the generated file unedited: regenerate it when the
toolkit improves. Rust projects made by new-project.sh build into `build/`: `target/` holds the game. `cargo clean` would delete it.

## Exactness checklist

These are the details that make a reimplementation drift, in order of how often they bite:

1. **Float precision.** Use the original's types: 32-bit floats stay 32-bit. Languages that
   compute in doubles by default (Python, JavaScript, Lua) need explicit rounding after every
   operation (`Math.fround`, `struct` round trip). 32-bit x87 games computed in 80-bit
   precision until a value was stored: match where it was stored.
2. **Order of operations.** Floating-point addition and multiplication are not associative:
   `(a + b) + c` differs from `a + (b + c)`. Keep the original's order, from the disassembly.
   Watch for fused multiply-add: compilers may fuse `a * b + c` on one side and not the other.
3. **Maths library.** `sin`, `cos`, `atan2`, `pow`, `exp` differ in the last bit between C
   runtimes. If the simulation depends on them, reimplement the original's runtime functions
   (emulate them for ground truth) instead of calling your platform's.
4. **Random draws.** Same generator, same seed, same number of draws, same order. A draw moved
   inside a condition, or skipped when a value is zero, changes everything after it.
5. **Iteration order.** Lists, arrays, hash maps: iterate in the original's order. Never let a
   hash map's order reach anything observable.
6. **Sorting.** Unstable sorts (C `qsort`) order equal elements in a way that depends on the
   algorithm; reproduce the original's algorithm when equal keys are possible.
7. **Integers.** Width, signedness, wrap-around on overflow, division rounding toward zero,
   arithmetic against logical shifts, float-to-int truncation (`cvttss2si`).
8. **Time.** Fixed step against variable step; integer ticks against float seconds; the exact
   step size and when the accumulated time is consumed.
9. **Order of evaluation.** The order of a call's arguments, and of the two sides of an
   operator, is the compiler's choice in C and C++. Take it from the binary. When two parts of
   one expression draw random numbers or change shared state, give each its own statement in
   the binary's order (`docs/COMPILED-CODE.md`, "Order of evaluation").
10. **Folded constants.** A float constant the compiler computed (`0x3eaaaaab` for 1/3) is exact
    in the binary. Copy its bits. An expression for it in the port may round differently.
11. **Update order.** Objects added or removed during an update, pools, queues, double
    buffers and dirty flags each decide what runs this frame (`docs/GAME-PATTERNS.md`, "Update
    order"). Copy each decision from the binary.
12. **Uninitialised and stale state.** The original may read a field before writing it, or keep
   a value from the last frame. That is behaviour too.

## Deviations

Any intentional difference (a platform difference, a fix the project decided on, an addition)
gets a comment containing `Deviation:` and the reason; a piece left out gets `not ported`.
`tools/triage/deviations.py --out docs/DEVIATIONS.md src/` lists them all in one ledger, and
`--check` fails a test when the ledger is out of date. A deviation is a decision; an accidental
difference is a bug.

`tools/triage/coverage.py SOURCE_DIR` reads the `Original:` citations and says how much of the
original's code (by count and by size) the reimplementation has ported from, with the largest
functions nobody cites yet.

## Workflow

1. List the functions to reimplement, from the topic document.
2. Do the smallest end-to-end slice first, and test it against ground truth before the rest.
3. After that, port a whole subsystem at a time (all of a routine's callees too), then verify
   it with one capture and one test (`re-verify` section 0 says how deep).
4. Build and test often; commit each verified subsystem (`re-land`).

`examples/sample-game/port.py` is a complete small example: a reimplementation that matches the
original bit for bit, and a `--naive` mode that makes mistake 1 to show what the checks catch.
