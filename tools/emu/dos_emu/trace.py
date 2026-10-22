"""Call traces: record the calls an emulated run makes to chosen routines (random draws), in
order, with their call sites.
Part of DosEmu (see __init__.py); the methods use its state."""
from unicorn import UC_HOOK_CODE

from .types import FarPtr


class Tracer:
    def trace(self, functions, args=0):
        """Record calls to chosen routines, in order, with their call sites (the random draws).

        `functions` maps an address (as for call) to a label. Only outermost calls are
        recorded, as tools/emu/call_trace.py does for x64. Pass `args=N` to record the first N
        word arguments (Pascal order: the first argument is deepest on the stack). Read
        `emu.traces`, or write them with `write_trace(path)` for tools/verify/seqdiff.py.
        """
        depth = {'until': 0}
        for where, label in functions.items():
            ptr = self.addr(where)
            def on_entry(uc, address, size, _, label=label):
                sp = self.reg('sp')
                if depth['until'] and sp < depth['until']:
                    return
                ss = self.reg('ss')
                ret = FarPtr(self.u16((ss, sp + 2)), self.u16((ss, sp)))
                text = label
                if args:
                    words = [self.u16((ss, sp + 4 + 2 * i)) for i in range(args)]
                    text += '(' + ', '.join(str(w) for w in reversed(words)) + ')'
                self.traces.append(f'{text}\t{ret!r}')
                depth['until'] = sp
                holder = []
                def on_return(uc2, a2, s2, _):
                    depth['until'] = 0
                    uc2.hook_del(holder[0])
                at = self.linear(*ret)
                holder.append(uc.hook_add(UC_HOOK_CODE, on_return, begin=at, end=at))
            at = self.linear(*ptr)
            self.uc.hook_add(UC_HOOK_CODE, on_entry, begin=at, end=at)

    def write_trace(self, path):
        with open(path, 'w') as f:
            f.write('\n'.join(self.traces) + '\n')


if __name__ == '__main__':
    print(__doc__)
