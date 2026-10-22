---
name: re-observe
description: Run the original game where it can be watched, driven and captured - GameFrame's emulators, DOSBox-X, Wine, a Windows machine or a streamed PC, console emulators - and capture frames, state, inputs, calls, file reads and sound as measured evidence. Also list every input from outside, inventory every interface screen, and build a queryable object model of the game with entry points for tests and agents. Use after inventory, before porting a system whose behaviour you need to see, and whenever a question can be settled by running the original.
---

# Observe the running original

Static reading says what the code could do; a run of the original says what it does. This
skill gets the original running somewhere you can watch, drive and capture it, and turns what
you see into evidence labelled *measured*. Details and tables are in `docs/observe/`.

## 1. Know the kind of game

Read `docs/games/README.md` and the family page for your game. Write the answers in
`docs/FINDINGS.md`: who drives (a stream, a simulation, an environment, turns), the time
model, how exact the interface must be. They decide what to capture.

## 2. Choose where to run it

| The game | Run it in | For |
|---|---|---|
| One function of a Windows x64 binary | `tools/emu/pe_emu.py` | exact results in tests (`re-emulate`) |
| One routine of a DOS program | `tools/emu/dos_emu` | the same, 16-bit |
| A whole DOS program, headless | `tools/emu/dos_env` | frames, memory and demos in scripts; keys from `env.keys` |
| A DOS program with a window | DOSBox-X (debugger, file log), scripted with `tools/dos/dbx_control.py` | breakpoints, memory, `fileio_trace.py` |
| A Windows game | Wine (`WINEDEBUG=+relay`), a Windows machine or VM, or a PC streamed to you | API calls, screenshots, debuggers, RenderDoc |
| A console or arcade game | an emulator with a debugger (Mesen, bsnes, Dolphin, PCSX2, MAME) | memory, input movies, frame advance |

Prefer headless, scripted runs for evidence. Use windowed runs to see what happens and where to
look. `docs/observe/running-the-original.md` has the full table.

In an emulator, start every run from a save state, record the inputs as a movie, and have a
script dump the game's memory after each frame. Those dumps are ground truth for
`tools/verify/ramdiff.py`. Search Data Crystal, The Cutting Room Floor and TASVideos for RAM
maps and mechanics before measuring (`docs/observe/emulators.md`).

## 3. Capture, with the inputs that made it

For every capture, write next to it the game version (hash), settings, seed, and the inputs
or demo, so it can be made again. Keep captures in ignored folders (`derived/`): they are game
material.

| Capture | How |
|---|---|
| Frames | video memory in the emulator; screenshots; `screencapture -l` of a window |
| State | memory dumps at chosen points; the game's own saves; `tools/triage/memscan.py` finds a value's address across dumps, `tools/triage/pointerpaths.py` a chain of pointers that finds it in every run, `tools/formats/markers.py` where a file's fields land (`docs/observe/memory.md`) |
| Frames | `tools/formats/rawimage.py` turns a dump of video memory into a PNG in the original's mode and palette |
| Inputs | the game's demo or replay files; breakpoints on input reads |
| Calls and random draws | `tools/emu/call_trace.py`; debugger traces |
| File reads | `tools/dos/fileio_trace.py` on DOSBox-X logs; Process Monitor; Wine's `WINEDEBUG=+file` (`docs/observe/system-calls.md`) |
| Calls to the system | Wine's relay log, strace, a proxy library (`docs/observe/system-calls.md`) |
| What it draws | apitrace or RenderDoc captures of the draw calls; dgVoodoo 2 for older APIs (`docs/observe/drawing.md`) |
| Network messages | Wireshark captures, then `tools/formats/pcap.py` and `tools/formats/fields.py` (`docs/observe/network.md`) |
| The code for one action | coverage of two runs, with and without the action, compared by `tools/triage/coverdiff.py` (`docs/observe/debuggers.md`) |
| Values at a moment | a debugger or an emulator's GDB stub, with memory breakpoints (`docs/observe/debuggers.md`) |
| Sound | the chip's register writes; WAV recordings |

## 4. List every input

Fill a table in `docs/FINDINGS.md` of every input from outside, with the address of each read:
controls, clocks and timers, random seeds, configuration and files, the command line and
environment, hardware detection, window messages, network. Search for them with the
signatures in `docs/observe/inputs.md` (interrupts, ports, API imports). Then design the port's
input record: one per frame or per command, the only way the rules receive anything from
outside.

## 5. Inventory the interface

List every screen: how to reach it, the code or layout that draws it, its states, inputs, what
it changes, its text and font, and its captures. Find screens by playing, by following the code
from the main menu, and by listing layout and string data. Capture each state with the inputs
that reached it. `docs/observe/ui.md` has the fields and how to rebuild data-driven and coded
interfaces.

## 6. Build the object model and its entry points

Give the original and the port the same model, named with the original's names: the interface
tree, the world's entities and fields, and the session (loop, frame, pause, seed). Add entry points as
the work needs them: query, input (as the player), commands, time (step, run *n*), state in and
out, trace, capture. Entry points only read or act as a player would; tests, scripts and agents use
nothing else. `docs/observe/object-model.md` has the rules and examples from three ports.

## Evidence to produce

- `docs/FINDINGS.md`: the kind of game, the inputs table, the screen inventory.
- Capture scripts in the project (not one-off commands), so every capture can be repeated.
- Labels (`labels/`) for every variable and function the captures name.

## Pitfalls

- A capture without its inputs cannot be repeated or trusted. Record the inputs first.
- A compressed video stream is not pixel-exact: compare layout. Use blurred image differences with a threshold.
- The clock and the random seed are inputs too. A second run that differs usually read one.
- A live online service belongs to someone else. Its terms usually forbid testing against it. Breaking copy protection is restricted in many countries. If the user's plan touches
  either, say so once with the source (`docs/LEGAL.md`, `docs/preservation/law/`), then follow
  the user's decision.
