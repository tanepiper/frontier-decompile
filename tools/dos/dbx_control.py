#!/usr/bin/env python3
"""Drive DOSBox-X's built-in debugger from a script, through its debugger control channel.

DOSBox-X (2026.10 and later, built with the debugger) connects to 127.0.0.1:<mcp_server> when
the [dosbox] setting mcp_server is set, and accepts one debugger command at a time over a
line-based text protocol (README.debugger in DOSBox-X). The agent-facing side of the same
channel is the dosbox-x-mcp-server program (registered in .mcp.json as "dosbox"); this module
is the script-facing side, for repeatable ground-truth runs and tests.

    from dbx_control import Debugger
    with Debugger(port=0) as dbx:                      # listen first, on a free port...
        dbx.launch([os.path.abspath('scripts/dosbox-run.sh'), '--break', '--mcp', str(dbx.port), GAME, 'GAME.EXE'])
        dbx.wait_connected()                            # DOSBox-X connected
        dbx.wait_break()                                # DEBUGBOX stopped at the first instruction
        dbx.exec('BPINT 21 3D')                         # any debugger command; returns its output lines
        dbx.run()                                       # resume
        regs = dbx.wait_break()                         # stopped again: registers (EV), one step taken
        data = dbx.memdump('DS', 0x12DE, 4)             # bytes, through MEMDUMPBIN
    Debugger commands that draw into the debugger window (CPU, D) return no text: use EV for
    registers and MEMDUMPBIN for memory (these helpers do).

Command line: dbx_control.py --port P [--timeout S] COMMAND ...   waits for DOSBox-X to connect,
runs each debugger COMMAND (for example BREAK "EXEC CPU" "EXEC D 0040:0000"), prints the replies.

Addresses are what the running program uses: real-mode segments, or the DPMI host's selectors
for a protected-mode program, which are NOT Ghidra's 1000/1008 numbering. Find a segment's
selector with the debugger's LDT command, or match its bytes (see docs/DOS.md).
Use port=0 (a free port) for scripts and tests: a DOSBox-X left from another run, or another
session's, may still be trying to connect to a fixed port, and the listener would accept it.
close() ends the whole process group that launch() started.
Standard library only. It does not start DOSBox-X itself unless you call launch().
"""
import os
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time



class Debugger:
    def __init__(self, port=58993, timeout=30.0, workdir=None):
        self.port, self.timeout = port, timeout
        self.workdir = workdir or tempfile.mkdtemp(prefix='dbx-')
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(('127.0.0.1', port))
        self.listener.listen(1)
        self.port = self.listener.getsockname()[1]        # port=0 picks a free port
        self.conn = None
        self.buf = b''
        self.next_id = 1
        self.process = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        for s in (self.conn, self.listener):
            try:
                if s:
                    s.close()
            except OSError:
                pass
        self.stop()

    def stop(self, grace=2.0):
        """End DOSBox-X and every process launch() started (the script wrapper, script, dosbox-x).

        Killing only the wrapper leaves dosbox-x running. Stopped in the debugger, it never
        reaches -time-limit, and it reconnects to the next listener on the same port.
        """
        if not self.process or self.process.poll() is not None:
            return
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(self.process.pid, sig)
            except (ProcessLookupError, PermissionError):
                return
            try:
                self.process.wait(grace)
                return
            except subprocess.TimeoutExpired:
                pass

    def launch(self, argv, **kw):
        """Start DOSBox-X (usually scripts/dosbox-run.sh --mcp PORT ...) in the background.

        It runs in its own process group, so close() can end all of it.
        """
        kw.setdefault('cwd', self.workdir)
        kw.setdefault('start_new_session', True)
        self.process = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **kw)
        return self.process

    def wait_connected(self, timeout=None):
        self.listener.settimeout(timeout or self.timeout)
        self.conn, _ = self.listener.accept()
        self.conn.settimeout(timeout or self.timeout)
        return self.request('PING')

    def _line(self):
        while b'\n' not in self.buf:
            chunk = self.conn.recv(65536)
            if not chunk:
                raise ConnectionError('DOSBox-X closed the control connection')
            self.buf += chunk
        line, self.buf = self.buf.split(b'\n', 1)
        return line.decode('latin-1').rstrip('\r')

    def request(self, what):
        """Send 'PING', 'BREAK' or 'EXEC <command>'; return the reply's output lines."""
        rid = self.next_id
        self.next_id += 1
        self.conn.sendall(f'REQ {rid} {what}\n'.encode('latin-1'))
        while True:
            head = self._line()
            if head.startswith(f'BEGIN {rid} '):
                break
        ok = head.endswith('OK')
        out = []
        while True:
            line = self._line()
            if line == f'END {rid}':
                break
            out.append(line)
        if not ok:
            raise RuntimeError(f'debugger refused {what!r}: ' + ' / '.join(out))
        return out

    def exec(self, command):
        return self.request(f'EXEC {command}')

    def brk(self):
        return self.request('BREAK')

    def run(self):
        return self.exec('RUN')

    def registers(self, names=('EAX', 'EBX', 'ECX', 'EDX', 'ESI', 'EDI', 'EBP', 'ESP', 'EIP',
                               'CS', 'DS', 'ES', 'SS', 'FS', 'GS')):
        """Register values as a dict, read with the debugger's EV command (no side effects)."""
        out = self.exec('EV ' + ' '.join(names))
        values = out[-1].split() if out else []
        return {n: int(v, 16) for n, v in zip(names, values)}

    def wait_break(self, timeout=None, poll=0.2):
        """Wait until execution stops in the debugger (a breakpoint, or DEBUGBOX's first stop).

        It polls with STEPIN, which only works while stopped, so it executes ONE instruction
        once stopped (at an INT breakpoint, that enters the interrupt handler). Returns the
        registers after that step, plus 'STOP_CS'/'STOP_IP': where execution had stopped.
        """
        end = time.time() + (timeout or self.timeout)
        while time.time() < end:
            try:
                line = ' '.join(self.exec('STEPIN'))
            except RuntimeError:
                time.sleep(poll)
                continue
            regs = self.registers()
            m = re.search(r'CS:IP=([0-9A-Fa-f]{4}):([0-9A-Fa-f]{4,8})', line)
            if m:
                regs['STEP_CS'], regs['STEP_IP'] = int(m.group(1), 16), int(m.group(2), 16)
            ax = re.search(r'AX=([0-9A-Fa-f]{4})', line)
            if ax:
                regs['STEP_AX'] = int(ax.group(1), 16)
            return regs
        raise TimeoutError('the debugger did not stop (breakpoint not reached?)')

    def memdump(self, seg, off, length):
        """Bytes at seg:off (seg may be a register name such as DS), through MEMDUMPBIN.

        DOSBox-X writes MEMDUMP.BIN into its working directory: `workdir`, which launch()
        uses as the working directory of the process it starts.
        """
        path = os.path.join(self.workdir, 'MEMDUMP.BIN')
        if os.path.exists(path):
            os.remove(path)
        seg = seg if isinstance(seg, str) else f'{seg:X}'
        self.exec(f'MEMDUMPBIN {seg}:{off:X} {length:X}')
        with open(path, 'rb') as f:
            return f.read()


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        sys.exit(2)
    port, timeout = 58993, 30.0
    while argv and argv[0].startswith('--'):
        if argv[0] == '--port':
            port, argv = int(argv[1]), argv[2:]
        elif argv[0] == '--timeout':
            timeout, argv = float(argv[1]), argv[2:]
        else:
            sys.exit(f'unknown option {argv[0]}')
    with Debugger(port, timeout) as dbx:
        print(f'waiting for DOSBox-X on 127.0.0.1:{port} (start it with mcp_server={port})', file=sys.stderr)
        dbx.wait_connected()
        for cmd in argv:
            what = cmd if cmd.split()[0] in ('PING', 'BREAK', 'EXEC') else f'EXEC {cmd}'
            print(f'> {what}')
            for line in dbx.request(what):
                print(line)


if __name__ == '__main__':
    main(sys.argv[1:])
