---
name: frontier-land
description: Land Frontier reverse-engineering or port work — write findings into docs/ in the project's evidence style, update the milestone list, then commit only your own changes (never game data or decompiled code). Use at the end of every trace, format or port slice.
---

# Document and commit

## 1. Document

- Extend the topic doc if one exists (`grep -rln <address> docs/`), else create
  `docs/<TOPIC-IN-CAPS>.md`. Opening paragraph: what it covers, binary, evidence type (Ghidra
  12.1.4, disassembly, emulation, DOSBox observation) and what was *not* observed.
- Tables keyed by address (`1000:2f3a`, `EL2M07:0412`) and field offsets; every claim is observed,
  *inferred*, or under **Open questions**. No copied pseudocode, disassembly or file dumps.
- Update the milestone list in `README.md` and `docs/FINDINGS.md` when a capability lands.

## 2. Check before committing

```bash
git status --short
git diff --stat
```
Never stage `game-data/`, `analysis/`, original files (`*.EXE *.OVL *.GRF *.ROL`), `.venv/`,
`target/`, `dist/`, captures or emulator dumps. Never force-add past `.gitignore`.

## 3. Stage only your own work

Never `git add -A` / `git add .` / `git commit -a`. Add your own paths by name. If a shared file has
someone else's edits, stage only your hunks (build the staged blob with `git hash-object -w` and
`git update-index --cacheinfo`), or ask.

## 4. Commit

Imperative subject ≤ 72 chars naming the trace/port; body says what was traced/ported and how it
was verified. End with the co-author line from the current session's attribution instructions.
Push only when a remote is configured and the user has asked for pushes.
