#!/usr/bin/env python3
"""Run a decoder over every file of a type and report which files fail or leave bytes unread.

Usage: validate.py DECODER.py GLOB [GLOB ...] [--show N]

DECODER.py defines `decode(reader)`, which reads one whole file from a `Reader` (reader.py) and
returns what it found; validate.py then calls `reader.done()`, so a file with unread bytes
counts as a failure. A decoder is right only when every file of its type passes: one sample is
not enough, because optional fields and version differences show up in a few files only.
`--show N` prints the result of the first N files. Exit status 1 if any file fails.

    validate.py examples/sample-game/decode_sample.py 'examples/sample-game/build/*.dat' --show 1
"""
import glob
import importlib.util
import os
import pprint
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from reader import FormatError, Reader  # noqa: E402


def main(argv):
    show = 0
    if '--show' in argv:
        i = argv.index('--show')
        show = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    decoder_path, *patterns = argv
    spec = importlib.util.spec_from_file_location('decoder', decoder_path)
    decoder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(decoder)
    files = sorted({f for p in patterns for f in glob.glob(p, recursive=True)})
    if not files:
        sys.exit('no files match')
    failed = 0
    for i, path in enumerate(files):
        data = open(path, 'rb').read()
        r = Reader(data, name=os.path.basename(path))
        try:
            result = decoder.decode(r)
            r.done()
        except (FormatError, ValueError, IndexError, UnicodeDecodeError) as e:
            failed += 1
            print(f'FAIL {path}: {e}')
            continue
        if i < show:
            print(f'{path}:')
            pprint.pprint(result, width=100)
    print(f'{len(files) - failed} of {len(files)} files decode completely')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1:])
