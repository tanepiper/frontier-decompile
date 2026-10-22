---
name: re-verify
description: Prove that a reimplementation matches the original - bit-exact parity tests against emulator ground truth (bitcompare.py), random-draw order (seqdiff.py), determinism of two identical runs with digests at checkpoints and on every platform, recordings and replays, round trips of real save files, picture comparison against the original game, comparison between two versions of the reimplementation, and test tiers. Use after every reimplemented slice and before committing it.
---

# Verify against the original

Report only what you ran and looked at: test names with pass and skip counts, files compared,
pictures opened. "It builds" is not verification.

## 0. How much to verify

Match the effort to the risk. Every slice gets at least the first row.

| Code | Check |
|---|---|
| Any ported routine | Recorded frames (a demo or a session) through the port, compared with the original's state after |
| Arithmetic, float or fixed-point maths, random draws, physics | Also direct calls on random inputs (`re-emulate`), thousands of them |
| Many branches, modes or record types that a recording misses | Also fuzz from real states (below) |
| Menus, text, plain drawing | Recorded frames and pictures only; no fuzz |

Run a big fuzz once when the slice is done, not after each change: a few dozen cases are
enough while you work.

## 1. Bit-exact parity with ground truth

Write the reimplementation's output in the same raw format as the emulator's (`re-emulate`),
then:

```bash
.venv/bin/python tools/verify/bitcompare.py f32 derived/emu/growth/seed-7.f32 out/growth/seed-7.f32
```

It compares by bits (0.0 and -0.0 differ), prints the match count, the first mismatches and the
largest difference in ULP (units in the last place). Reading the result:

| Result | Usual cause |
|---|---|
| All match | Done for these inputs; widen the inputs |
| Off by 1 to a few ULP from some point | Order of operations, a fused multiply-add, a maths library function, double against float |
| Large differences from the first value | Wrong formula, wrong constant, wrong input layout |
| Matches, then diverges for good | A random draw out of order, or state carried between steps |

Turn this into a test in the reimplementation's test suite: it loads the ground-truth files,
**skips with a clear message when `derived/` is missing** (other machines and CI have no game
files), compares exactly, and on failure reports the first mismatching index and the count.

For a DOS game, also compare against the whole original running in DOSBox-X: its exit code
(`scripts/dosbox-run.sh`), its file reads (`--fileio`), or memory read with the debugger at a
breakpoint (`tools/dos/dbx_control.py`). Fix the CPU speed (`--cycles N`) for any comparison
that depends on timing. Recorded demos (a file of inputs the game replays) are the best
end-to-end check: the reimplementation must replay them to the same frames.

### Reach the code a recording never reaches: fuzz from real states

A demo or a recorded session only covers some branches (one game mode, one difficulty, a
few event types). To cover the rest:

1. Capture real states at the start of the fragment, as the whole program runs (for a DOS
   game, memory images of the data and stack segments at a breakpoint).
2. For each case, take one state and change only the inputs the fragment reads: plant a
   random record (an event, a command), set random mode flags. Draw values the code tests
   for (sentinels such as -99, 0, boundaries, the ends of ranges) as often as random ones.
3. Load the changed state into the emulator and run the original fragment (`dos_emu`
   `run_range`). Heap data it reads goes into a new segment, with the pointers in the
   state set to it. Drop cases where the original never finishes, and say how many.
4. Save before and after, and replay them through the same test as the recorded cases.
   Report which inputs the passing cases covered (for example, the event types planted).

This finds what reading misses: loops that start at 0 instead of 1, writes one past an
array, branches only one mode takes.

### Capturing fragments of a main loop

A capture saves the state when execution reaches a fragment's start and again at its end,
frame after frame. For a DOS program, `tools/emu/fragments.py` does it in one headless run
(dos_emu + dos_env):

```bash
.venv/bin/python tools/emu/fragments.py target/game/GAME.EXE derived/fragments 2000 \
  'events=1000:2a14-1000:2a40,changed=0c20' 'draw=1000:2b00-1000:2b9e|1000:2bc4,every=5,max=200,heap=1' \
  --frame-top 1000:2a10 --screen 0a40 --display 0a42 --heap 0a50,0a54
```

- `--frame-top` is the top of the main loop (it counts frames). `--screen` and `--display`
  are the DS offsets of the words that hold the selectors of the screen being drawn and of
  the VGA display; `--heap` lists the words that hold heap blocks' selectors. `--ds` sets the
  data segment when it is not the program's start-up DS. `--start` starts at a routine
  instead of the entry; `--clock` and `--date` fix the time DOS gives.
- A SPEC is `NAME=START-END`. Join several ENDs with `|`. `changed=OFFSET` keeps only runs
  that changed that DS word; `every=N` keeps one in N; `max=K` stops at K; `heap=1` saves
  the heap blocks.
- It writes `derived/fragments/NAME/NNNNN.before.*` and `.after.*`: `.ds`, `.ss`, `.bp`,
  `.scr`, `.vga`, `.files` (handle, position, name) and `.heap` (selector + 64 KB each;
  `after` holds only the blocks that changed). The port's test loads `before`, runs the
  fragment and compares with `after`.
- Two fragments cannot share a START. A fragment's END may be the next one's START.

What a capture must hold:

- **Every way out.** A loop body may end in more than one place (the normal jump back, a
  shortcut, a jump to the level's end). Stop at any of them. A capture that waits for one
  end that a frame never reaches runs on into later frames and misses every other fragment.
- **The memory it reads.** Besides the data and stack segments and the screens, save the
  heap blocks the fragment reads or draws into (tile maps, off-screen buffers), as selector
  plus contents; for `after`, only the blocks that changed.
- **Open files.** Save each open DOS handle's name and position. Code that reads input from
  a file (a recorded demo) then replays from the same place in the port.
- **Pieces and the whole.** Capture each new piece of a loop body and the whole body too.
  The pieces say where a port goes wrong; the whole proves the wiring between them (state a
  piece's own test never saw, such as temporaries the caller keeps in globals).
- **State an interrupt owns.** A timer or music interrupt runs while the frame waits for its
  time: its counters after a frame depend on the host, not on the frame. Leave them out of
  the comparison with the reason written down. A clock the frame reads (the BIOS tick count)
  is an input: give the port the value the original read.
- **What it covered.** Before trusting a pass, count what the captured cases changed (the
  score, the armour, the enemies removed, the screen). A recording may never take a path;
  say which paths passed and which were never reached.

### Heap addresses and bytes that are not the game's

- **Addresses from the allocator.** Where the original stores heap pointers in variables
  (a loaded bank's selector, copied into each enemy), the port cannot rebuild the
  allocator's history from a snapshot. Give the port a heap whose allocator a test can
  script: before the run, queue the addresses the original returned (read them from the
  `after` state); with nothing queued it picks its own. Every other byte stays comparable.
- **Compare the named variables, list the rest.** The data segment also holds the runtime
  library's own state (its heap and file bookkeeping), padding, and blocks without symbols
  (a driver's data that an interrupt changes mid-frame). Build a mask of the bytes the
  port's variable model covers (store a model twice, over two different fills: the bytes
  that differ are not covered), compare only those, and write down what is left out and
  why. Never mask a byte you have not explained.

## 2. Random draw order

```bash
.venv/bin/python tools/verify/seqdiff.py derived/emu/ai/seed-1.trace out/ai/seed-1.trace
```

Make the reimplementation log each draw as `label<TAB>file:line`. seqdiff compares the labels
(and arguments) and shows the first divergence with the original's call site on one side and
your line on the other: usually it points straight at the bug.

## 3. Determinism

Run the reimplementation twice with the same seed and inputs and compare everything it outputs
(state dumps, saves, frames). Any difference means hidden nondeterminism: wall-clock time, hash
map order, uninitialised memory, threads, or a random generator seeded from the clock. This
check needs no original at all, so run it on every change.

Compare **digests at checkpoints**, not only the end: print a hash of the whole state every N
updates (`re-port`, "Build it so it can be checked"). The first checkpoint that differs narrows
the search to N updates; halve N until you have the step, then compare the state field by field.

**Every platform the reimplementation runs on** must give the same digests: native builds on
different CPUs, a 32-bit build, the browser (WebAssembly). Run one seeded game on each and
compare. Differences usually come from the platform's maths library, from fused multiply-add
on one target only, or from a different iteration order of a hash map.

**Recordings.** Record a game's commands with the digest at each checkpoint, then replay the
recording and check every digest. A recording made today is a regression test for every later change. It is also how a lockstep multiplayer game is checked: two players are two replays of
the same commands.

## 4. Real saves

Load saves made by the original game (several sizes and ages) and check every value you decode;
write them back and compare bytes when the reimplementation writes saves. Continuing a real
save for a while in the reimplementation, then comparing with the original continuing the same
save, is the strongest whole-system test you have.

## 5. Pictures

To compare rendering or interface with the original, capture the same view in both (same save,
camera, time, resolution) and measure the difference with an image tool (mean error, the share
of differing pixels, where they are), not by eye. A heat map of differences shows which part is
wrong. If the original only runs on another machine, stream it (Steam Remote Play, Parsec) and
capture there.

Make the reimplementation's pictures with its headless runner (a save, scripted input at fixed
times, a capture at a fixed time), never by hand, so the same picture can be made again after
every change. The same runner makes the screenshots and short videos for documentation.

### Memory frame by frame (console and computer games)

Run the same input movie in the emulator and in the port. Dump the game's memory after each
frame in both, the port writing its state in the original's layout, and compare with
`tools/verify/ramdiff.py ORIGINAL PORT --size N --map ram.map --ignore 0x100-0x1ff`. It gives the
first frame and the first named addresses that differ. Fix that one and run again. Keep the
movie and its dumps as a test (`docs/observe/emulators.md`).

## 6. Version against version

When you optimise or restructure the reimplementation, the results must not change: run fixed
scenarios with the old and the new build, compare saves and frames, and also run the old build
against itself to know what "identical" looks like on this machine. Any difference needs an
exact cause before the change goes in.

**Frames that differ while the state does not.** GPU drivers are not always deterministic: a
large scene can differ by a level or two on a few pixels between two runs of the same build.
When saves are identical and only a frame differs slightly, run that scenario again a few times
with both builds. If each build also differs from itself, it is the driver; record the case
(how often, how large) so the next person does not chase it. If the difference repeats every
time, it is real.

## 7. Test tiers

Group the checks by cost and run them by tier: a quick tier on every change (unit tests,
formats, a few headless scenarios against recorded baselines), a standard tier before a commit
that touches the game (determinism, pictures, builds without additions), and a long tier before
a release (replays, multiplayer, version against version). A baseline that changes is a change
in behaviour: find the cause, and record a new baseline only when the change is intended.

## 8. The toolkit itself

`tests/run.sh` proves GameFrame's tools against the sample game; run it after changing a tool.
