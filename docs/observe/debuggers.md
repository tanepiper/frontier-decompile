# Debugging the original

A debugger stops the original game at a chosen instruction or when a chosen value changes, and
shows its registers and memory at that moment. It answers questions that reading the code
cannot settle quickly: which of three functions runs when a unit dies, what a table holds once
the level has loaded, which code writes the score. This page lists the debuggers for each kind
of game, and how to connect Ghidra's debugger to an emulator.

## Debuggers by kind of game

| Game | Debugger | Notes |
|---|---|---|
| DOS | DOSBox-X's debugger | Breakpoints on addresses, interrupts and memory changes, memory dumps. `tools/dos/dbx_control.py` drives it from a script |
| DOS, early PC, booters | Bochs | A built-in debugger. Bochs can also be built with a GDB stub |
| Any PC, raw machine | QEMU | `-s -S` waits for a GDB connection on port 1234 before it starts |
| Windows | x64dbg, WinDbg | On Windows itself. Under Wine, `winedbg --gdb game.exe` serves the process to GDB |
| Arcade and many consoles and computers | MAME | A built-in debugger with memory views and watchpoints. `-debugger gdbstub` serves GDB for some CPUs |
| Game Boy Advance | mGBA | `-g` starts a GDB stub |
| Nintendo DS | DeSmuME | `--arm9gdb=PORT` and `--arm7gdb=PORT` start GDB stubs for the two CPUs |
| GameCube and Wii | Dolphin | A built-in debugger, and a GDB stub on some builds (set in its configuration) |
| PlayStation | PCSX-Redux | A built-in debugger, and a GDB server in its settings |
| Other consoles | The emulator's own debugger | The page for each console lists them ([Console games](../consoles/README.md)) |

Each debugger has its own commands. The parts that matter most are the same everywhere.

- **Memory breakpoints** (watchpoints) stop when an address is written or read. Use them with
  [Finding values in memory](memory.md): find the address of the score, then stop at the code
  that changes it.
- **Conditional breakpoints** stop only when a condition holds, such as a register value.
- **Logging breakpoints** print registers or memory and carry on without stopping. A series
  of them gives a trace of values over time. Compare that trace with the port's own trace
  using `tools/verify/seqdiff.py`.
- **Memory dumps** save a region to a file for `memscan.py`, Ghidra or a decoder.

## Ghidra's debugger

Ghidra 10 and later include a debugger. It shows the running program inside the same listing
and decompiler you use for reading, with your labels and types. It connects to GDB, LLDB and
WinDbg. Through GDB it reaches any GDB stub in the table above. Start the emulator with its stub,
then use Ghidra's GDB launcher with a remote target (`target remote localhost:PORT`). The
addresses in the emulator must match the addresses of the program in Ghidra. For a game that
loads at a fixed address, they match. For a program that moves, Ghidra maps the module by its
base address.

Ghidra records the session as a trace that you can step back through. A memory value or
register can be followed back to the instruction that set it.

## Ghidra's emulator

Ghidra can also run code without the game, with its P-Code emulator (`EmulatorHelper` in a
script, or the Emulator in the debugger). It runs any processor Ghidra can decompile:
68000, MIPS, SH-2, Z80, 6502, ARM and others. For those processors it can take the place of
Unicorn in [re-emulate](../../.claude/skills/re-emulate/SKILL.md). Set the registers and memory,
run one function, and read the result. It is slower than Unicorn and needs the same care with
the hardware the function reads.

## Code coverage: what runs when

![Coverage of two runs: the action's code is what ran only in the second](../assets/diagrams/coverage-diff.svg)

A game runs most of its code every frame. The code for one action, such as buying a unit,
saving or opening a door, runs only when that action happens. Coverage shows it. Record which
basic blocks run while you play without the action. Then record again, doing the same and the
action once. The blocks in the second run and not the first are the action's code.

- **Recording.** DynamoRIO's drcov tool (`drrun -t drcov -- game.exe`) writes a coverage log
  on Windows and Linux. Frida scripts write the same format. On a console or DOS emulator, a
  debugger that logs executed addresses gives a list to use instead.
- **Comparing.** `tools/triage/coverdiff.py BASELINE.log ACTION.log --module game.exe --base
  0x400000 --functions analysis/export/game.exe/functions.tsv` lists the functions that ran
  only with the action, by name.
- **Viewing.** Coverage plug-ins for Ghidra (such as Cartographer) and IDA (Lighthouse) colour
  the blocks that ran in the listing.

Keep the two runs alike: the same save, the same menus, the same length. Every extra
difference adds functions to the list.

## Patching a copy

Some questions are quicker to answer by changing the game. Examples are skipping a long intro
on every test run, or forcing a level to load first. Patch a copy, never the copy you analyse
([Handling the game as evidence](../working/evidence.md)). Record each patch with its address,
the old and new bytes, and its reason. Patches that defeat copy protection are a different
matter ([Packed and protected code](../media/protection.md)).

## What to record

- The emulator or debugger, its version, and the commands or script that set the breakpoints.
- For each answer, the breakpoint, the address it stopped at, and the values seen.
- Any patch to a copy, as above.
