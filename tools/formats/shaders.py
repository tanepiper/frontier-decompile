#!/usr/bin/env python3
"""Find the shaders inside a game's files: compiled Direct3D 9 and 10-12 shaders, SPIR-V, and
shader source text in GLSL, HLSL and the OpenGL ARB assembly.

Usage: shaders.py FILE [--dump DIR]

Shaders are small programs that run on the graphics card, and they decide what each frame
looks like: lighting, fog, water, the colour of the terrain. Games keep them in their own files,
in archives, or inside the executable. This tool finds each one, prints its kind, its version,
its size and the names it carries, and with --dump writes each to its own file for a
disassembler or a decompiler:

  Direct3D 9 (vs_1_1 to ps_3_0)  version token 0xFFFE.... (vertex) or 0xFFFF.... (pixel), ending
                                 with 0x0000FFFF. The constant table (CTAB) names the inputs.
                                 Disassemble with fxc /dumpbin, or a D3D9 disassembler.
  DXBC (Direct3D 10 and 11)      'DXBC' container of chunks: RDEF (names of the constants), ISGN and
                                 OSGN (inputs and outputs), SHDR or SHEX (the code). Disassemble
                                 with fxc /dumpbin or decompile with a DXBC decompiler.
  DXIL (Direct3D 12)             the same container with a DXIL chunk: LLVM bitcode. dxc -dumpbin.
  SPIR-V (Vulkan)                magic 0x07230203. spirv-dis, or spirv-cross to GLSL or HLSL.
  GLSL, HLSL, ARB                source text: #version, void main(, technique, !!ARBfp1.0.

Names in the constant tables (fog colour, light direction, world matrix) say what each
shader is for. RenderDoc or a similar capture tool shows which shader draws which object in a
frame; this tool finds them in the files without running the game. Standard library only.
"""
import os
import re
import struct
import sys

STAGE = {0: 'pixel', 1: 'vertex', 2: 'geometry', 3: 'hull', 4: 'domain', 5: 'compute'}
NAME = re.compile(rb'[A-Za-z_][A-Za-z0-9_]{2,}')


def names_in(blob, limit=12):
    seen = []
    for m in NAME.finditer(blob):
        n = m.group().decode()
        if n not in seen and not n.startswith('Microsoft'):
            seen.append(n)
    return seen[:limit]


def d3d9(data):
    for m in re.finditer(rb'[\x00-\x03][\x01-\x03](?:\xfe|\xff)\xff', data):
        start = m.start()
        minor, major, kind = data[start], data[start + 1], data[start + 2]
        if not (major in (1, 2, 3) and minor in (0, 1, 2, 3, 4) and (major > 1 or minor >= 1)):
            continue
        i, ctab, ok = start + 4, b'', False
        while i + 4 <= len(data) and i - start < 0x20000:
            tok = struct.unpack_from('<I', data, i)[0]
            if tok == 0x0000FFFF:
                ok = True
                i += 4
                break
            op = tok & 0xFFFF
            if op == 0xFFFE:  # a comment block
                n = (tok >> 16) & 0x7FFF
                if data[i + 4:i + 8] == b'CTAB':
                    ctab = data[i + 8:i + 4 + n * 4]
                i += 4 + n * 4
                continue
            if op > 0x60 and op not in (0xFFFD,):
                break
            length = (tok >> 24) & 0x0F if major >= 2 else 0
            if major < 2:  # 1.x: walk to the next token with a known opcode
                i += 4
                while i + 4 <= len(data) and struct.unpack_from('<I', data, i)[0] & 0x80000000:
                    i += 4
                continue
            i += 4 + length * 4
        if ok and (ctab or i - start >= 24):
            stage = 'vs' if kind == 0xFE else 'ps'
            yield start, i - start, f'Direct3D 9 {stage}_{major}_{minor}', names_in(ctab)


def dxbc(data):
    for m in re.finditer(rb'DXBC', data):
        start = m.start()
        if start + 32 > len(data):
            continue
        one, size, count = struct.unpack_from('<III', data, start + 20)
        if one != 1 or not 32 < size <= len(data) - start or count > 64:
            continue
        chunks, kind, names = [], '', []
        for k in range(count):
            off = struct.unpack_from('<I', data, start + 32 + 4 * k)[0]
            tag = data[start + off:start + off + 4]
            clen = struct.unpack_from('<I', data, start + off + 4)[0]
            body = data[start + off + 8:start + off + 8 + clen]
            chunks.append(tag.decode('latin-1'))
            if tag in (b'SHDR', b'SHEX') and len(body) >= 4:
                ver = struct.unpack_from('<I', body)[0]
                kind = f'{STAGE.get(ver >> 16, "?")} {(ver >> 4) & 0xF}.{ver & 0xF}'
            if tag == b'RDEF':
                names = names_in(body)
            if tag in (b'DXIL', b'ILDB'):
                kind = kind or 'DXIL (Direct3D 12)'
        yield start, size, f'DXBC {kind or "?"} [{" ".join(chunks)}]', names


def spirv(data):
    for m in re.finditer(rb'\x03\x02\x23\x07', data):
        start = m.start()
        if start % 4 or start + 20 > len(data):
            continue
        version, _, bound, schema = struct.unpack_from('<IIII', data, start + 4)
        if schema != 0 or not 0x10000 <= version <= 0x10600 or not 0 < bound < 1 << 22:
            continue
        i, names = start + 20, []
        while i + 4 <= len(data):
            word = struct.unpack_from('<I', data, i)[0]
            count, op = word >> 16, word & 0xFFFF
            if count == 0 or op > 6000:
                break
            if op in (5, 15):  # OpName, OpEntryPoint
                text = data[i + (8 if op == 5 else 12):i + 4 * count].split(b'\0')[0]
                if text and text.decode('latin-1') not in names:
                    names.append(text.decode('latin-1'))
            i += 4 * count
        yield start, i - start, f'SPIR-V {version >> 16}.{(version >> 8) & 0xFF}', names[:12]


SOURCE = [(rb'#version \d+', 'GLSL source'), (rb'!!ARB[vf]p1\.0', 'ARB assembly'), (rb'!![VF]P1\.0', 'NV assembly'),
          (rb'\btechnique1?\d*\s+\w+\s*\{', 'HLSL effect source'), (rb':\s*SV_Target\b|:\s*COLOR0?\b', 'HLSL source'),
          (rb'gl_FragColor|gl_Position\s*=', 'GLSL source')]


def source(data, taken):
    for pat, kind in SOURCE:
        for m in re.finditer(pat, data):
            if any(a <= m.start() < a + n for a, n in taken):
                continue
            a = m.start()
            while a > 0 and (32 <= data[a - 1] < 127 or data[a - 1] in b'\r\n\t'):
                a -= 1
            b = m.end()
            while b < len(data) and (32 <= data[b] < 127 or data[b] in b'\r\n\t'):
                b += 1
            if b - a < 40:
                continue
            taken.append((a, b - a))
            yield a, b - a, kind, names_in(re.sub(rb'\b(void|float[234]?|vec[234]|mat[34]|uniform|in|out|return|main|'
                                                    rb'struct|sampler2D|texture2D|version)\b', b' ', data[a:b]), 8)


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0
    with open(argv[0], 'rb') as f:
        data = f.read()
    found = sorted(list(d3d9(data)) + list(dxbc(data)) + list(spirv(data)))
    taken = [(a, n) for a, n, _, _ in found]
    found += list(source(data, taken))
    found.sort()
    dump = argv[argv.index('--dump') + 1] if '--dump' in argv else None
    if dump:
        os.makedirs(dump, exist_ok=True)
    for off, size, kind, names in found:
        print(f'0x{off:08x}  {size:7d} bytes  {kind}')
        if names:
            print(f'            names: {", ".join(names)}')
        if dump:
            ext = {'D': '.cso', 'S': '.spv'}.get(kind[0], '.txt')
            with open(os.path.join(dump, f'{off:08x}{ext}'), 'wb') as f:
                f.write(data[off:off + size])
    print(f'{len(found)} shaders')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
