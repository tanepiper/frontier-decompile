# Linux, cloud and container environments

GameFrame runs on macOS and Linux, on a desktop or headless. This page lists what an agent
needs to set up a Linux machine, a container or a cloud agent's workspace, and what to check
before work starts.

## Ask your agent

Set up a Linux environment. Copy this prompt into your coding agent:

```text
Set up GameFrame in this Linux environment.

1. Clone https://github.com/tanepiper/game-frame (or update it) and read
   docs/working/environments.md and the re-setup skill.
2. Run scripts/setup.sh and show me the table. Tell me what is missing and how large each
   part is. Wait for my answer before installing anything.
3. Install what I agree to. Verify Ghidra's download against the SHA-256 on its release page.
4. If DOSBox-X is older than 2026.10, tell me, and build a current one with the debugger
   only if I agree.
5. Run tests/run.sh --ghidra and report the count. Fix the setup until every check passes or
   is skipped for a reason I accept.
6. Write a short report: the versions installed, the test result, and anything left to do.
   Keep logs outside the repository.

Never put game files, extracted assets or decompiled code inside a git checkout.
```

## What the toolkit needs

| Tool | Version that works | Needed for |
|---|---|---|
| Python | 3.10 to 3.13, with `.venv` from `requirements.txt` (Unicorn 2.1, pefile, Capstone 5) | every tool |
| JDK | 21, the full JDK (with `javac`) | Ghidra |
| Ghidra | 11 or 12 (12.1.4 tested) | decompiling |
| objdump | GNU binutils or LLVM | reading instructions exactly |
| Node.js | 18 or later (24 tested) | REA |
| Zig | 0.14 | building the sample game in the tests |
| DOSBox-X | 2026.10 or later, with the heavy debugger | DOS games, the debugger tests |
| innoextract, 7-Zip | any recent | unpacking installers |
| REA | 6.4.0 | the MCP server for agents (optional) |

`scripts/setup.sh` checks each one and prints a table. `--install` installs with apt on Debian
and Ubuntu. Ghidra is not in apt: download the release zip, check its SHA-256 against the
release page, unzip it and set `GHIDRA_INSTALL_DIR`.

## Steps on a Debian or Ubuntu machine

1. Install the system packages: `python3-venv`, `openjdk-21-jdk`, `binutils`, `nodejs`,
   `innoextract` and `p7zip-full` (or let `scripts/setup.sh --install` do it).
2. Install Ghidra from its release zip, as above.
3. Run `scripts/setup.sh --install` to make `.venv` and write `.env` with the tool paths.
4. Check the DOSBox-X row. Distribution packages are often older than 2026.10. Build a current
   one with the debugger as the re-setup skill describes, or leave DOS games out.
5. Run `tests/run.sh --ghidra`. Every check should pass, or skip with a reason.

## Each session

Agents often start a new shell for each command. Source `scripts/env.sh` at the start of each
one:

```bash
source ~/code/game-frame/scripts/env.sh
```

It sets `GAMEFRAME_ROOT`, loads `.env`, exports `GHIDRA_INSTALL_DIR` and `JAVA_HOME`, and puts
`gameframe` and the `.venv` Python first on `PATH`. The scripts load `.env` themselves, so
this matters for commands typed by hand and for agents.

## No display

The tests and the tools need no display. `dosbox-run.sh` uses SDL's dummy video and audio
drivers unless you pass `--window`. A DOSBox-X built without X11 or OpenGL support runs the
tests. Ghidra runs headless.

## REA in a cloud or container session

- MCP servers start with an agent session. After you register REA, start a new session to get
  its tools.
- REA's doctor may warn about a host it does not list, such as Debian 13. Test it on the
  sample game: open `examples/sample-game/build/game.exe` and analyse a function.
- Ghidra analysis can take minutes. Raise the client's tool timeout for REA.

## Game material on someone else's machine

A cloud machine belongs to its provider. Before you put a game there, check that your
licence and the provider's terms allow it.

- Keep the game's files, extracted assets, decompiled code and analysis output outside every
  git checkout.
- `scripts/new-project.sh DIR "Name"` without a game path creates the project and copies
  nothing. Put the game's files in the project's ignored `target/` folder only where you are
  allowed to.
- Each cloud task usually runs in its own copy, so a git worktree is rarely needed.

## Keep the environment repeatable

- Save the versions you installed, and the output of `scripts/setup.sh` and
  `tests/run.sh --ghidra`, outside the repository.
- Write a small refresh script for your environment that pulls GameFrame, reruns
  `scripts/setup.sh --install` and registers REA again.
- Report changes you had to make to GameFrame's own files upstream, so the next environment
  does not need them.
