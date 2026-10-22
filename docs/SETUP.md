# Setup

To install and check everything:

```bash
scripts/setup.sh --install   # Ghidra, JDK 21, objdump, Node, Zig; .venv with unicorn, pefile, capstone
tests/run.sh --ghidra        # proves every tool against the sample game (~1½ min)
```

`scripts/setup.sh` without options only checks and prints a table. The details (sizes,
Linux and Windows, REA registration for each agent, common problems) are in the `re-setup`
skill: [.claude/skills/re-setup/SKILL.md](../.claude/skills/re-setup/SKILL.md).

For Linux machines, containers and cloud agent workspaces, see
[Linux, cloud and container environments](working/environments.md). In each new shell or agent
session, `source scripts/env.sh` sets up the paths.

`./gameframe doctor` runs the same check. `./gameframe tools` then lists every tool with its
first line of help, and `./gameframe tool NAME` prints the rest.

![tests/run.sh on the sample game. Each tool has a line marked ok, grouped by section, and the summary reads 94 passed, 0 failed.](assets/captures/run-tests.svg)

When the setup is complete, the run ends like this. Each check runs a tool against the sample game. The summary line
counts the checks that passed and failed. With `--ghidra` the run has more checks.

| Tool | Used for | Where it comes from |
|---|---|---|
| Ghidra 11 or 12 | Decompiling native code | https://ghidra-sre.org (Homebrew: `ghidra`) |
| JDK 21 | Running Ghidra | Homebrew `openjdk@21`, apt `openjdk-21-jdk`, or Adoptium |
| Python 3.10+ with unicorn, pefile, capstone | Emulation, PE reading, disassembly | `requirements.txt` into `.venv` |
| objdump (binutils or LLVM) | Reading instructions exactly | Homebrew `binutils`/`llvm`, apt `binutils`; Xcode's on macOS |
| Node.js 18+ | REA (`npx -y rea-agents@6.4.0 mcp`) | https://nodejs.org |
| Zig (optional) | Building the sample game for Windows from any host | https://ziglang.org |
| REA (optional, recommended) | Evidence-backed inspection of binaries, packages, apps, runtimes | https://github.com/morluto/rea |

Other tools that fit the method, for targets the toolkit does not cover itself: ILSpy or
dnSpyEx (.NET), Il2CppDumper or Cpp2IL (Unity IL2CPP), unluac (Lua bytecode), FModel and UE4SS
(Unreal), RenderDoc and apitrace (graphics frames), x64dbg or Frida (debugging and run-time
hooks, on your own copy only), binwalk (unknown archives), Wine or a Windows machine with Steam
Remote Play (running the original next to the reimplementation).
