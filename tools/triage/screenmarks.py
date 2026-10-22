#!/usr/bin/env python3
"""Mark positions read from the game's memory on a screenshot, through the camera's matrices.

Usage: screenmarks.py SCREENSHOT.png --view 16FLOATS --proj 16FLOATS [--column]
                      (--point X,Y,Z[=LABEL] ... | --points FILE) [--out marks.svg]

Each position is multiplied by the view matrix, then the projection matrix, divided by its
last component, and scaled to the screenshot's size. The result is an SVG with the screenshot
inside it and a labelled circle at each position. If the circles sit on the objects, the
positions, the matrices and their layout are right. If they do not, one of the three is wrong.

Matrices are 16 numbers in memory order, as `readdata.py f32 ADDRESS --count 16` prints them,
separated by commas or spaces. Direct3D programs multiply a row vector by the matrix (the
default). OpenGL programs multiply the matrix by a column vector: give --column. --points
reads a file of lines "x y z [label]". Points behind the camera are listed and left off.
"""
import argparse
import base64
import struct
import sys


def png_size(path):
    with open(path, 'rb') as f:
        head = f.read(24)
    if head[:8] != b'\x89PNG\r\n\x1a\n':
        sys.exit(f'{path}: not a PNG file')
    return struct.unpack('>II', head[16:24])


def matrix(text):
    vals = [float(v) for v in text.replace(',', ' ').split()]
    if len(vals) != 16:
        sys.exit(f'a matrix needs 16 numbers, got {len(vals)}')
    return [vals[i * 4:i * 4 + 4] for i in range(4)]


def apply(m, v, column):
    """v (4 values) through m: v * m for row vectors, m * v for column vectors."""
    if column:
        return [sum(m[r][c] * v[c] for c in range(4)) for r in range(4)]
    return [sum(v[r] * m[r][c] for r in range(4)) for c in range(4)]


def project(point, view, proj, column, width, height):
    """Screen x and y of a world position, or None when it is behind the camera."""
    clip = apply(proj, apply(view, list(point) + [1.0], column), column)
    if clip[3] <= 1e-9:
        return None
    x, y = clip[0] / clip[3], clip[1] / clip[3]
    return (x + 1) / 2 * width, (1 - y) / 2 * height


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('screenshot')
    ap.add_argument('--view', required=True)
    ap.add_argument('--proj', required=True)
    ap.add_argument('--column', action='store_true')
    ap.add_argument('--point', action='append', default=[])
    ap.add_argument('--points')
    ap.add_argument('--out', default='marks.svg')
    a = ap.parse_args(argv)
    width, height = png_size(a.screenshot)
    view, proj = matrix(a.view), matrix(a.proj)
    points = []
    for spec in a.point:
        coords, _, label = spec.partition('=')
        points.append(([float(v) for v in coords.split(',')], label))
    if a.points:
        for line in open(a.points):
            parts = line.split()
            if len(parts) >= 3:
                points.append(([float(v) for v in parts[:3]], ' '.join(parts[3:])))
    image = base64.b64encode(open(a.screenshot, 'rb').read()).decode()
    marks = []
    for (xyz, label) in points:
        at = project(xyz, view, proj, a.column, width, height)
        name = label or ','.join(f'{v:g}' for v in xyz)
        if at is None:
            print(f'behind the camera: {name}')
            continue
        x, y = at
        print(f'{name}: {x:.1f}, {y:.1f}')
        marks.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="6" fill="none" stroke="#f43f5e" stroke-width="2"/>'
                     f'<text x="{x + 9:.1f}" y="{y - 9:.1f}" font-family="monospace" font-size="12" fill="#f43f5e" '
                     f'stroke="#000" stroke-width="3" paint-order="stroke">{name}</text>')
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">'
           f'<image href="data:image/png;base64,{image}" width="{width}" height="{height}"/>{"".join(marks)}</svg>\n')
    with open(a.out, 'w') as f:
        f.write(svg)
    print(f'{len(marks)} marks written to {a.out}')


if __name__ == '__main__':
    main(sys.argv[1:])
