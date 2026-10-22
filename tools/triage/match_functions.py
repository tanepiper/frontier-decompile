#!/usr/bin/env python3
"""Match the functions of two builds of a game, and carry your labels from one to the other:
a patch, another release, a demo, a version for another platform with the same compiler.

Usage: match_functions.py EXPORT_A EXPORT_B [--labels A_NAMES.tsv --out B_NAMES.tsv]
                          [--min 0.8] [--all] [--json]

  EXPORT_A, EXPORT_B   two Ghidra exports of one binary each (analysis/export/<binary>/, made
                       by scripts/ghidra-export.sh), with decomp.c and functions.tsv
  --labels FILE        your labels for build A (labels/names.tsv: address, label, evidence)
  --out FILE           write those labels for build B, at the matched addresses, with the
                       match as evidence. Review the file before you apply it (ghidra-run labels)
  --min R              the lowest similarity accepted for a fuzzy match (default 0.8)
  --all                also print the functions left unmatched

Months of naming functions belong to one build. When a patch or another release arrives, the
addresses move, but most functions stay the same. This tool finds them again in four passes:

1. exact      the pseudocode is the same once addresses and generated names are removed;
2. similar    the closest function by shared pieces of pseudocode, when it is clearly the closest;
3. calls      the callees of two matched functions, paired by their position in the call order;
4. repeat     passes 2 and 3 again, until nothing new matches.

Every fuzzy match is a lead. Check the ones your port depends on. Ghidra's own Version
Tracking and BSim do this inside Ghidra with more detail (docs/working/ghidra.md); this tool
works on the text exports, with no database and no Ghidra session. Standard library only.
"""
import difflib
import hashlib
import json
import os
import re
import sys

HEADER = re.compile(r'^// ==== ([0-9a-fA-F:]+) (.*)$')
COMMENT = re.compile(r'/\*.*?\*/', re.S)
GENERATED = [
    (re.compile(r'\b(FUN|DAT|LAB|PTR|UNK|switchD|caseD|joined_r0x|code_r0x|thunk_FUN)_[0-9a-fA-F_]+\b'), r'\1'),
    (re.compile(r'\b_?DAT_[0-9a-fA-F_]+\b'), 'DAT'),
    (re.compile(r'\b(s|u)_([A-Za-z0-9_]*?)_[0-9a-fA-F]{6,}\b'), r'\1_\2'),
    (re.compile(r'\b0x[0-9a-fA-F]{6,}\b'), 'ADDR'),
    (re.compile(r'\b[0-9a-fA-F]{8,}\b'), 'ADDR'),
]
TOKEN = re.compile(r'[A-Za-z_][A-Za-z0-9_]*|0x[0-9a-fA-F]+|\d+|\S')
CALL = re.compile(r'\b([A-Za-z_][A-Za-z0-9_]*)\s*\(')
KEYWORDS = {'if', 'while', 'for', 'switch', 'return', 'sizeof', 'do'}


class Export:
    def __init__(self, folder):
        self.folder = folder
        self.name = os.path.basename(os.path.normpath(folder))
        self.funcs = {}                  # address -> name
        self.size = {}
        tsv = os.path.join(folder, 'functions.tsv')
        if os.path.exists(tsv):
            for line in open(tsv, encoding='utf-8', errors='replace'):
                parts = line.rstrip('\n').split('\t')
                if len(parts) >= 3 and 'thunk' not in parts[3:]:
                    self.size[parts[0].lower()] = int(parts[1] or 0)
        self.body = {}
        addr = None
        lines = []
        for line in open(os.path.join(folder, 'decomp.c'), encoding='utf-8', errors='replace'):
            m = HEADER.match(line)
            if m:
                if addr:
                    self.body[addr] = ''.join(lines)
                addr, lines = m.group(1).lower(), []
                self.funcs[addr] = m.group(2).strip()
            elif addr:
                lines.append(line)
        if addr:
            self.body[addr] = ''.join(lines)
        self.by_name = {n: a for a, n in self.funcs.items()}
        names = set(self.funcs.values())
        self.tokens, self.calls, self.digest = {}, {}, {}
        for a, text in self.body.items():
            self.calls[a] = [self.by_name[c] for c in CALL.findall(COMMENT.sub('', text))
                             if c in self.by_name and c not in KEYWORDS and self.by_name[c] != a]
            norm = COMMENT.sub('', text)
            for pattern, repl in GENERATED:
                norm = pattern.sub(repl, norm)
            toks = ['FN' if t in names else t for t in TOKEN.findall(norm)]
            self.tokens[a] = toks
            self.digest[a] = hashlib.sha1(' '.join(toks).encode()).hexdigest()


def shingles(tokens, n=4):
    return {hash(tuple(tokens[i:i + n])) for i in range(max(1, len(tokens) - n + 1))}


def similarity(ta, tb):
    sm = difflib.SequenceMatcher(None, ta, tb, autojunk=False)
    if sm.real_quick_ratio() < 0.5 or sm.quick_ratio() < 0.5:
        return 0.0
    return sm.ratio()


def match(a, b, floor):
    pairs, used = {}, set()

    def take(x, y, how, score):
        pairs[x] = (y, how, round(score, 3))
        used.add(y)

    # 1. exact: a normalised body that is unique in both builds.
    count_a, count_b = {}, {}
    for x, d in a.digest.items():
        count_a.setdefault(d, []).append(x)
    for y, d in b.digest.items():
        count_b.setdefault(d, []).append(y)
    for d, xs in count_a.items():
        ys = count_b.get(d, [])
        if len(xs) == 1 and len(ys) == 1 and len(a.tokens[xs[0]]) > 8:
            take(xs[0], ys[0], 'exact', 1.0)

    # An index of B's pieces, without the ones shared by many functions.
    index = {}
    shing_b = {y: shingles(t) for y, t in b.tokens.items()}
    for y, sh in shing_b.items():
        for s in sh:
            index.setdefault(s, []).append(y)
    index = {s: ys for s, ys in index.items() if len(ys) <= 20}

    changed = True
    while changed:
        changed = False
        # 2. similar: the clearly closest function by shared pieces, then by sequence.
        for x, toks in a.tokens.items():
            if x in pairs or len(toks) < 12:
                continue
            votes = {}
            for s in shingles(toks):
                for y in index.get(s, ()):
                    if y not in used:
                        votes[y] = votes.get(y, 0) + 1
            best = sorted(votes, key=votes.get, reverse=True)[:5]
            scored = sorted(((similarity(toks, b.tokens[y]), y) for y in best), reverse=True)
            if scored and scored[0][0] >= floor and (len(scored) == 1 or scored[0][0] - scored[1][0] >= 0.05):
                take(x, scored[0][1], 'similar', scored[0][0])
                changed = True
        # 3. calls: callees of matched pairs, by position, when both lists have the same length.
        for x, (y, _, _) in list(pairs.items()):
            ca, cb = a.calls.get(x, []), b.calls.get(y, [])
            if len(ca) != len(cb):
                continue
            for cx, cy in zip(ca, cb):
                if cx in pairs or cy in used:
                    continue
                s = similarity(a.tokens[cx], b.tokens[cy]) if a.tokens.get(cx) and b.tokens.get(cy) else 0
                if s >= 0.5:
                    take(cx, cy, 'calls', s)
                    changed = True
    return pairs


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    a, b = Export(argv[0]), Export(argv[1])
    pairs = match(a, b, float(opt('--min', '0.8')))
    rows = [{'a': x, 'a_name': a.funcs[x], 'b': y, 'b_name': b.funcs[y], 'how': how, 'score': s}
            for x, (y, how, s) in sorted(pairs.items())]
    if opt('--labels'):
        out = opt('--out') or 'names-b.tsv'
        written = 0
        with open(opt('--labels'), encoding='utf-8') as src, open(out, 'w', encoding='utf-8') as dst:
            for line in src:
                if not line.strip() or line.startswith('#'):
                    dst.write(line)
                    continue
                parts = line.rstrip('\n').split('\t')
                addr = parts[0].lower().replace('0x', '').replace('fun_', '')
                if addr in pairs:
                    y, how, s = pairs[addr]
                    evidence = f'matched from {a.name} {addr} ({how} {s})'
                    if len(parts) > 2 and parts[2]:
                        evidence += f'; {parts[2]}'
                    dst.write(f'{y}\t{parts[1]}\t{evidence}\n')
                    written += 1
        print(f'{written} labels written to {out}')
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        print(f'{r["a"]} {r["a_name"][:30]:30s} -> {r["b"]} {r["b_name"][:30]:30s} {r["how"]:8s} {r["score"]}')
    if '--all' in argv:
        for x in sorted(set(a.funcs) - set(pairs)):
            print(f'{x} {a.funcs[x][:30]:30s} -> (no match)')
    by = {}
    for r in rows:
        by[r['how']] = by.get(r['how'], 0) + 1
    print(f'{len(rows)} of {len(a.funcs)} functions matched ({", ".join(f"{k} {v}" for k, v in by.items())}); '
          f'build B has {len(b.funcs)}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
