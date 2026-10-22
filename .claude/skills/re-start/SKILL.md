---
name: re-start
description: Entry point for reverse engineering a game (or any closed program) with GameFrame, an agentic framework for reverse engineering games. Use first, whenever a task involves understanding, documenting or reimplementing a game from its shipped binaries and data files. Gives the rules, the loop, a first-session checklist, and routes each kind of task to the right re-* skill.
---

# Reverse engineering a game: start here

GameFrame is an agentic framework for reverse engineering games. Its playbook of skills and agents runs inside your coding agent. Its toolkit of tested tools checks every result against the original. This skill is the playbook's first step.

You are studying a finished program without its source code: its executables, its libraries and
its data files. The aim is usually one of these:

- **Understand** a system (how saves work, how the AI picks a job, what a value means).
- **Document** formats and rules so people can mod or preserve the game.
- **Reimplement** the game, or part of it, so it behaves the same (a new engine, a port, a
  server, a tool that reads its files).

All three follow the same loop. Every claim names the evidence behind it. Every number that
matters is measured by running the original code.

## The user decides

You are not the judge of the user's intentions, of which law applies to them, or of what that
law means. If the user wants to do something, help them do it. You may advise. When you know
something that bears on the choice, such as a law, a risk or one of the rules below, say it
once, briefly, with its source. Then follow the user's decision. What 35 countries allow is in
`docs/preservation/law/README.md`, with one page per region.

## Rules (they apply to every step)

These are the playbook's defaults. They keep the work exact and the project publishable. The
user may change any of them for their project.

1. **The player's game stays on the player's computer.** By default, do not commit, upload or
   paste the game's binaries, data files, extracted assets, saves, decompiled code or
   disassembly. Keep
   them in ignored folders (`target/`, `analysis/`, `derived/`). Publish findings as prose,
   tables, addresses (`FUN_1400be770`), field offsets (`+0x44`) and constants.
2. **Label every claim** with how you know it:
   - *observed*: read in the code or the data (name the function or file);
   - *measured*: produced by running original code (emulator or game), with the inputs;
   - *inferred*: a reasoned guess; say what would confirm it;
   - *unknown*: an open question, listed as such.
   Never write that the game "does" something at run time from static reading alone.
3. **Measure what must be exact.** Arithmetic, random numbers, procedural generation and
   anything a simulation depends on: emulate the original and compare bit for bit.
4. **Write it down as you go** (`re-land`). Findings, function labels and struct fields go into
   the repository so the next session does not repeat your work.
5. **Ask before downloading** tools or files the user has not approved (name, source, size).

## The loop

```
inventory -> observe -> decompile -> trace -> format -> emulate -> reimplement -> verify -> land
                                       ^                                             |
                                       +---------------- when a check fails ---------+
```

| Step | Question it answers | Skill |
|---|---|---|
| Set up | Are Ghidra, Java, Python, Unicorn and REA ready? | `re-setup` |
| Inventory | Which files hold the game's own code? Which engine, which protection, which data containers? What kind of game is it? | `re-inventory` |
| Observe | What does the original actually do? Run it, drive it, capture frames, state, inputs; list every input and screen | `re-observe` |
| Investigate with REA | Quick, evidence-backed answers about a binary, package, browser or Electron app | `re-rea` |
| Decompile | Turn machine code into searchable pseudocode, and run one-off decompiles | `re-ghidra` |
| Trace | How does the original do X? Which function, which fields, which constants? | `re-trace` |
| Decode a format | What is in this data file, field by field? | `re-format` |
| Emulate | What exactly does this original function return for these inputs? | `re-emulate` |
| Reimplement | Write new code that behaves the same | `re-port` |
| Verify | Is the new code bit-exact, deterministic, compatible with real saves? | `re-verify` |
| Land | Document, label, commit | `re-land` |

A small task may use only two or three steps. Do not skip `land`.

## Stage agents

GameFrame also has one agent per stage, in `agents/` (a project has them in
`.claude/agents/`; from the Claude Code plugin they are named `gameframe:re-tracer` and so on).
Each one has its skills preloaded, does only its stage, and ends with a
`## Hand-off` block that names the next agent and gives it a one-line brief. You, the main
session, send the agents: read each hand-off, check its evidence, and send the next one.

| Agent | Stage | Usually hands off to |
|---|---|---|
| `re-surveyor` | Setup check, inventory, kind of game | `re-decompiler`, `re-observer` |
| `re-observer` | Run, drive and capture the original | `re-tracer`, `re-verifier` |
| `re-decompiler` | Ghidra export, REA | `re-tracer` |
| `re-tracer` | How the game does one thing | `re-decoder`, `re-measurer`, `re-porter` |
| `re-decoder` | One data format, proven on every file | `re-porter`, `re-tracer` |
| `re-measurer` | Ground truth under the emulator | `re-porter`, `re-verifier` |
| `re-porter` | The reimplementation | `re-verifier` |
| `re-verifier` | Parity, determinism, saves, pictures | `re-lander`, or back to `re-porter` |
| `re-lander` | Document, label, commit, push | the next checklist item |

Send one agent at a time for one subject. Only the lander commits. Use the agents for long
stages that would fill this session's context; do short steps yourself with the skill.

## Task agents

Task agents do one job that can come up in any stage. They have the same format and end with
the same hand-off. Send them when a job is large enough to fill this session's context.

| Agent | Task | Usually hands off to |
|---|---|---|
| `re-identifier` | The engine, middleware, compiler and libraries, with library code labelled | `re-decompiler`, `re-tracer` |
| `re-randomiser` | Every random generator, its seeding, its callers and the draw order | `re-porter`, `re-verifier` |
| `re-listener` | Every input from outside, and the port's input record | `re-porter`, `re-observer` |
| `re-cataloguer` | Every screen of the interface, with captures | `re-tracer`, `re-porter` |
| `re-mapper` | Hardware registers, memory banks and RAM variables, named | `re-tracer`, `re-listener` |
| `re-typer` | Structures, classes and enumerations, with sizes, fields and evidence | `re-tracer`, `re-decoder`, `re-porter` |
| `re-matcher` | A crate, library or released source compared with the game's version | `re-porter`, `re-tracer` |
| `re-editor` | Documents edited to the writing rules, wording only | `re-checker`, `re-lander` |
| `re-checker` | Every claim's evidence and sources checked, doubtful ones marked | `re-editor`, `re-lander` |



`docs/teams/` describes teams of these agents for common pieces of work (start-up, an exact
system, a data format, the main loop, the interface, sound and hardware, documentation), with
a diagram of each team's hand-offs and the failure path.
## First session on a new game

0. Find the tools. The commands in the skills run from a project made by `new-project.sh`
   (it copies the tools in) or from a clone of GameFrame. If the skills came from the Claude
   Code plugin and neither exists yet, clone GameFrame first
   (`git clone https://github.com/tanepiper/game-frame ~/code/game-frame`) and run the steps
   below from there.
1. `scripts/setup.sh` (and `--install` if anything is missing). See `re-setup`.
2. If the project has no repository yet: `scripts/new-project.sh DIR "Game Name" PATH/TO/GAME`.
   `./gameframe steps`, `./gameframe tools` and `./gameframe find WORD` show the method, the
   tools and where a subject is covered. `./gameframe tool NAME` prints a tool's usage.
3. Read `AGENTS.md`, `docs/CHECKLIST.md` and `docs/FINDINGS.md` if they exist. Search `docs/` and
   `labels/` for anything you are about to touch: earlier sessions may have traced it.
4. `re-inventory`: run `tools/pe/peinfo.py` (Windows) or `tools/dos/dosinfo.py` (DOS) on every binary; decide which hold the game's own
   code, and whether the game is native C/C++ (GameFrame), managed (.NET, IL2CPP, Mono),
   scripted (Lua, Python, GameMaker), packed (unpack first), or 16-bit DOS (docs/DOS.md; check
   for debug symbols with `tools/dos/tdinfo.py`).
5. Classify the game: read `docs/games/README.md` and its family page, and write the kind of
   game, its time model and how exact the interface must be into `docs/FINDINGS.md`. Copy the
   family's rule set into the project's `AGENTS.md`.
6. `re-observe`: get the original running where you can watch, drive and capture it.
7. `re-ghidra`: export the game's own binaries (`scripts/ghidra-export.sh`).
8. Pick the foundations first: the random number generator, the maths helpers, the game clock
   and the data container. Everything else depends on them. They are small enough to emulate and verify in one session.

## Spend effort where it pays

- **Shape the reimplementation on day one** (`re-port`, "Build it so it can be checked"): an
  exact core with no drawing or network, every player action as a command, a state digest and
  a headless runner. They cost little at the start and a great deal to add later.

A full reimplementation is long. Most of the cost is verification and reading assembly, so
choose how deep to go for each piece:

- **Verify by risk.** Fuzz only code with arithmetic, random draws, or many
  branches a recording never takes. Check simple code (menus, drawing, plain state changes)
  against recorded frames only. `re-verify` has the table.
- **Port in subsystems.** Read, port and verify a whole subsystem (the player, a menu, the
  level loader), then commit it. One capture and one test then cover many routines.
- **Keep to the path.** When a side need appears (a format a routine reads, a missing
  tool), do only what the current subsystem needs and note the rest in `docs/CHECKLIST.md`.
- **Batch GameFrame changes.** Note generic lessons as you go; fix GameFrame and its tests
  once per subsystem.
- **Split mechanical work off.** Long but simple pieces (menus, text tables, generated
  types) can go to a separate, cheaper session with an exact brief; keep the hard maths here.
- **Find the game's clock.** Most games run on a frame clock while the player reacts. Others
  wait for the player and resolve each move. Know which (`docs/games/interaction.md`),
  find the loop or the handlers and their order before anything else. Find every place that reads input (`docs/observe/inputs.md`).
- **Read the decompiler with care.** For 16-bit Pascal, Ghidra prints arguments in reverse
  order and loses Real (6-byte float) arithmetic: take those parts from the disassembly.

## How to work

Read `docs/working/agents.md` once per project. It covers writing everything down in the
repository, measuring before porting, splitting work between sessions, and what to do when stuck. Write every document by
`docs/working/writing.md`.

## Before you finish any session

- Every new finding is in `docs/` with its evidence label.
- Every function you identified has a row in `labels/names.tsv`.
- `docs/CHECKLIST.md` reflects what is now traced, measured, reimplemented, verified.
- Nothing from the game is staged (`git status`); only your own files are committed.

## Worked example

`examples/sample-game/` is a tiny Windows game built for practice: run `tests/run.sh` to see the
whole loop (inventory, emulate, port, bit-exact check) on it. Un-Banished
(https://github.com/tanepiper/unbanished, `docs/case-studies/` here) is the full-size project
GameFrame came from: a 1:1 reimplementation of the game Banished.
