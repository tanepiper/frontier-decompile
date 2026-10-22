#!/usr/bin/env python3
"""Checks of a CD's file systems, for discinfo.py: sectors that hold data no file system names,
the system area, files that only one directory tree lists, and file dates later than the
volume's own date.

    from checks import unused_data, system_area, tree_differences, late_dates

A mastered disc normally accounts for every sector: the system area (sectors 0 to 15), the
volume descriptors, the path tables, the directories and the files. Data outside all of them
can be a file left over from development, a second copy, padding written by the mastering
tool, or data a protection or the game reads by sector number. The system area is empty on a
PC disc, and holds the console's licence and boot data on PlayStation, Saturn and Mega-CD
discs. A file listed in the ISO 9660 tree but missing from the Joliet tree is hidden from
Windows. Standard library only.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'triage'))
from filetypes import kind_of  # noqa: E402


def runs(numbers):
    """Consecutive runs in a sorted list: [(first, last), ...]."""
    out = []
    for n in numbers:
        if out and n == out[-1][1] + 1:
            out[-1][1] = n
        else:
            out.append([n, n])
    return [tuple(r) for r in out]


def unused_data(track, volumes, limit=200000):
    """Runs of sectors inside the data track that hold non-zero bytes and belong to no file
    system structure. Each run: (first sector, last sector, a guess at the kind of data)."""
    used = set()
    base = 0
    for v in volumes:
        if v.root is not None and v.track is track:
            used |= v.used()
            base = v.base
    if not used:
        return []
    spare = []
    for index in range(min(track.frames, limit)):
        lba = base + index
        if lba in used or lba < 16:
            continue
        try:
            data = track.read(index)
        except EOFError:
            break
        if any(data):
            spare.append(lba)
    out = []
    for first, last in runs(spare):
        head = track.read(first - base)
        out.append((first, last, kind_of(head + bytes(0x8010 - len(head)), len(head))))
    return out


def system_area(track):
    """What sectors 0 to 15 hold, when they are not empty."""
    try:
        data = track.read(0, 16)
    except EOFError:
        return None
    if not any(data):
        return None
    marks = [(b'Licensed  by', 'PlayStation licence text'), (b'SEGA SEGASATURN', 'Sega Saturn boot header'),
             (b'SEGADISCSYSTEM', 'Sega Mega-CD boot header'), (b'SEGA SEGAKATANA', 'Sega Dreamcast boot header'),
             (b'PC Engine CD-ROM', 'PC Engine CD-ROM header'), (b'\x55\xaa', 'a PC boot signature')]
    for mark, what in marks:
        if mark in data:
            return what
    return f'{sum(1 for i in range(16) if any(data[i * 2048:(i + 1) * 2048]))} non-empty sectors'


def tree_differences(volumes):
    """Files whose sectors one tree lists and another does not, by disc sector."""
    trees = [v for v in volumes if v.root is not None]
    out = []
    for a in trees:
        for b in trees:
            if a is b:
                continue
            in_b = {e.lba for e in b.entries() if not e.directory}
            only = [e.path for e in a.entries() if not e.directory and e.size and e.lba not in in_b]
            if only:
                out.append((a.kind, b.kind, only))
    return out


def late_dates(volume):
    """Files dated after the volume was created. A mastered disc's files are older than the
    volume, so a later date suggests the dates were changed, or the clock was wrong."""
    created = (volume.info.get('created') or '')[:19]
    if not created or volume.root is None:
        return []
    return [e.path for e in volume.entries() if e.date and e.date[:19] > created]
