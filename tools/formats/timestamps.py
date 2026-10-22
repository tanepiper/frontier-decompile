#!/usr/bin/env python3
"""Decode a number as every common kind of timestamp, or find timestamps inside a file.

Usage: timestamps.py VALUE                 a number (decimal, or hex with 0x), every reading
       timestamps.py --bytes HEX           raw bytes, read little-endian and big-endian
       timestamps.py --scan FILE [--from YEAR] [--to YEAR] [--kinds K,K] [--max N]

Kinds:
  unix      seconds since 1970-01-01 (C time_t; also the PE header's TimeDateStamp)
  unix-ms   milliseconds since 1970
  dos       a FAT date and time packed in 32 bits: date in the high word, time in the low
            word (directory entries, ZIP headers, INT 21h AX=5700h), 2-second steps
  dos-swap  the same with the words the other way round
  filetime  Windows FILETIME: 100-nanosecond steps since 1601-01-01
  hfs       Mac OS classic: seconds since 1904-01-01 (local time)
  ole       an OLE automation date: a double, days since 1899-12-30 (Visual Basic, Excel)
  cocoa     seconds since 2001-01-01 (later Mac and iOS games)

A save game usually records when it was saved, and the executable records when it was
linked. Finding these fields helps to decode a save format (one field fewer to explain, and
a known value to check against), and to date a release. --scan reads every offset, so most
readings are noise: keep the year range narrow (the game's release years), and trust a hit
when a second save, written later, has a later value at the same offset. Standard library only.
"""
import datetime
import struct
import sys

UTC = datetime.timezone.utc
EPOCH = datetime.datetime(1970, 1, 1, tzinfo=UTC)


def unix(n):
    return EPOCH + datetime.timedelta(seconds=n)


def dos(n):
    date, time = n >> 16, n & 0xFFFF
    return datetime.datetime(1980 + (date >> 9), (date >> 5) & 15, date & 31,
                             time >> 11, (time >> 5) & 63, (time & 31) * 2)


KINDS = {
    'unix': (4, lambda n: unix(n)),
    'unix-ms': (8, lambda n: unix(n / 1000)),
    'dos': (4, dos),
    'dos-swap': (4, lambda n: dos(((n & 0xFFFF) << 16) | (n >> 16))),
    'filetime': (8, lambda n: datetime.datetime(1601, 1, 1, tzinfo=UTC) + datetime.timedelta(microseconds=n // 10)),
    'hfs': (4, lambda n: datetime.datetime(1904, 1, 1) + datetime.timedelta(seconds=n)),
    'cocoa': (4, lambda n: datetime.datetime(2001, 1, 1, tzinfo=UTC) + datetime.timedelta(seconds=n)),
}


def readings(n):
    out = []
    for kind, (_, f) in KINDS.items():
        try:
            out.append((kind, f(n)))
        except (ValueError, OverflowError):
            pass
    return out


def show(n, label=''):
    print(f'{label}{n} (0x{n:x})')
    for kind, when in readings(n):
        print(f'  {kind:9s} {when.isoformat(sep=" ")}')


def ole_of(raw8):
    days = struct.unpack('<d', raw8)[0]
    if days != days or not 1 <= days < 2958465:     # 1899-12-31 to 9999-12-31
        return None
    return datetime.datetime(1899, 12, 30) + datetime.timedelta(days=days)


def scan(data, lo, hi, kinds, limit):
    hits = 0
    for at in range(len(data) - 3):
        for kind in kinds:
            if kind == 'ole':
                if at + 8 > len(data):
                    continue
                when = ole_of(data[at:at + 8])
            else:
                size, f = KINDS[kind]
                if at + size > len(data):
                    continue
                n = int.from_bytes(data[at:at + size], 'little')
                if kind.startswith('dos') and (n & 0xFFFF == 0 or n >> 16 == 0):
                    continue            # a zero time or date word matches far too often
                try:
                    when = f(n)
                except (ValueError, OverflowError):
                    continue
            if when is not None and lo <= when.year <= hi:
                print(f'0x{at:08x}  {kind:9s} {when.isoformat(sep=" ")}')
                hits += 1
                if hits >= limit:
                    print(f'(stopped after {limit} hits: narrow --from/--to or --kinds)')
                    return


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    if argv[0] == '--scan':
        with open(argv[1], 'rb') as f:
            data = f.read()
        kinds = opt('--kinds', 'unix,dos,dos-swap,filetime,hfs,ole').split(',')
        scan(data, int(opt('--from', '1980')), int(opt('--to', '2030')), kinds, int(opt('--max', '200')))
        return 0
    if argv[0] == '--bytes':
        raw = bytes.fromhex(argv[1].replace(' ', ''))
        show(int.from_bytes(raw, 'little'), 'little-endian: ')
        show(int.from_bytes(raw, 'big'), 'big-endian:    ')
        if len(raw) == 8:
            when = ole_of(raw)
            if when:
                print(f'  ole       {when.isoformat(sep=" ")}')
        return 0
    show(int(argv[0], 0))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
