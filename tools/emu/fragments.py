#!/usr/bin/env python3
"""Capture fragments of a DOS program's main loop, before and after, from one headless run.

Usage: fragments.py PROGRAM OUT FRAMES SPEC [SPEC...] --frame-top SEG:OFF [options]

Runs PROGRAM (an NE or MZ executable) headless under dos_emu + dos_env, from its entry, for
FRAMES frames: a frame starts each time execution reaches --frame-top (an address in the main
loop). Each time execution reaches a fragment's START it saves the program's state, runs to the
fragment's END and saves it again. A port of the fragment loads `before`, runs, and must give
`after` (re-verify: "Capturing fragments of a main loop").

SPEC is NAME=START-END[,changed=OFFSET][,every=N][,max=K][,heap=1]. START and END are seg:off.
END may be several addresses joined by | (a fragment that can finish in more than one place):
  draw=1000:014f-1000:0183|1000:0185,heap=1   event=1000:0186-1000:0192,changed=a8
  changed=OFFSET  keep a run only if the fragment changed that data-segment word (hex), so
                  only frames where something happened (an event fired)
  every=N         keep one run in N
  max=K           stop saving that fragment after K runs
  heap=1          also save the heap blocks named by the --heap words (see .heap below)
Fragments are taken in the order the frame reaches them; one a frame does not reach is skipped
for that frame. A fragment's END may be the next fragment's START or the frame top; two
fragments cannot share a START.

Options:
  --frame-top SEG:OFF  the top of the main loop (required); counts frames
  --ds SEL             the data segment's selector (default: DS at the program's start, the NE
                       auto data segment)
  --screen OFF         DS offset of the word holding the selector of the screen being drawn
  --display OFF        DS offset of the word holding the selector of the VGA display (A000h)
  --heap OFF,OFF,...   DS offsets of words holding selectors of heap blocks (for heap=1)
  --root DIR           the game folder DOS sees (default: PROGRAM's folder); never written:
                       files the program writes go to OUT/overlay
  --start SEG:OFF      where to start instead of the entry (a routine that runs the loop)
  --program PATH       the program's DOS path, for its PSP (default C:\\GAME\\<name>)
  --date Y-M-D, --clock H:M:S   the fixed DOS date and time (default 1999-08-31 12:00:00)
  --max-steps N        instruction budget for the whole run (default 4 000 000 000)
Offsets and selectors are hex.

Outputs, for each kept run K of fragment NAME: OUT/NAME/KKKKK.before.EXT and .after.EXT, with
  .ds     the data segment (64 KB)          .ss     the stack segment (64 KB)
  .bp     BP (2 bytes, little endian: the main loop's frame in .ss)
  .scr    the screen segment (--screen)     .vga    the display segment (--display)
  .files  the open DOS files, one `handle<TAB>position<TAB>name` line each
  .heap   (heap=1) for each block, its selector word then 64 KB. `after` keeps only the blocks
          that changed
It prints `NAME: K of N runs saved in DIR` per fragment, and says so if the program exits
first. Evidence: measured.

It does not: compare anything (that is the port's test), save registers other than BP, follow
far pointers inside the saved segments, or run what dos_env does not emulate (the run stops
with the missing interrupt or import named). It captures one run; run it again with another
--clock or input to cover other paths.
"""
import argparse
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

SEG = 0x10000


def parse_spec(spec):
    name, rest = spec.split('=', 1)
    span, *opts = rest.split(',')
    start, end = span.split('-')
    f = {'name': name, 'start': start, 'ends': end.split('|'), 'changed': None, 'every': 1,
         'max': None, 'heap': 0, 'seen': 0, 'kept': 0}
    for o in opts:
        k, v = o.split('=')
        if k not in ('changed', 'every', 'max', 'heap'):
            sys.exit(f'{spec}: unknown option {k}')
        f[k] = int(v, 16) if k == 'changed' else int(v)
    return f


class Capture:
    def __init__(self, emu, env, a):
        self.emu, self.env, self.a = emu, env, a

    def word(self, ds, off):
        return struct.unpack_from('<H', ds, off)[0]

    def snapshot(self):
        emu, a = self.emu, self.a
        ds = emu.read((a.ds, 0), SEG)
        snap = {'ds': ds, 'ss': emu.read((emu.reg('ss'), 0), SEG), 'bp': struct.pack('<H', emu.reg('bp'))}
        if a.screen is not None:
            snap['scr'] = emu.read((self.word(ds, a.screen), 0), SEG)
        if a.display is not None:
            snap['vga'] = emu.read((self.word(ds, a.display), 0), SEG)
        snap['files'] = ''.join(f'{h}\t{f[2]}\t{f[0]}\n' for h, f in sorted(self.env.files.items())).encode()
        return snap

    def heap(self, ds):
        out = {}
        for at in self.a.heap:
            sel = self.word(ds, at)
            if sel and sel not in out:
                try:
                    out[sel] = self.emu.read((sel, 0), SEG)
                except Exception:      # a selector not yet given out: no block there
                    pass
        return out


def pack_heap(blocks):
    return b''.join(struct.pack('<H', sel) + data for sel, data in blocks.items())


def save(snap, path):
    for k, v in snap.items():
        with open(f'{path}.{k}', 'wb') as f:
            f.write(v)


def run(a):
    from dos_emu import DosEmu, ProgramStopped
    from dos_env import DosEnv
    frags = [parse_spec(s) for s in a.specs]
    starts = [f['start'].lower() for f in frags]
    if len(set(starts)) < len(starts):
        sys.exit('two fragments share a START: only the first would ever be captured')
    for f in frags:
        os.makedirs(os.path.join(a.out, f['name']), exist_ok=True)
    emu = DosEmu(a.program, max_steps=a.max_steps)
    y, mo, d = (int(x) for x in a.date.split('-'))
    h, mi, s = (int(x) for x in a.clock.split(':'))
    name = a.dos_path or 'C:\\GAME\\' + os.path.basename(a.program).upper()
    env = DosEnv(emu, root=a.root or os.path.dirname(os.path.abspath(a.program)),
                 overlay=os.path.join(a.out, 'overlay'), program=name, clock=(y, mo, d, h, mi, s, 0))
    if a.ds is None:
        a.ds = emu.regs['ds']
    cap = Capture(emu, env, a)
    targets = [a.frame_top] + [f['start'] for f in frags]
    n = 0
    try:
        emu.run_range(a.start or emu.entry, a.frame_top)
        while True:
            # The frame top, reached at the start of every frame.
            if n == a.frames:
                break
            n += 1
            emu.step()
            # Run to the next fragment start or the next frame top, whichever comes first.
            while (hit := emu.run_to_any(targets)) != 0:
                f = frags[hit - 1]
                before = cap.snapshot()
                hb = cap.heap(before['ds']) if f['heap'] else None
                emu.run_to_any(f['ends'])
                after = cap.snapshot()
                f['seen'] += 1
                at = f['changed']
                if at is not None and before['ds'][at:at + 2] == after['ds'][at:at + 2]:
                    continue
                if (f['seen'] - 1) % f['every'] or (f['max'] is not None and f['kept'] >= f['max']):
                    continue
                if f['heap']:
                    ha = cap.heap(after['ds'])
                    before['heap'] = pack_heap(hb)
                    after['heap'] = pack_heap({k: v for k, v in ha.items() if hb.get(k) != v})
                base = os.path.join(a.out, f['name'], f"{f['kept']:05d}")
                save(before, base + '.before')
                save(after, base + '.after')
                f['kept'] += 1
    except ProgramStopped as e:
        print(f'the program exited (code {e.args[0]}) after {n} frames')
    for f in frags:
        print(f"{f['name']}: {f['kept']} of {f['seen']} runs saved in {os.path.join(a.out, f['name'])}")


def hexint(v):
    return int(v, 16)


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        sys.exit(0 if argv else 2)
    p = argparse.ArgumentParser(add_help=False)
    p.add_argument('program')
    p.add_argument('out')
    p.add_argument('frames', type=int)
    p.add_argument('specs', nargs='+')
    p.add_argument('--frame-top', required=True)
    p.add_argument('--ds', type=hexint)
    p.add_argument('--screen', type=hexint)
    p.add_argument('--display', type=hexint)
    p.add_argument('--heap', type=lambda v: [int(x, 16) for x in v.split(',')], default=[])
    p.add_argument('--root')
    p.add_argument('--start')
    p.add_argument('--program', dest='dos_path')
    p.add_argument('--date', default='1999-08-31')
    p.add_argument('--clock', default='12:00:00')
    p.add_argument('--max-steps', type=int, default=4_000_000_000)
    run(p.parse_args(argv))


if __name__ == '__main__':
    main(sys.argv[1:])
