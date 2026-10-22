#!/usr/bin/env python3
"""Trace calls to chosen functions in a running game, with Frida.

Usage: frida_trace.py (--spawn PROGRAM [ARG ...] | --attach PID_OR_NAME)
                      [--hook SPEC ...] [--watch SPEC:SIZE ...] [--rearm MS]
                      [--args N] [--seconds S] [--out FILE]

SPEC names one function, with an optional label after `=`:
  game.exe+0x1a2b30=rng_next     an offset in a module (the address Ghidra shows minus the base)
  engine.dll!Random_Next          an exported name
  libc.so.6!rand=rand             any module the process has loaded

Each call becomes one line, `label<TAB>module+offset`, where the offset is the call's return
address: the same format as tools/emu/call_trace.py. tools/verify/seqdiff.py then compares a
live run of the original with a run of the port. `--args N` adds the first N integer arguments
as `label(a, b)`. Only outermost calls are logged on each thread, so a traced function calling
another traced function logs one line, as the port would count one call.

--watch logs reads and writes of chosen memory: `game.exe+0x2a1000:256=table` or
`game.exe!g_units:4096`. Each access becomes `label:r+0x10<TAB>module+offset`, with the offset
into the watched block and the instruction that made it. Frida's memory access monitor reports
the first access to a page, then stops watching that page. The tracer starts it again
--rearm milliseconds (default 20) after each report, so further accesses to the same page in
that time are not logged. An instruction that reads and then writes (`add [x], 1`) shows as a
read. Use it to find which code touches a value. Use --hook to count calls.

The emulator traces one function with chosen inputs. This traces the whole game while you
play, on its real platform: Windows, Linux, macOS or Android, wherever Frida runs. Install it
with scripts/setup.sh --install (requirements-live.txt). On macOS, Frida can trace programs
you built yourself. Signed applications and system programs need System Integrity Protection
relaxed, or a copy without the hardened runtime. Some protected games detect Frida. Trace a
copy you own, for analysis only ([Legal](../../docs/LEGAL.md)).
"""
import argparse
import json
import sys
import threading
import time

import frida

AGENT = r"""
const hooks = %s;
const nargs = %d;
const watches = %s;
const rearm = %d;
const depth = {};
function where(addr) {
  const m = Process.findModuleByAddress(addr);
  return m ? m.name + '+0x' + addr.sub(m.base).toString(16) : addr.toString();
}
function resolve(h) {
  if (h.offset !== null) return Process.getModuleByName(h.module).base.add(h.offset);
  return Process.getModuleByName(h.module).getExportByName(h.symbol);
}
for (const h of hooks) {
  let target;
  try { target = resolve(h); } catch (e) { send({error: h.spec + ': ' + e.message}); continue; }
  Interceptor.attach(target, {
    onEnter(args) {
      const t = this.threadId;
      depth[t] = (depth[t] || 0) + 1;
      if (depth[t] !== 1) return;
      let text = h.label;
      if (nargs > 0) {
        const a = [];
        for (let i = 0; i < nargs; i++) a.push(args[i].toString(10));
        text += '(' + a.join(', ') + ')';
      }
      send({line: text + '\t' + where(this.returnAddress)});
    },
    onLeave() { depth[this.threadId] -= 1; }
  });
}
if (watches.length) {
  const ranges = [];
  for (const w of watches) {
    try { ranges.push({base: resolve(w), size: w.size, label: w.label}); }
    catch (e) { send({error: w.spec + ': ' + e.message}); }
  }
  let armed = false, pending = false;
  const arm = () => {
    pending = false;
    if (armed) { MemoryAccessMonitor.disable(); armed = false; }
    MemoryAccessMonitor.enable(ranges.map(r => ({base: r.base, size: r.size})), {
      onAccess(d) {
        try {
          const r = ranges[d.rangeIndex];
          const op = d.operation === 'write' ? 'w' : (d.operation === 'execute' ? 'x' : 'r');
          send({line: r.label + ':' + op + '+0x' + d.address.sub(r.base).toString(16) + '\t' + where(d.from)});
        } catch (e) { send({error: 'watch: ' + e.message}); }
        if (!pending) { pending = true; setTimeout(arm, rearm); }
      }
    });
    armed = true;
  };
  if (ranges.length) arm();
}
send({ready: hooks.length + watches.length});
"""


def parse_hook(spec):
    """'mod+0xOFF=label' or 'mod!symbol=label' into the agent's description."""
    body, _, label = spec.partition('=')
    if '!' in body:
        module, symbol = body.split('!', 1)
        return {'spec': spec, 'module': module, 'symbol': symbol, 'offset': None, 'label': label or symbol}
    if '+' in body:
        module, off = body.rsplit('+', 1)
        return {'spec': spec, 'module': module, 'symbol': None, 'offset': int(off, 16), 'label': label or body}
    raise SystemExit(f'--hook {spec}: write module+0xOFFSET or module!name')


def parse_watch(spec):
    """'mod+0xOFF:SIZE=label' or 'mod!symbol:SIZE=label'."""
    body, _, label = spec.partition('=')
    where, _, size = body.rpartition(':')
    if not where or not size:
        raise SystemExit(f'--watch {spec}: write module+0xOFFSET:SIZE or module!name:SIZE')
    w = parse_hook(where + ('=' + label if label else ''))
    w.update(spec=spec, size=int(size, 0))
    return w


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    target = ap.add_mutually_exclusive_group(required=True)
    target.add_argument('--spawn', nargs=argparse.REMAINDER, metavar='PROGRAM')
    target.add_argument('--attach')
    ap.add_argument('--hook', action='append', default=[])
    ap.add_argument('--watch', action='append', default=[])
    ap.add_argument('--rearm', type=int, default=20)
    ap.add_argument('--args', type=int, default=0)
    ap.add_argument('--seconds', type=float, default=0, help='stop after this long (default: when the program ends)')
    ap.add_argument('--out', help='write the trace here (default: standard output)')
    # --spawn takes the rest of the line, so the other options must come first.
    a = ap.parse_args(argv)
    hooks = [parse_hook(h) for h in a.hook]
    watches = [parse_watch(w) for w in a.watch]
    if not hooks and not watches:
        raise SystemExit('give at least one --hook or --watch')

    device = frida.get_local_device()
    if a.spawn:
        pid = device.spawn(a.spawn)
    else:
        pid = int(a.attach) if a.attach.isdigit() else a.attach
    session = device.attach(pid)
    lines, done, problems = [], threading.Event(), []

    def on_message(msg, _data):
        if msg['type'] == 'error':
            problems.append(msg.get('description', str(msg)))
            return
        p = msg['payload']
        if 'line' in p:
            lines.append(p['line'])
        elif 'error' in p:
            problems.append(p['error'])

    script = session.create_script(AGENT % (json.dumps(hooks), a.args, json.dumps(watches), a.rearm))
    script.on('message', on_message)
    session.on('detached', lambda *_: done.set())
    script.load()
    if a.spawn:
        device.resume(pid)
    try:
        if a.seconds:
            done.wait(a.seconds)
        else:
            done.wait()
    except KeyboardInterrupt:
        pass
    time.sleep(0.2)
    try:
        session.detach()
    except frida.InvalidOperationError:
        pass
    for p in problems:
        print(f'warning: {p}', file=sys.stderr)
    out = open(a.out, 'w') if a.out else sys.stdout
    out.write('\n'.join(lines) + ('\n' if lines else ''))
    if a.out:
        out.close()
        print(f'{len(lines)} calls written to {a.out}', file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
