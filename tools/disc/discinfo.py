#!/usr/bin/env python3
"""First look at a CD image (CUE/BIN, raw BIN or ISO): its tracks, its file systems, its dates,
and the files on it. It can also copy the files and the audio tracks out.

Usage: discinfo.py IMAGE [--files] [--map] [--extract DIR] [--audio DIR] [--swap] [--json]

  IMAGE           a .cue sheet (its BIN files beside it), a raw .bin, or a .iso
  --files         list every file: disc sector, size, date, path
  --map           list the runs of sectors that hold data but belong to no file (slower: it
                  reads every sector of the data tracks)
  --extract DIR   copy every file of the first readable volume into DIR (Joliet names if present)
  --audio DIR     write each audio track as a WAV file (44,100 Hz, 16-bit stereo) into DIR
  --swap          the audio was ripped big-endian: swap the bytes of each sample
  --json          print everything as JSON

What it reports, and why it matters for a port:
- the tracks: data or audio, the sector mode (Mode 1, Mode 2 XA), the start and length. A
  game with audio tracks plays its music from the disc (CD-DA), so the port must play those
  tracks, and the game's code holds a table of track numbers.
- the file systems: ISO 9660, Joliet (long Windows names), Rock Ridge, High Sierra, El Torito
  (a bootable disc), and HFS or HFS+ (a hybrid Mac and PC disc, where the Mac files can differ).
- the identifiers and dates in the volume descriptor: the publisher, the data preparer, the
  application that mastered it, and when. With the file dates, they date a release and tell
  two pressings apart.
- Mode 2 Form 2 sectors (video, streamed sound) in a data track, which a cooked .iso drops.
- hidden files and files outside the volume's size, which some copy protections use.
- files in Mode 2 Form 2 (XA): PlayStation STR video and XA sound, for example. --extract
  copies these as whole 2352-byte sectors, because a copy of 2048 bytes per sector loses data.
- the system area (sectors 0 to 15): empty on a PC disc, the licence and boot data on a console.
- files that only one of the ISO 9660 and Joliet trees lists, and files dated after the volume.

It reads the image only. The files it copies are game material: extract into target/ (or
another folder git ignores). Standard library only.
"""
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from image import open_image  # noqa: E402
from volumes import scan  # noqa: E402
from checks import late_dates, system_area, tree_differences, unused_data  # noqa: E402


def wav(path, pcm, swap):
    if swap:
        b = bytearray(pcm)
        b[0::2], b[1::2] = pcm[1::2], pcm[0::2]
        pcm = bytes(b)
    with open(path, 'wb') as f:
        f.write(b'RIFF' + struct.pack('<I', 36 + len(pcm)) + b'WAVE')
        f.write(b'fmt ' + struct.pack('<IHHIIHH', 16, 1, 2, 44100, 44100 * 4, 4, 16))
        f.write(b'data' + struct.pack('<I', len(pcm)) + pcm)


def safe_join(root, rel):
    parts = [p for p in rel.replace('\\', '/').split('/') if p not in ('', '.', '..')]
    return os.path.join(root, *parts)


def form2_count(track, limit=20000):
    if track.size not in (2352, 2336) or not track.mode.startswith(('MODE2', 'CDI')):
        return 0
    return sum(track.form2(i) for i in range(min(track.frames, limit)))


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opts = {a: argv[i + 1] for i, a in enumerate(argv) if a in ('--extract', '--audio')}
    flags = set(argv)
    disc = open_image(argv[0])
    report = {'image': os.path.basename(argv[0]), 'tracks': [], 'volumes': [], 'notes': list(disc.notes)}
    volumes = []
    for t in disc.tracks:
        d = t.describe()
        d['kind'] = 'audio' if t.audio else 'data'
        d['session'] = disc.sessions.get(t.number, 1)
        if not t.audio:
            n = form2_count(t)
            if n:
                d['form2_sectors'] = n
            for v in scan(t):
                v.info = {'track': t.number, **v.info}
                volumes.append(v)
        report['tracks'].append(d)
    audio = [t for t in disc.tracks if t.audio]
    if audio:
        secs = sum(t.frames for t in audio) / 75
        report['notes'].append(f'{len(audio)} audio tracks, {int(secs // 60)} min {int(secs % 60)} s: the game may '
                               f'play them as music (look for MCI or MSCDEX play calls and a track table)')
    for v in volumes:
        entry = {'kind': v.kind, **v.info}
        if v.root is not None:
            files = v.entries()
            entry['files'] = sum(not e.directory for e in files)
            entry['directories'] = sum(e.directory for e in files)
            hidden = [e.path for e in files if e.hidden]
            if hidden:
                entry['hidden'] = hidden[:20]
            limit = v.info.get('sectors') or 0
            outside = [e.path for e in files if limit and e.lba + (e.size + 2047) // 2048 > limit]
            if outside:
                entry['outside_volume'] = outside[:20]
            form2 = [e.path for e in files if e.form2]
            if form2:
                entry['form2_files'] = form2[:20]
            late = late_dates(v)
            if late:
                entry['dated_after_volume'] = late[:20]
            if '--files' in flags:
                entry['list'] = [{'sector': e.lba, 'size': e.size, 'date': e.date,
                                  'path': e.path + ('/' if e.directory else '') + (' [XA Form 2]' if e.form2 else '')}
                                 for e in files]
        report['volumes'].append(entry)

    for t in disc.tracks:
        if not t.audio and any(v.track is t for v in volumes):
            area = system_area(t)
            if area:
                report['notes'].append(f'track {t.number}: the system area (sectors 0 to 15) holds {area}')
            if '--map' in flags:
                spare = unused_data(t, volumes)
                report['unused_data'] = [{'track': t.number, 'first': a, 'last': b, 'kind': k} for a, b, k in spare]
                if not spare:
                    report['notes'].append(f'track {t.number}: every sector with data belongs to the file system')
    for a, b, only in tree_differences(volumes):
        report['notes'].append(f'{len(only)} files in the {a} tree are missing from the {b} tree: {", ".join(only[:5])}')

    if '--extract' in opts:
        readable = [v for v in volumes if v.root is not None]
        chosen = next((v for v in readable if v.joliet), readable[0] if readable else None)
        if chosen is None:
            report['notes'].append('--extract: no ISO 9660 volume to copy from')
        else:
            n = 0
            # The chosen tree's files, then any file another tree lists and this one leaves out.
            listed = {e.lba for e in chosen.entries()}
            extra = [(v, e) for v in readable if v is not chosen for e in v.entries()
                     if not e.directory and e.lba not in listed]
            for src, e in [(chosen, e) for e in chosen.entries()] + extra:
                dest = safe_join(opts['--extract'], e.path)
                if e.directory:
                    os.makedirs(dest, exist_ok=True)
                    continue
                os.makedirs(os.path.dirname(dest), exist_ok=True)
                with open(dest, 'wb') as f:
                    if e.form2 and src.track.size in (2336, 2352):
                        f.write(src.track.raw(e.lba - src.base, e.sectors))   # whole sectors
                    else:
                        f.write(src.file(e))
                n += 1
            report['notes'].append(f'--extract: {n} files from the {chosen.kind} volume into {opts["--extract"]}')
    if '--audio' in opts and audio:
        os.makedirs(opts['--audio'], exist_ok=True)
        for t in audio:
            dest = os.path.join(opts['--audio'], f'track{t.number:02d}.wav')
            wav(dest, t.raw(0, t.frames), '--swap' in flags)
        report['notes'].append(f'--audio: {len(audio)} WAV files in {opts["--audio"]}')

    if '--json' in flags:
        print(json.dumps(report, indent=2))
        return 0
    print(f'{report["image"]}: {len(disc.tracks)} tracks')
    for d in report['tracks']:
        extra = f', {d["form2_sectors"]} Mode 2 Form 2 sectors' if d.get('form2_sectors') else ''
        sess = f' session {d["session"]}' if any(x['session'] > 1 for x in report['tracks']) else ''
        print(f'  track {d["track"]:2d}  {d["kind"]:5s} {d["mode"]:11s} start {d["start"]}  '
              f'length {d["length"]}  ({d["frames"]} sectors){sess}{extra}')
    for entry in report['volumes']:
        print(f'{entry["kind"]} (track {entry["track"]})')
        for k, val in entry.items():
            if k in ('kind', 'track', 'list') or val in (None, '', False):
                continue
            print(f'  {k:16s} {val}')
        for f in entry.get('list', []):
            print(f'    {f["sector"]:8d} {f["size"]:11,d}  {f["date"] or "":25s} {f["path"]}')
    for u in report.get('unused_data', []):
        print(f'unused  track {u["track"]} sectors {u["first"]}-{u["last"]} ({u["last"] - u["first"] + 1}): {u["kind"]}')
    for note in report['notes']:
        print(f'note: {note}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
