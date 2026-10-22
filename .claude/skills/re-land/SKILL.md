---
name: re-land
description: Finish a slice of reverse-engineering work - write findings into docs/ in the evidence style, add function and field labels, update the checklist, then commit only your own files (never game material or decompiled code) and push, safely alongside other sessions. Use at the end of every trace, format, emulate, port or verify slice.
---

# Document, label, commit

Undocumented findings are re-derived by the next session, at full cost. Landing is part of the
work.

## 1. Document

- **Where:** extend the topic document if one exists (`grep -rln ADDRESS docs/`); otherwise
  create `docs/<TOPIC>.md` and link it from `docs/FINDINGS.md`.
- **Style:**
  - an opening paragraph: what it covers, which binary and version, which kinds of evidence
    (static reading of Ghidra pseudocode, disassembly, emulation, decoded files, observation of
    the running game) and what was not checked;
  - prose and tables keyed by function address and field offset;
  - every claim *observed*, *measured* (with the script and inputs), *inferred* (with what would
    confirm it), or under **Open questions**;
  - no copied pseudocode, disassembly or data dumps: describe them.
  - plain English with no rhetorical patterns (colon reveals, "X, not Y" endings, matched
    pairs, two clauses of the same shape joined by ", and", "and the ..." chains): see `docs/working/writing.md`.
- **Labels:** a row in `labels/names.tsv` for every function you identified
  (`1400be770<TAB>Citizen::ChooseJob<TAB>called from the AI tick, reads the job table`), a row in
  `labels/fields.tsv` for every field.
  Functions the port cites with `Original: FUN_…` get labels automatically:
  `tools/triage/cite_labels.py SRC --existing labels/names.tsv --out labels/names-from-port.tsv`,
  then `decomp_query.py --labels labels/names.tsv --labels labels/names-from-port.tsv`.
- **Checklist:** tick or add the items in `docs/CHECKLIST.md` and update its date.
- **Ledgers:** if the change adds or removes a `Deviation:` or `not ported` comment, regenerate
  the ledger (`tools/triage/deviations.py --out docs/DEVIATIONS.md SOURCE_DIR`) and commit it
  with the change. `tools/triage/coverage.py SOURCE_DIR` gives the progress figures for the
  checklist.

## 2. Check what you are about to commit

```bash
git status --short
git diff --stat
```

Never stage anything from `target/`, `analysis/` or `derived/`, game files (`.exe .dll .pak
.sav` and the game's own extensions), decompiled code, disassembly, emulator output, extracted
assets or screenshots of the original game. `.gitignore` covers the folders: never force-add.

## 3. Stage only your own work

Other sessions may edit the repository at the same time:

- stage files by name: `git add docs/AI.md labels/names.tsv` (never `git add -A`, `git add .`,
  `git commit -a`);
- in a shared file that also holds someone else's edits, stage only your lines (build the version
  with only your change and write it to the index with `git hash-object -w` and
  `git update-index --cacheinfo`), or ask;
- never revert or reformat changes you did not make.

### Mistakes that cost time

- **A scripted edit that silently does nothing.** A search-and-replace whose text does not
  match the file changes nothing and reports nothing. In scripts, check that each replacement
  matched (count it, or assert it). Look at `git diff` before you commit.
- **`git add` with a moved file.** After a file is moved, naming its old or new path may stage only part of the move. Then a commit that does not build goes in. After staging, check that
  nothing you meant to commit is still unstaged (`git status --short`, look for lines that do
  not start with a letter in the first column) and read `git diff --cached --stat`.
- **Processes left running.** Games, servers and emulators started for a test keep running
  after it fails. Stop them before you finish. Give long runs a time limit.

## 4. Commit and push

A subject in the imperative, at most 72 characters (`Trace the citizen job choice`), and a body
that says what was traced, measured or reimplemented and how it was checked. Then:

```bash
git show --stat HEAD                  # only your files?
git pull --rebase && git push
```

If the rebase conflicts in files you did not touch, stop and report.

## 5. Ship

When a version is ready for players, `docs/OUTPUTS.md` lists what to release and how: programs
for each system built by CI from the tag, a browser version, a website.

## 6. Report

Two or three lines: the commit, what landed, what is verified, and the next open item.
