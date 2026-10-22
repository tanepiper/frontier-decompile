#!/usr/bin/env python3
"""Find the key of a simply obfuscated data file: XOR with one byte or a repeating key, adding
a byte, a key that steps by a fixed amount each byte, or each byte XORed with the one before.

Usage: xorscan.py FILE [--offset N] [--length N] [--known HEX@OFFSET] [--top N]
       xorscan.py FILE --apply SCHEME -o OUT

  --offset, --length   scan only part of the file (a header's encrypted body, one entry)
  --known HEX@OFFSET   bytes you know are there once decoded (a magic number, a name, zeros),
                       for example 52494646@0 for "RIFF" at the start
  --top N              how many candidates to show (default 8)
  --apply SCHEME       decode the whole file with one result line's scheme, such as
                       xor:5a  xor-key:1f2e3d  add:80  step:5a+3  chain:00  rol:3

Many games hide their text, scripts or tables with a simple XOR so that people cannot read
them in a file viewer. The schemes are weak, and the decoded bytes give the key away: game
data has many zero bytes and much ASCII. Each candidate is scored by how much of the result
is zeros or printable text, with a bonus when a known format's signature appears. Read the
preview, then confirm the scheme in the code that reads the file (re-format, re-trace) before
porting it: the code may also compress, or change the key per file or per entry.

If nothing scores well, the file may be compressed (filetypes.py shows its entropy) or use
a real cipher. Then find the decoder in the code. Decoded files are game material: write
them into target/ or another folder that git ignores. Standard library only.
"""
import sys

MAGICS = [b'RIFF', b'MZ', b'PK\x03\x04', b'FORM', b'MThd', b'BM', b'\x89PNG', b'GIF8', b'OggS',
          b'<?xml', b'IWAD', b'PWAD', b'PACK', b'Creative', b'SQLite']


def score(d):
    if not d:
        return 0.0
    good = sum(1 for b in d if b == 0 or 32 <= b < 127 or b in (9, 10, 13))
    s = good / len(d)
    if any(d.startswith(m) for m in MAGICS):
        s += 0.5
    return s


def decode(data, scheme, start=0):
    """Apply a scheme. `start` is the file offset of data[0], for schemes whose key depends on
    the position (a repeating key, a stepping key)."""
    kind, _, arg = scheme.partition(':')
    out = bytearray(len(data))
    if kind == 'xor':
        k = int(arg, 16)
        for i, b in enumerate(data):
            out[i] = b ^ k
    elif kind == 'xor-key':
        key = bytes.fromhex(arg)
        for i, b in enumerate(data):
            out[i] = b ^ key[(start + i) % len(key)]
    elif kind == 'add':
        k = int(arg, 16)
        for i, b in enumerate(data):
            out[i] = (b + k) & 0xFF
    elif kind == 'step':
        k0, step = (int(x, 16) for x in arg.split('+'))
        for i, b in enumerate(data):
            out[i] = b ^ ((k0 + step * (start + i)) & 0xFF)
    elif kind == 'chain':
        prev = int(arg, 16)
        for i, b in enumerate(data):
            out[i] = b ^ prev
            prev = b
    elif kind == 'rol':
        n = int(arg) % 8
        for i, b in enumerate(data):
            out[i] = ((b << n) | (b >> (8 - n))) & 0xFF
    else:
        raise SystemExit(f'unknown scheme {scheme}')
    return bytes(out)


def key_lengths(data, top=4, longest=32):
    """Likely repeating-key lengths: the length whose columns repeat values most often."""
    out = []
    for n in range(2, min(longest, len(data) // 8) + 1):
        coinc = 0.0
        for c in range(n):
            col = data[c::n]
            counts = {}
            for b in col:
                counts[b] = counts.get(b, 0) + 1
            coinc += sum(v * (v - 1) for v in counts.values()) / max(1, len(col) * (len(col) - 1))
        out.append((coinc / n, n))
    out.sort(reverse=True)
    return [n for _, n in out[:top]]


def repeating_key(data, n, start):
    """Each key byte: the value that turns its column's most common byte into zero, or into a
    space when that gives more printable text."""
    key = bytearray(n)
    for c in range(n):
        col = data[c::n]
        counts = {}
        for b in col:
            counts[b] = counts.get(b, 0) + 1
        common = max(counts, key=counts.get)
        best = max((common, common ^ 0x20), key=lambda k: score(bytes(b ^ k for b in col)))
        key[(start + c) % n] = best
    for p in range(1, n):                     # a multiple of the real length: keep one period
        if n % p == 0 and all(key[i] == key[i % p] for i in range(n)):
            return bytes(key[:p])
    return bytes(key)


def candidates(data, start):
    out = []
    for k in range(256):
        out.append(f'xor:{k:02x}')
        out.append(f'add:{k:02x}')
        out.append(f'chain:{k:02x}')
    for n in range(1, 8):
        out.append(f'rol:{n}')
    # A key that steps: try each step with the first byte's best start value.
    for step in range(1, 256):
        best = max(range(256), key=lambda k0: score(decode(data[:256], f'step:{k0:02x}+{step:02x}', start)))
        out.append(f'step:{best:02x}+{step:02x}')
    for n in key_lengths(data):
        out.append(f'xor-key:{repeating_key(data, n, start).hex()}')
    return out


def from_known(data, start, known):
    hexpart, _, at = known.partition('@')
    plain, at = bytes.fromhex(hexpart), int(at, 0) - start
    cipher = data[at:at + len(plain)]
    if len(cipher) < len(plain):
        return []
    key = bytes(c ^ p for c, p in zip(cipher, plain))
    out = [f'xor:{key[0]:02x}'] if len(set(key)) == 1 else []
    for n in range(2, len(key) // 2 + 1):     # the shortest period that explains the bytes
        if all(key[i] == key[i % n] for i in range(len(key))):
            rot = (start + at) % n
            out.append(f'xor-key:{(key[-rot:] + key[:-rot]).hex() if rot else key[:n].hex()}')
            break
    out.append(f'add:{(plain[0] - cipher[0]) & 0xFF:02x}')
    return out


def preview(d):
    return ''.join(chr(b) if 32 <= b < 127 else '.' for b in d[:64])


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    with open(argv[0], 'rb') as f:
        whole = f.read()
    if '--apply' in argv:
        out = opt('-o')
        if not out:
            raise SystemExit('--apply needs -o OUT')
        with open(out, 'wb') as f:
            f.write(decode(whole, opt('--apply')))
        print(f'wrote {out}')
        return 0
    start = int(opt('--offset', '0'), 0)
    length = int(opt('--length', str(min(len(whole) - start, 1 << 16))), 0)
    data = whole[start:start + length]
    schemes = candidates(data, start)
    if '--known' in argv:
        schemes = from_known(whole, 0, opt('--known')) + schemes
    seen, rows = set(), []
    for s in schemes:
        if s in seen:
            continue
        seen.add(s)
        d = decode(data, s, start)
        rows.append((score(d), s, d))
    rows.sort(key=lambda r: (-r[0], len(r[1])))   # on a tie, the simpler scheme
    print(f'as stored  score {score(data):.2f}  {preview(data)}')
    for sc, s, d in rows[:int(opt('--top', '8'))]:
        print(f'{s:28s} score {sc:.2f}  {preview(d)}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
