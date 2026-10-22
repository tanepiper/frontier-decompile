#!/usr/bin/env python3
"""Read the tracks and sectors of a CD image: a CUE sheet with its BIN files, a raw
BIN without a sheet, or a plain ISO.

    from image import open_image
    disc = open_image('target/cd/GAME.CUE')
    for t in disc.tracks: print(t.number, t.mode, t.lba, t.frames)
    data = disc.tracks[0].read(16)            # 2048 bytes of user data from sector 16

A CD sector holds 2352 bytes. A data sector puts 2048 bytes of user data inside that, after a
sync pattern and a header: at byte 16 in Mode 1, and at byte 24 in Mode 2 Form 1 (CD-ROM XA,
used by the PlayStation, the CD-i and many PC discs). Mode 2 Form 2 sectors hold 2324 bytes
(video and streamed sound) and no error correction. An audio sector is 2352 bytes of 16-bit
stereo samples at 44,100 Hz, so 75 sectors make one second. A cooked image (.iso, or
MODE1/2048 in a CUE sheet) keeps only the 2048 user bytes of each data sector.

Positions are frames (sectors). A CUE sheet gives them as minutes:seconds:frames from the
start of each FILE. `lba` below is the frame count from the start of the image, which is the
disc's own sector number for a single-session image. Standard library only.
"""
import os
import re

SYNC = b'\x00' + b'\xff' * 10 + b'\x00'
# CUE track modes: bytes per sector in the file, and where the user data starts.
MODES = {
    'AUDIO': (2352, 0), 'CDG': (2448, 0),
    'MODE1/2048': (2048, 0), 'MODE1/2352': (2352, 16),
    'MODE2/2048': (2048, 0), 'MODE2/2324': (2324, 0), 'MODE2/2336': (2336, 8),
    'MODE2/2352': (2352, 24), 'CDI/2336': (2336, 8), 'CDI/2352': (2352, 24),
}


def msf(text):
    """'mm:ss:ff' -> frames."""
    m, s, f = (int(x) for x in text.split(':'))
    return (m * 60 + s) * 75 + f


def to_msf(frames):
    return f'{frames // 4500:02d}:{frames // 75 % 60:02d}:{frames % 75:02d}'


class Track:
    def __init__(self, number, mode, path, offset, lba):
        self.number, self.mode, self.path = number, mode, path
        self.offset = offset            # byte offset of INDEX 01 in its file
        self.lba = lba                  # frames from the start of the image to INDEX 01
        self.pregap = 0                 # frames of INDEX 00 before INDEX 01 (in the file)
        self.frames = 0                 # length from INDEX 01, in frames
        self.size, self.data_at = MODES[mode]

    @property
    def audio(self):
        return self.mode == 'AUDIO'

    def raw(self, index, count=1):
        """The stored bytes of `count` sectors from sector `index` of this track."""
        with open(self.path, 'rb') as f:
            f.seek(self.offset + index * self.size)
            return f.read(self.size * count)

    def read(self, index, count=1):
        """The 2048-byte user data of `count` data sectors from sector `index` of this track."""
        out = bytearray()
        for i in range(count):
            s = self.raw(index + i)
            if len(s) < self.size:
                raise EOFError(f'track {self.number}: sector {index + i} is past the end of {self.path}')
            at = self.data_at
            # A raw sector says its own mode in byte 15; Mode 2 data then starts at 24.
            if self.size == 2352 and s[:12] == SYNC:
                at = 24 if s[15] == 2 else 16
            out += s[at:at + 2048]
        return bytes(out)

    def form2(self, index):
        """True when sector `index` is Mode 2 Form 2 (XA subheader submode bit 5)."""
        s = self.raw(index)
        if self.size == 2352 and s[:12] == SYNC and s[15] == 2:
            return bool(s[18] & 0x20)
        if self.size == 2336:
            return bool(s[2] & 0x20)
        return False

    def describe(self):
        return {'track': self.number, 'mode': self.mode, 'file': os.path.basename(self.path),
                'lba': self.lba, 'start': to_msf(self.lba), 'frames': self.frames,
                'length': to_msf(self.frames), 'pregap': self.pregap,
                'bytes_per_sector': self.size}


class Disc:
    def __init__(self, path, tracks, sessions=None, notes=None):
        self.path, self.tracks = path, tracks
        self.sessions = sessions or {}  # track number -> session number (REM SESSION)
        self.notes = notes or []


def parse_cue(path):
    base = os.path.dirname(os.path.abspath(path))
    tracks, notes, sessions = [], [], {}
    current_file = None
    file_start = 0                      # frames before this FILE in the image
    session = 1
    pending = {}                        # INDEX 00 of the current track
    by_file = []                        # (path, [tracks]) in order
    with open(path, encoding='latin-1') as f:
        lines = f.read().splitlines()
    for line in lines:
        words = line.strip().split()
        if not words:
            continue
        key = words[0].upper()
        if key == 'FILE':
            name = re.match(r'\s*FILE\s+"?(.*?)"?\s+\w+\s*$', line, re.I).group(1)
            p = os.path.join(base, name)
            if not os.path.exists(p):   # sheets made on Windows: try another case
                low = {n.lower(): n for n in os.listdir(base)}
                p = os.path.join(base, low.get(name.lower(), name))
            if by_file:
                prev = by_file[-1][0]
                size = by_file[-1][1][-1].size if by_file[-1][1] else 2352
                file_start += os.path.getsize(prev) // size if os.path.exists(prev) else 0
            current_file = p
            by_file.append((p, []))
        elif key == 'TRACK':
            pending = {'number': int(words[1]), 'mode': words[2].upper(), 'index0': None}
        elif key == 'INDEX' and pending:
            frames = msf(words[2])
            if int(words[1]) == 0:
                pending['index0'] = frames
            elif int(words[1]) == 1:
                group = by_file[-1][1]
                size = MODES[pending['mode']][0]
                if group:               # bytes from the previous track's INDEX 01
                    prev = group[-1]
                    offset = prev.offset + (frames - prev._at) * prev.size
                else:
                    offset = frames * size
                t = Track(pending['number'], pending['mode'], current_file, offset, file_start + frames)
                t._at = frames
                if pending['index0'] is not None:
                    t.pregap = frames - pending['index0']
                group.append(t)
                tracks.append(t)
                sessions[t.number] = session
                pending = {}
        elif key == 'REM' and len(words) >= 3 and words[1].upper() == 'SESSION':
            session = int(words[2])
        elif key == 'PREGAP':
            notes.append(f'track {pending.get("number")}: PREGAP {words[1]} is not stored in the file')
    # Lengths: to the next track in the same file, else to the end of the file.
    for p, group in by_file:
        for i, t in enumerate(group):
            if i + 1 < len(group):
                nxt = group[i + 1]
                t.frames = (nxt.offset - t.offset) // t.size - nxt.pregap
            elif os.path.exists(p):
                t.frames = (os.path.getsize(p) - t.offset) // t.size
            else:
                notes.append(f'{os.path.basename(p)} is missing')
    return Disc(path, tracks, sessions, notes)


def open_image(path):
    ext = os.path.splitext(path)[1].lower()
    if ext == '.cue':
        return parse_cue(path)
    with open(path, 'rb') as f:
        head = f.read(16)
    size = os.path.getsize(path)
    if head[:12] == SYNC:               # raw sectors without a sheet: one data track
        mode = 'MODE2/2352' if head[15] == 2 else 'MODE1/2352'
        t = Track(1, mode, path, 0, 0)
        t.frames = size // 2352
        return Disc(path, [t], notes=['no CUE sheet: read as one raw data track; audio tracks, if any, are not listed'])
    t = Track(1, 'MODE1/2048', path, 0, 0)
    t.frames = size // 2048
    return Disc(path, [t])
