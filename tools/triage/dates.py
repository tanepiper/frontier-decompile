#!/usr/bin/env python3
"""Find dates and times stored in a binary file: a save's time, an archive entry's date.

Usage: dates.py FILE --near YYYY-MM-DD[THH:MM] [--days N] [--format NAME ...]

Every offset is read in each format below. An offset is printed when its value falls within
N days (default 1) of the date given with --near. Save the game, note the time, and look for
it. A date is needed because, over a wide range, almost any four bytes read as some date.

Formats:
  unix32    seconds since 1970-01-01, 32-bit little-endian (C time_t until 64-bit builds)
  unix64    seconds since 1970-01-01, 64-bit little-endian
  unixms    milliseconds since 1970-01-01, 64-bit little-endian (Java, JavaScript, .NET Unix time)
  filetime  100 ns units since 1601-01-01, 64-bit little-endian (Windows FILETIME)
  dos       a 16-bit time then a 16-bit date, both little-endian (FAT, ZIP, many DOS games);
            the year counts from 1980 and seconds are stored halved
  hfs       seconds since 1904-01-01, 32-bit big-endian (classic Mac OS, HFS)
  ole       days since 1899-12-30 as a 64-bit float (Windows OLE/COM DATE, Excel)
  iso9660   7 bytes: years since 1900, month, day, hour, minute, second, offset from GMT in
            15-minute units (CD directory records)
  systemtime  eight 16-bit values: year, month, weekday, day, hour, minute, second, ms
            (Windows SYSTEMTIME, often written whole into a save)

The value a game stores is local time or UTC; try --days 1 to cover both.
"""
import argparse
import datetime as dt
import struct
import sys

UTC = dt.timezone.utc
E1970 = dt.datetime(1970, 1, 1, tzinfo=UTC)


def at(base, **kw):
    try:
        return base + dt.timedelta(**kw)
    except (OverflowError, ValueError):
        return None


def unix32(b, i):
    return at(E1970, seconds=struct.unpack_from('<I', b, i)[0]) if i + 4 <= len(b) else None


def unix64(b, i):
    return at(E1970, seconds=struct.unpack_from('<q', b, i)[0]) if i + 8 <= len(b) else None


def unixms(b, i):
    return at(E1970, milliseconds=struct.unpack_from('<q', b, i)[0]) if i + 8 <= len(b) else None


def filetime(b, i):
    if i + 8 > len(b):
        return None
    return at(dt.datetime(1601, 1, 1, tzinfo=UTC), microseconds=struct.unpack_from('<Q', b, i)[0] // 10)


def dos(b, i):
    if i + 4 > len(b):
        return None
    t, d = struct.unpack_from('<HH', b, i)
    try:
        return dt.datetime(1980 + (d >> 9), (d >> 5) & 15, d & 31, t >> 11, (t >> 5) & 63, (t & 31) * 2, tzinfo=UTC)
    except ValueError:
        return None


def hfs(b, i):
    return at(dt.datetime(1904, 1, 1, tzinfo=UTC), seconds=struct.unpack_from('>I', b, i)[0]) if i + 4 <= len(b) else None


def ole(b, i):
    if i + 8 > len(b):
        return None
    v = struct.unpack_from('<d', b, i)[0]
    if not (v == v) or not 0 < v < 100000:
        return None
    return at(dt.datetime(1899, 12, 30, tzinfo=UTC), days=v)


def iso9660(b, i):
    if i + 7 > len(b):
        return None
    y, mo, d, h, mi, s, off = struct.unpack_from('<6Bb', b, i)
    try:
        return dt.datetime(1900 + y, mo, d, h, mi, s, tzinfo=dt.timezone(dt.timedelta(minutes=15 * off)))
    except ValueError:
        return None


def systemtime(b, i):
    if i + 16 > len(b):
        return None
    y, mo, wd, d, h, mi, s, ms = struct.unpack_from('<8H', b, i)
    if wd > 6 or ms > 999:
        return None
    try:
        return dt.datetime(y, mo, d, h, mi, s, ms * 1000, tzinfo=UTC)
    except ValueError:
        return None


FORMATS = {'unix32': unix32, 'unix64': unix64, 'unixms': unixms, 'filetime': filetime, 'dos': dos,
           'hfs': hfs, 'ole': ole, 'iso9660': iso9660, 'systemtime': systemtime}


def scan(data, near, days=1.0, formats=None):
    formats = formats or list(FORMATS)
    lo, hi = near - dt.timedelta(days=days), near + dt.timedelta(days=days)
    for i in range(len(data)):
        for name in formats:
            when = FORMATS[name](data, i)
            if when is not None and lo <= when <= hi:
                yield i, name, when


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('file')
    ap.add_argument('--near', required=True, help='a date the file should hold, YYYY-MM-DD or YYYY-MM-DDTHH:MM')
    ap.add_argument('--days', type=float, default=1.0)
    ap.add_argument('--format', action='append', choices=sorted(FORMATS))
    a = ap.parse_args(argv)
    near = dt.datetime.fromisoformat(a.near).replace(tzinfo=UTC)
    data = open(a.file, 'rb').read()
    found = 0
    for i, name, when in scan(data, near, a.days, a.format):
        print(f'{i:#08x}  {name:<10} {when.isoformat(sep=" ")}')
        found += 1
    print(f'{found} candidates')


if __name__ == '__main__':
    main(sys.argv[1:])
