---
name: re-format
description: Decode a game's data files - package/archive containers, compressed entries, resource and asset formats, save games - by finding the code that reads them, prototyping a decoder with tools/formats/reader.py, proving it against every file of its type with validate.py (all bytes consumed), then porting it. Use when a file type is not yet understood or a decoder fails on some files.
---

# Decode a data format

A decoder is right only when it reads **every file** of its type to the **last byte**. One
sample proves little: optional fields, versions and rare cases show up in a few files only.

## 1. Look at the bytes

```bash
xxd target/data/units.dat | head -20
```

Look for: a magic number at the start; counts and sizes (small integers that match the file's
length or the number of something visible); offsets (values that point inside the file);
length-prefixed strings (a small integer followed by that many printable bytes); repeated
records of a fixed stride; float values (`0000803f` is 1.0, `0000003f` is 0.5). Measure entropy
(`re-inventory` section 4): near 8.0 means compressed or encrypted.

```bash
python3 tools/triage/filetypes.py target/data/units.dat --blocks 4096   # where the dense parts are
python3 tools/formats/xorscan.py target/data/script.dat                 # a simple XOR or add key?
python3 tools/formats/timestamps.py --scan saves/slot1.sav --from 1996 --to 1998   # a saved date?
python3 tools/triage/similar.py saves/before.sav saves/after.sav                  # how much changed
python3 tools/triage/constants.py target/GAME.EXE        # the decompressor or checksum, by its constants
python3 tools/triage/binview.py target/data/units.dat derived/units.png --width 24   # records show as stripes
python3 tools/formats/rawimage.py target/data/TITLE.SCR derived/title.png --mode linear8   # a raw picture?
python3 tools/formats/markers.py plant saves/slot1.sav derived/marked.sav --from 0x40   # follow fields into memory
python3 tools/formats/fields.py target/data/units.dat --stride 24 --png derived/units.png   # what each offset holds
python3 tools/formats/skeleton.py target/models/hero.mdl    # parent arrays, quaternions, joint names
python3 tools/formats/shaders.py target/data/shaders.pak --dump derived/shaders   # compiled and source shaders
```

Text that turns readable under one XOR byte or a short key is a common way games hid their
scripts and tables. xorscan.py tries those weak schemes and shows a preview of each. Confirm
the scheme in the reader's code before you port it. Two saves made a minute apart, with one
setting changed, show which bytes hold that setting and where the save time is.

## 2. Find the reader in the code

Guessing a format from bytes alone is slow and fragile. Find the code that reads it:

- the file's name or extension in the strings (`decomp_query.py strings .dat`), then the
  function that opens it, then the function that parses the opened bytes;
- the magic number as an integer constant (`decomp_query.py find 0x4c504d53`);
- for engines with a serializer pattern (`Serialize(Stream&)`), the class's serializer: it reads
  each field in order, with its type, and its version checks (`if (version > 2)`).

Write down each read in order: offset, type, meaning, and the condition under which it is read.

### Measure the reads (DOS games)

Run the original under DOSBox-X with the file log, then map the reads per file:

```bash
scripts/dosbox-run.sh --fileio --time 30 target/game GAME.EXE
python3 tools/dos/fileio_trace.py analysis/dosbox/dosbox.log --game target/game --file LEVELS.DAT
```

A program that reads a file field by field shows its record layout in the read sizes:
`[2, 1, 4] x 120 (stride 7)` is 120 records of a word, a byte and a longint. A 1-byte read
followed by a read of varying length is a Pascal string. This is *measured* evidence; name the
run (program, inputs, time). With Borland debug symbols, `tools/dos/tdinfo.py BINARY types NAME`
gives the record type the reader fills, field by field.

## 3. Containers and compression

Most games pack files into an archive: a header, a table of entries (name or hash, offset, size,
compressed size, flags), then the data. Find the table reader first; then each entry is an
ordinary file. Common compression: zlib/deflate (`78 9c`), LZ4, zstd (`28 b5 2f fd`), LZO, LZF,
LZMA, Oodle (Kraken/Mermaid: a proprietary library, look for `oo2core` DLLs). If the bytes are
encrypted, the key and the algorithm are in the reader: trace them, do not brute-force.

Extract entries into `derived/extracted/` (ignored by git), never into tracked folders.

## 4. Prototype the decoder in Python

```python
# decoders/units.py
def decode(r):                       # r is a tools/formats/reader.py Reader
    r.magic(b'UNIT')
    version = r.u32()
    units = []
    for _ in range(r.u32()):
        u = {'id': r.u32(), 'name': r.pstr32(), 'speed': r.f32()}
        if version >= 2:
            u['armour'] = r.u16()
        units.append(u)
    return units
```

`reader.py` gives bounds-checked reads (`u8 u16 u32 i32 u64 f32 f64 bool bytes cstr pstr8
pstr32 wstr`), big-endian reads (`u16be i16be u32be i32be f32be`) for Amiga, Mac and console
data, variable-length numbers (`uleb sleb`), bit fields (`bits(n)`, `align()`), `magic()`, and `block(size)` for a length-prefixed section that must be consumed
exactly. Then run it over every file:

```bash
.venv/bin/python tools/formats/validate.py decoders/units.py 'derived/extracted/**/*.unit' --show 1
```

Every file must report complete. A failure says where it stopped and shows the next bytes: a
missed field, a wrong type or a version branch. `examples/sample-game/decode_sample.py` is a
complete small example.

## 5. Port and test

Port the decoder to the reimplementation's language as a plain reader with no dependencies, one
function per type, documenting each field's meaning and naming the original reader function.
Its test opens the real files from the player's game, **skips with a message when they are not
there** (they are never committed), parses every file and spot-checks a known value.

If the project writes the format too (an editor, a mod tool, saves), prove the writer with a
round trip: read every file, write it back, and compare the bytes. Each difference is a field
the writer loses.

## 6. Save games

Saves are the strictest test of a format: they hold the whole state of a running game. Find the
save function (strings: the save folder, the extension), trace it like a serializer, and decode
real saves of different sizes and ages. Saves from an older game version may use older
branches: keep them in the corpus. Later, a reimplementation that writes saves must produce
files the original game loads.

## Pitfalls

- Files in one folder can be different types: select by magic or type id, not by folder.
- Two empty strings in a row look like a zero integer: confirm with a file that has text.
- Booleans are often one byte; enums four; colours four floats or one packed integer: the reader
  code says which.
- Alignment and padding: some formats align records to 4 or 16 bytes.
- Never commit sample game files as test fixtures; generate synthetic ones if a test needs them.
