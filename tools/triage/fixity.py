#!/usr/bin/env python3
"""Keep a copy of a game unchanged over the years, and prove it: package it as a BagIt bag with
a checksum for every file, and check the checksums again whenever you like.

Usage: fixity.py bag SOURCE BAG [--info KEY=VALUE ...]   copy SOURCE into a new bag
       fixity.py manifest FOLDER [OUT]                    checksums of a folder, in place
       fixity.py verify BAG|FOLDER|MANIFEST               check every file against its checksum

  bag        BAG/data/ holds the files; BAG/manifest-sha256.txt their checksums; bag-info.txt the
             date, the size (Payload-Oxum) and any --info lines, such as
             Source-Organization=..., External-Description="GOG release 1.2, installer 5.1";
             tagmanifest-sha256.txt the checksums of those files in turn
  manifest   writes FOLDER/manifest-sha256.txt (or OUT) for files you keep where they are,
             such as target/ in a project
  verify     reports every file that changed, went missing or was added. Exit status 1 when
             anything differs

Digital preservation calls this fixity: knowing that a file is still exactly what it was.
Storage fails slowly and quietly, and copies drift as people work on them. The BagIt layout
(RFC 8493, from the Library of Congress) is the common way libraries and archives package
files with their checksums, and many tools read it. A bag of the game as you received it,
checked now and then, is the reference every finding goes back to
(docs/working/preservation.md). Bags of game files are game material: keep them outside git.
Standard library only.
"""
import datetime
import hashlib
import os
import shutil
import sys


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def files(root, skip=()):
    for base, dirs, names in os.walk(root):
        dirs.sort()
        for n in sorted(names):
            p = os.path.join(base, n)
            rel = os.path.relpath(p, root).replace(os.sep, '/')
            if rel not in skip:
                yield rel, p


def write_manifest(root, out, prefix='', skip=()):
    total = count = 0
    with open(out, 'w', encoding='utf-8') as f:
        for rel, p in files(root, skip):
            f.write(f'{sha256(p)}  {prefix}{rel}\n')
            total += os.path.getsize(p)
            count += 1
    return total, count


def read_manifest(path):
    rows = {}
    for line in open(path, encoding='utf-8'):
        line = line.rstrip('\n')
        if line.strip():
            digest, _, rel = line.partition(' ')
            rows[rel.lstrip(' *')] = digest
    return rows


def bag(source, target, info):
    data = os.path.join(target, 'data')
    if os.path.exists(target) and os.listdir(target):
        raise SystemExit(f'{target} exists and is not empty')
    shutil.copytree(source, data)
    with open(os.path.join(target, 'bagit.txt'), 'w', encoding='utf-8') as f:
        f.write('BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n')
    total, count = write_manifest(data, os.path.join(target, 'manifest-sha256.txt'), 'data/')
    with open(os.path.join(target, 'bag-info.txt'), 'w', encoding='utf-8') as f:
        f.write(f'Bagging-Date: {datetime.date.today().isoformat()}\n')
        f.write(f'Payload-Oxum: {total}.{count}\n')
        f.write('Bag-Software-Agent: GameFrame tools/triage/fixity.py\n')
        for item in info:
            key, _, value = item.partition('=')
            f.write(f'{key.strip()}: {value.strip()}\n')
    with open(os.path.join(target, 'tagmanifest-sha256.txt'), 'w', encoding='utf-8') as f:
        for name in ('bagit.txt', 'bag-info.txt', 'manifest-sha256.txt'):
            f.write(f'{sha256(os.path.join(target, name))}  {name}\n')
    print(f'bag written to {target}: {count} files, {total:,} bytes')


def verify(path):
    if os.path.isfile(path):
        manifest, root = path, os.path.dirname(os.path.abspath(path))
    elif os.path.exists(os.path.join(path, 'bagit.txt')):
        manifest, root = os.path.join(path, 'manifest-sha256.txt'), path
    else:
        manifest, root = os.path.join(path, 'manifest-sha256.txt'), path
    expected = read_manifest(manifest)
    problems = 0
    for rel, digest in expected.items():
        p = os.path.join(root, rel)
        if not os.path.exists(p):
            print(f'missing  {rel}')
            problems += 1
        elif sha256(p) != digest:
            print(f'changed  {rel}')
            problems += 1
    in_bag = os.path.exists(os.path.join(root, 'bagit.txt'))
    scan = os.path.join(root, 'data') if in_bag else root
    prefix = 'data/' if in_bag else ''
    tags = {'manifest-sha256.txt', 'bagit.txt', 'bag-info.txt', 'tagmanifest-sha256.txt'}
    for rel, _ in files(scan):
        if prefix + rel not in expected and (prefix or rel not in tags):
            print(f'added    {prefix}{rel}')
            problems += 1
    if in_bag and os.path.exists(os.path.join(root, 'tagmanifest-sha256.txt')):
        for rel, digest in read_manifest(os.path.join(root, 'tagmanifest-sha256.txt')).items():
            if sha256(os.path.join(root, rel)) != digest:
                print(f'changed  {rel} (a tag file)')
                problems += 1
    print(f'{len(expected)} files checked: ' + ('all unchanged' if not problems else f'{problems} problems'))
    return 1 if problems else 0


def main(argv):
    if len(argv) < 2 or argv[0] not in ('bag', 'manifest', 'verify'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    if argv[0] == 'bag':
        info = [argv[i + 1] for i, a in enumerate(argv) if a == '--info']
        bag(argv[1], argv[2], info)
        return 0
    if argv[0] == 'manifest':
        out = argv[2] if len(argv) > 2 else os.path.join(argv[1], 'manifest-sha256.txt')
        total, count = write_manifest(argv[1], out, skip={os.path.relpath(out, argv[1]).replace(os.sep, '/')})
        print(f'{count} files, {total:,} bytes: checksums in {out}')
        return 0
    return verify(argv[1])


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
