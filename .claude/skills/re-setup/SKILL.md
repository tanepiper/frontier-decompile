---
name: re-setup
description: Install and check the reverse-engineering toolchain (Ghidra, JDK 21, Python with Unicorn/pefile/capstone and optional Frida, objdump, Node for REA, Zig for the sample) and register the REA MCP server for the agent. Use at the start of a project, when a tool is missing or fails to start, or when REA tools do not appear in the session.
---

# Set up the toolchain

## 1. Check, then install

```bash
scripts/setup.sh              # a table: ok / MISSING for each tool, with a hint
scripts/setup.sh --install    # installs what is missing (Homebrew on macOS, apt on Debian/Ubuntu)
                              # and creates .venv with requirements.txt
```

`./gameframe doctor` runs the same check, and passes options on (`./gameframe doctor
--install`). `./gameframe run NAME ARGS` then runs any Python tool with the `.venv`.

The script writes `.env` with `GHIDRA_INSTALL_DIR` and `JAVA_HOME` once both are found. Every
script and `.mcp.json` read these, so set them there if your installs are elsewhere.

Sizes (tell the user before installing on their machine if they have not approved it): Ghidra
about 500 MB, JDK 21 about 300 MB, the Python packages about 40 MB, Zig about 200 MB (only for
building the sample game).

On Linux, Ghidra is not packaged: download the release zip from
https://github.com/NationalSecurityAgency/ghidra/releases, unzip it, and put its folder in
`GHIDRA_INSTALL_DIR`. On Windows, use WSL2 and follow the Linux steps.

On Linux, in a container or in a cloud agent's workspace, read
`docs/working/environments.md` as well. In each new shell, `source scripts/env.sh` loads `.env`
and puts `gameframe` and the `.venv` Python on `PATH`.

## 2. Prove it works

```bash
tests/run.sh            # ~20 s: builds the sample game, runs every tool against it
tests/run.sh --ghidra   # + a real Ghidra export (~1 min)
```

All checks must pass. A failure names the tool and shows its output.

## 3. REA (the reverse-engineering MCP server)

REA ("Reverse Engineer Anything", https://github.com/morluto/rea, npm package `rea-agents`)
gives the agent tools to open binaries, archives, Android packages, .NET assemblies, browser
pages and Electron apps, decompile through Ghidra, and keep every result as cited Evidence.
See `re-rea` for when to use it.

- **Claude Code:** this repository has a project `.mcp.json` that starts REA with
  `npx -y rea-agents@6.4.0 mcp`, passing `GHIDRA_INSTALL_DIR` and `JAVA_HOME`. Claude Code asks
  once to approve project MCP servers; after that the `mcp__rea__*` tools appear.
- **Other agents** (Codex, Cursor, Gemini CLI, Windsurf...): use REA's own setup, which writes
  the client's MCP configuration and installs REA's skill:
  ```bash
  scripts/setup.sh --rea                          # dry run: shows the plan for Claude Code
  REA_CLIENT=codex scripts/setup.sh --rea         # the plan for another client
  npx -y rea-agents@6.4.0 setup --client codex --yes   # apply, after the user agrees
  ```
- REA's own skill (`reverse-engineer-anything`) explains every tool. Install it with REA's setup,
  or from its repository.

REA lists the hosts it supports. On others (for example Debian 13) its doctor warns, but the MCP
connection, Ghidra-backed tools and the sample checks have worked. Try REA on the sample game
before relying on it.

Ghidra analysis of a large binary can take minutes, longer than many clients wait for one
tool call. Raise the client's tool timeout for REA (in Codex, `tool_timeout_sec` in the
server's entry).

If REA tools do not appear: restart the agent (MCP servers start with the session), then check
`npx -y rea-agents@6.4.0 doctor --client claude_code --json`. A tool that reports
`provider_unavailable` usually means Ghidra or Java is not where `.env` says.

## 4. DOS games: DOSBox-X and its debugger

- DOSBox-X: `scripts/setup.sh --install` (on macOS the formula, `brew install --formula
  dosbox-x`; the cask is disabled for failing Gatekeeper). Homebrew's build has the heavy
  debugger; `brew install --HEAD --formula dosbox-x` adds the newest commands (`STEPIN`). Check:
  `scripts/dosbox-run.sh` on the DOS sample prints `exit code 23` (tests/run.sh does this).
- **The debugger control channel needs DOSBox-X 2026.10 or later**, built with the debugger.
  `dosbox-run.sh --mcp`, `tools/dos/dbx_control.py` and the `dosbox` MCP server all use it.
  Distribution packages are often older (Debian 13 ships a 2025 build). `scripts/setup.sh`
  then shows a `dosbox-x debugger OLD` row, `--mcp` stops with a message, and the debugger test
  is skipped. Build a current one on Linux (ask the user first: it is a download and a long
  build):
  ```bash
  sudo apt install libtool autogen autoconf automake libncurses-dev gcc g++ make nasm \
    libsdl2-dev libsdl2-net-dev libpcap-dev libslirp-dev fluidsynth libfluidsynth-dev \
    libavformat-dev libavcodec-dev libavcodec-extra libswscale-dev libfreetype-dev \
    libxkbfile-dev libxrandr-dev
  git clone --depth 1 https://github.com/joncampbell123/dosbox-x.git
  cd dosbox-x && ./build-debug-sdl2      # SDL2 with the heavy debugger
  ```
  The package list follows DOSBox-X's `BUILD.md`; the debugger needs `libncurses-dev`. Use
  the repository's build script. It passes the compiler flags the code needs; a plain
  `./configure && make` with a newer compiler can stop on a warning treated as an error.
  Put the result on `PATH` (or `sudo make install`) and run `scripts/setup.sh` again.
- The debugger MCP server for agents (ask the user first: it is a download and a build):
  `cargo install --git https://github.com/caiiiycuk/dosbox-x-mcp-server --tag v0.1.2 --locked`.
  `.mcp.json` registers it as `dosbox` (set `DOSBOX_X_MCP_SERVER` to its path if it is not on
  PATH). Restart the agent session so the tools load, ask `dosbox_mcp_port`, then run
  `scripts/dosbox-run.sh --mcp PORT ...`.

## Common problems

- `dosbox-x not found` after `brew install --HEAD`: run `brew link dosbox-x`.
- `dbx_control` or the dosbox MCP server times out waiting for DOSBox-X: check that
  `scripts/setup.sh` shows no `OLD` row. An older DOSBox-X ignores `mcp_server` and never
  connects.

| Symptom | Cause and fix |
|---|---|
| `Ghidra not found` | Set `GHIDRA_INSTALL_DIR` in `.env` to the folder that contains `support/analyzeHeadless` |
| Ghidra exits with `UnsupportedClassVersionError` | Wrong Java: set `JAVA_HOME` to a JDK 21 |
| Ghidra runs out of memory on a large game | `MAXMEM=16G scripts/ghidra-export.sh ...` |
| `Unable to lock project` | Another headless job uses the same Ghidra project: wait, or pass `--project` |
| `pip install unicorn` fails to build | Use a Python version that has a Unicorn wheel (3.10 to 3.13 are safest): `PYTHON=python3.12` |
| The sample game does not build | Install Zig (`brew install zig`, or https://ziglang.org/download) |
| `frida` MISSING in the check | Optional, for live tracing: `scripts/setup.sh --install`, or `.venv/bin/pip install -r requirements-live.txt` |
| Frida cannot attach on macOS | The program is signed with the hardened runtime. Trace a copy re-signed without it, or one you built |

## Working in a git worktree

A worktree has no `target/`, `analysis/` or `.venv`. Symlink them from the main checkout and add
them to `$(git rev-parse --git-common-dir)/info/exclude` (a symlink is not matched by a
`/target/` rule in `.gitignore`).
