#!/usr/bin/env python3
"""First look at a Flash game: an SWF file, or a Flash projector (an .exe with the SWF inside).
It also recognises Shockwave Director files and projectors.

Usage: swfinfo.py FILE [--tags] [--decompress OUT] [--extract OUT] [--json]

  FILE              an .swf, a projector .exe, or any file with an SWF inside
  --tags            list every tag in order, with its size
  --decompress OUT  write the movie uncompressed (an FWS file), which other tools read
  --extract OUT     write the SWF found inside a projector or another file

It reports the SWF version, the compression (FWS none, CWS zlib, ZWS LZMA), the stage size,
the frame rate and frame count, and whether the code is ActionScript 1 or 2 (DoAction tags,
run by the old virtual machine) or ActionScript 3 (DoABC tags, run by the AVM2). It counts
the shapes, bitmaps, sounds, fonts, sprites and binary data, and lists the class names the
movie links to symbols, the exported names, the metadata and any protection tags.

What it does not do is decompile the scripts. JPEXS Free Flash Decompiler (FFDec) does that,
and Ruffle runs the movie. docs/engines/flash-and-shockwave.md describes the route from
here. A Shockwave Director file (RIFX or XFIR) is only recognised: its Lingo scripts need
ProjectorRays. Decompressed and extracted movies are game material: write them into derived/.
Standard library only.
"""
import json
import lzma
import struct
import sys
import zlib

TAGS = {
    0: 'End', 1: 'ShowFrame', 2: 'DefineShape', 4: 'PlaceObject', 5: 'RemoveObject', 6: 'DefineBits',
    7: 'DefineButton', 8: 'JPEGTables', 9: 'SetBackgroundColor', 10: 'DefineFont', 11: 'DefineText',
    12: 'DoAction', 13: 'DefineFontInfo', 14: 'DefineSound', 15: 'StartSound', 17: 'DefineButtonSound',
    18: 'SoundStreamHead', 19: 'SoundStreamBlock', 20: 'DefineBitsLossless', 21: 'DefineBitsJPEG2',
    22: 'DefineShape2', 24: 'Protect', 26: 'PlaceObject2', 28: 'RemoveObject2', 32: 'DefineShape3',
    33: 'DefineText2', 34: 'DefineButton2', 35: 'DefineBitsJPEG3', 36: 'DefineBitsLossless2',
    37: 'DefineEditText', 39: 'DefineSprite', 41: 'ProductInfo', 43: 'FrameLabel', 45: 'SoundStreamHead2',
    46: 'DefineMorphShape', 48: 'DefineFont2', 56: 'ExportAssets', 57: 'ImportAssets',
    58: 'EnableDebugger', 59: 'DoInitAction', 60: 'DefineVideoStream', 61: 'VideoFrame',
    62: 'DefineFontInfo2', 63: 'DebugID', 64: 'EnableDebugger2', 65: 'ScriptLimits', 66: 'SetTabIndex',
    69: 'FileAttributes', 70: 'PlaceObject3', 71: 'ImportAssets2', 72: 'DoABC (old)',
    73: 'DefineFontAlignZones', 74: 'CSMTextSettings', 75: 'DefineFont3', 76: 'SymbolClass',
    77: 'Metadata', 78: 'DefineScalingGrid', 82: 'DoABC', 83: 'DefineShape4', 84: 'DefineMorphShape2',
    86: 'DefineSceneAndFrameLabelData', 87: 'DefineBinaryData', 88: 'DefineFontName', 89: 'StartSound2',
    90: 'DefineBitsJPEG4', 91: 'DefineFont4', 93: 'EnableTelemetry',
}
KINDS = {
    'shapes': ('DefineShape', 'DefineShape2', 'DefineShape3', 'DefineShape4', 'DefineMorphShape', 'DefineMorphShape2'),
    'bitmaps': ('DefineBits', 'DefineBitsJPEG2', 'DefineBitsJPEG3', 'DefineBitsJPEG4', 'DefineBitsLossless',
                'DefineBitsLossless2'),
    'sounds': ('DefineSound', 'SoundStreamHead', 'SoundStreamHead2'),
    'fonts': ('DefineFont', 'DefineFont2', 'DefineFont3', 'DefineFont4'),
    'sprites': ('DefineSprite',), 'buttons': ('DefineButton', 'DefineButton2'),
    'texts': ('DefineText', 'DefineText2', 'DefineEditText'), 'videos': ('DefineVideoStream',),
    'binary data': ('DefineBinaryData',),
}
PROJECTOR_MAGIC = 0xFA123456


def find_swf(data):
    """The offset of an SWF inside a file: the Windows projector trailer, else a plausible header."""
    if len(data) > 8 and struct.unpack_from('<I', data, len(data) - 8)[0] == PROJECTOR_MAGIC:
        size = struct.unpack_from('<I', data, len(data) - 4)[0]
        start = len(data) - 8 - size
        if start >= 0 and data[start + 1:start + 3] == b'WS':
            return start, 'Flash projector (Windows trailer)'
    for sig in (b'FWS', b'CWS', b'ZWS'):
        at = data.find(sig)
        while at >= 0:
            version = data[at + 3] if at + 3 < len(data) else 0
            length = struct.unpack_from('<I', data, at + 4)[0] if at + 8 <= len(data) else 0
            if 1 <= version <= 60 and 20 < length < 1 << 30:
                return at, 'embedded SWF' if at else 'SWF'
            at = data.find(sig, at + 1)
    return None, None


def unpack(swf):
    """The whole movie, uncompressed, starting with its 8-byte header."""
    sig, version = swf[:3], swf[3]
    length = struct.unpack_from('<I', swf, 4)[0]
    if sig == b'FWS':
        return swf[:length]
    if sig == b'CWS':
        body = zlib.decompressobj().decompress(swf[8:])
    elif sig == b'ZWS':
        props, data = swf[12:17], swf[17:]
        body = lzma.decompress(props + struct.pack('<Q', length - 8) + data, format=lzma.FORMAT_ALONE)
    else:
        raise ValueError('not an SWF')
    return b'FWS' + bytes([version]) + struct.pack('<I', length) + body


def header(movie):
    """The stage rectangle (bit-packed, in twips), the frame rate (8.8 fixed point) and the
    frame count, after the 8-byte header."""
    nbits = movie[8] >> 3
    nbytes = (5 + 4 * nbits + 7) // 8
    bits = int.from_bytes(movie[8:8 + nbytes], 'big')
    vals = []
    for i in range(4):
        shift = nbytes * 8 - 5 - nbits * (i + 1)
        v = (bits >> shift) & ((1 << nbits) - 1)
        if nbits and v >> (nbits - 1):
            v -= 1 << nbits
        vals.append(v)
    at = 8 + nbytes
    rate = struct.unpack_from('<H', movie, at)[0] / 256
    frames = struct.unpack_from('<H', movie, at + 2)[0]
    return {'width': (vals[1] - vals[0]) / 20, 'height': (vals[3] - vals[2]) / 20,
            'frame_rate': rate, 'frames': frames}, at + 4


def tags(movie, at, end=None):
    end = end or len(movie)
    while at + 2 <= end:
        word = struct.unpack_from('<H', movie, at)[0]
        code, size = word >> 6, word & 0x3F
        at += 2
        if size == 0x3F:
            size = struct.unpack_from('<I', movie, at)[0]
            at += 4
        yield code, at, size
        if code == 39:                  # DefineSprite: id, frame count, then its own tags
            for inner in tags(movie, at + 4, at + size):
                yield inner
        at += size
        if code == 0 and end == len(movie):
            break


def cstr(data, at):
    end = data.index(b'\0', at)
    return data[at:end].decode('utf-8', 'replace'), end + 1


def describe(movie):
    info, at = header(movie)
    info['version'] = movie[3]
    counts, order, classes, exports, extra = {}, [], [], [], {}
    for code, start, size in tags(movie, at):
        name = TAGS.get(code, f'tag {code}')
        counts[name] = counts.get(name, 0) + 1
        order.append((name, size))
        body = movie[start:start + size]
        if code == 69 and size >= 4:
            flags = body[0]
            extra['actionscript3_flag'] = bool(flags & 0x08)
            extra['network_access'] = bool(flags & 0x01)
        elif code == 76:
            n = struct.unpack_from('<H', body, 0)[0]
            p = 2
            for _ in range(n):
                p += 2
                s, p = cstr(body, p)
                classes.append(s)
        elif code == 56:
            n = struct.unpack_from('<H', body, 0)[0]
            p = 2
            for _ in range(n):
                p += 2
                s, p = cstr(body, p)
                exports.append(s)
        elif code == 77:
            extra['metadata'] = body.decode('utf-8', 'replace')[:400]
        elif code in (24, 58, 64):
            extra.setdefault('protection', []).append(name)
        elif code == 41 and size >= 26:
            extra['product_build'] = struct.unpack_from('<IIII', body, 0)
    if counts.get('DoABC') or counts.get('DoABC (old)'):
        info['actionscript'] = '3 (DoABC: AVM2)'
    elif counts.get('DoAction') or counts.get('DoInitAction'):
        info['actionscript'] = '1 or 2 (DoAction: AVM1)'
    else:
        info['actionscript'] = 'none found'
    info['content'] = {k: sum(counts.get(t, 0) for t in v) for k, v in KINDS.items() if sum(counts.get(t, 0) for t in v)}
    info['scripts'] = {k: counts[k] for k in ('DoAction', 'DoInitAction', 'DoABC', 'DoABC (old)') if k in counts}
    info['classes'] = classes[:40]
    info['exports'] = exports[:40]
    info.update(extra)
    return info, order


def director(data):
    sig = data[:4]
    if sig not in (b'RIFX', b'XFIR'):
        at = max(data.rfind(b'RIFX'), data.rfind(b'XFIR'))
        if at < 0:
            return None
        sig = data[at:at + 4]
    else:
        at = 0
    kind = data[at + 8:at + 12]
    if sig == b'XFIR':
        kind = kind[::-1]
    names = {b'MV93': 'Director movie (.dir, .dxr)', b'MC95': 'Director cast (.cst, .cxt)',
             b'FGDM': 'Shockwave movie, compressed (.dcr)', b'FGDC': 'Shockwave cast, compressed (.cct)',
             b'APPL': 'Director projector data'}
    return {'format': 'Shockwave Director', 'offset': at,
            'byte_order': 'big-endian (Mac)' if sig == b'RIFX' else 'little-endian (Windows)',
            'kind': names.get(kind, kind.decode('latin-1', 'replace')),
            'next': 'decompile Lingo with ProjectorRays; ScummVM runs many Director games'}


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name: argv[argv.index(name) + 1] if name in argv else None
    with open(argv[0], 'rb') as f:
        data = f.read()
    at, where = find_swf(data)
    if at is None:
        d = director(data)
        if d:
            print(json.dumps(d, indent=2) if '--json' in argv else '\n'.join(f'{k:12s} {v}' for k, v in d.items()))
            return 0
        print('no SWF or Director movie found')
        return 1
    swf = data[at:]
    movie = unpack(swf)
    info, order = describe(movie)
    info = {'found': where, 'offset': at, 'compression': {b'F': 'none', b'C': 'zlib', b'Z': 'LZMA'}[swf[:1]],
            **info}
    if opt('--extract'):
        length = struct.unpack_from('<I', swf, 4)[0]
        end = len(swf) if swf[:1] != b'F' else length
        if where == 'Flash projector (Windows trailer)':
            end = struct.unpack_from('<I', data, len(data) - 4)[0]
        with open(opt('--extract'), 'wb') as f:
            f.write(swf[:end])
        info['extracted'] = opt('--extract')
    if opt('--decompress'):
        with open(opt('--decompress'), 'wb') as f:
            f.write(movie)
        info['decompressed'] = opt('--decompress')
    if '--json' in argv:
        print(json.dumps(info, indent=2))
        return 0
    for k, v in info.items():
        if v in (None, [], {}, ''):
            continue
        print(f'{k:20s} {v}')
    if '--tags' in argv:
        for name, size in order:
            print(f'  {name:30s} {size:10,d}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
