# Data formats

The shipped data files and the data held inside the binaries: the image archive, screenshots,
the MIDI songs, the 3D models, text and the packed position files. Readers are in
`crates/formats` (`grf`, `smf`, `model`, `overlay`, `text`, `identify`) and the
`frontier-viewer` app shows all of them (§7). Addresses as in [OVERLAYS.md](OVERLAYS.md):
`1000:xxxx` main program, `DS:xxxx` main data segment, `EL2xxx:xxxx` overlay load images.

Evidence: static reading of the shipped binaries (Ghidra export, `analysis/`) and the readers
run over every shipped file (`crates/formats/tests/original_data.rs`, skipped without
`game-data/`). Nothing here was compared with the original running yet, except where marked.

## 1. `EL2G.GRF`: the image archive

`EL2MCGA3:062a` (inside the video initialisation `EL2MCGA3:05b8`) opens `EL2G.GRF`
(name at `EL2MCGA3:5d5b`) and calls the main program's archive loader `1000:e2ba`
(slot `DS:0094`) with the table `DS:5813`. The loader unpacks each entry into the 48 EMS pages,
16 KB per page, never splitting an entry across pages (`1000:e3cb`), and appends a far pointer
(offset, EMS page) per entry to `DS:5813`. *Observed* in code.

**File layout.** A sequence of entries, ended by a zero length word:

| Bytes | Field |
|---|---|
| 2 | `n`, length of the entry in memory (little-endian). 0 ends the file |
| 1 | flag. `FFh`: `n` raw bytes follow and are copied without a header (`1000:e36e`) |
| 7 | with the flag, the 8-byte image header below |
| … | `n − 8` bytes of data: copied as stored if flag bit 1 is set (`1000:e37c`), else unpacked by the coder below |

**Image header** (copied by the blitter `EL2MCGA3:02e9` to `CS:010e`–`0115`):

| Offset | Size | Field | Evidence |
|---|---|---|---|
| 0 | byte | flags: bit 0 colour 0 is transparent (`EL2MCGA3:0476`), bit 1 pixels stay packed and are unpacked into a stack buffer at draw time (`EL2MCGA3:03e3`) | observed |
| 1 | byte | rows (`CS:010f`, the row count at `EL2MCGA3:031c`) | observed |
| 2 | word | bytes per row (`CS:0110`) | observed |
| 4 | word | added to the x position (`EL2MCGA3:0380`) | observed |
| 6 | word | added to the y position (`EL2MCGA3:0317`) | observed |

**The coder** (`1000:e38d` in the loader, the same at `EL2MCGA3:040a` in the blitter). A control
byte `c` below 80h copies the next `c` bytes. Otherwise it copies `−(c SAR 3)` bytes (1–16)
from `((c & 7) << 8) | next byte` bytes back in the output, byte by byte, so a copy may overlap
itself.

**Measured against the shipped file:** 225 entries, every byte consumed, every header's
`rows × bytes per row` equals the unpacked size, and every packed entry is consumed exactly by
its unpacking. No raw (`FFh`) entries. The entries are the cockpit icons and panels, the scanner
grid, the faces of the comms screen (parts: hair, eyes, mouths), planet images, the galaxy map
and the FRONTIER logo. Pixels are palette indices; the game sets the palette at run time
(`EL2MCGA3:0234`, not traced), so the viewer shows them with the palette saved in
`PIC000.GRF` (*assumed* to be close: it is one the game set).

## 2. `PIC000.GRF`: a screenshot

The game does not read this file. `1000:db9f`, called from `1000:cdcd` when `DS:8908` = `FDh`
(a key, not identified), writes the screen to the first free `picNNN.grf` (name at `1000:da0a`,
digits counted up by `1000:dbe5`). The shipped `PIC000.GRF` (dated 3 Nov 1993) is such a file
left in the release.

| Bytes | Content | Evidence |
|---|---|---|
| 768 | the 256 DAC entries read from port `3C9h` (6-bit red, green, blue) | `1000:ddf5` |
| rest | the 64,000 bytes of `A000:0000`, run-length coded | `1000:dc0c` |

The coder (`1000:dcb1`, flushed by `1000:dc52`) is PackBits: a control byte `c` below 80h is
followed by `c + 1` literal bytes; otherwise the next byte is repeated `257 − c` times. Runs are
at most 125 bytes (`1000:dd82`). The shipped file decodes to exactly 64,000 pixels: the Boston
Base police inspection screen.

## 3. The `.ROL` songs: Standard MIDI Files

The nine `.ROL`/`.rol` files are Standard MIDI Files (`MThd`), loaded whole by `EL2ROL` for the
Roland MT-32 ([AUDIO.md](AUDIO.md) §4). `crates/formats::smf` reads them by the published SMF
1.0 layout (running status, meta and SysEx events, tempo). All nine parse; how `EL2ROL` walks
them is not traced. The AdLib music is not in these files: it is byte code inside
`EL2ADLIB.OVL` ([AUDIO.md](AUDIO.md) §3, §6).

## 4. 3D models

**Lookup.** The renderer gets a model from `1000:18b5` (slot `DS:0050`): with `BX` the model
number × 2, `ES` = `[DS:5701]`, `BX` = `ES:[BX + b] + b` where `b` = `[DS:56ff]`.
`1000:18de` (slot `DS:013c`) sets the default: segment = the relocated word `DS:07fa`, `b` = 4.
In the shipped `FRONTIER.EXE` that word is load-image paragraph `412Dh`, so the default table
is inside the main program, in its last 64 KB. A module can install its own table with
`1000:18c5` (slot `DS:0150`): `EL2M08` (the title sequence) does at `EL2M08:6eeb` with its
own code segment and `b` = `3B0h`. No other binary calls slot `0150`. *Observed.*

Both tables have 240 word slots; slots 0 and 1 are empty. The length is *inferred*: the smallest
offset in each table is exactly the table's size, so the table ends where the first model
starts. An object's model number is the word at object `+5Ah` (`1000:3810`).

**Header** (relative offsets from the header):

| Offset | Field | Evidence |
|---|---|---|
| `+0` | mesh commands | `1000:b2de` |
| `+2` | vertex definitions; also the header's length (`1Eh` for most models, `10h` for a few) | `1000:b27c` |
| `+4` | bytes of vertex records to reserve, 20h per vertex | `1000:b290` |
| `+6` | normals | `1000:b2ad` |
| `+8` | bytes of the normal list, 4 per normal | `1000:b298` |
| `+0Ah`, `+0Ch` | added into the frame's scale | *inferred* from `1000:7c4e` |
| `+0Eh` | read with the scale (a radius?) | *inferred* from `1000:7c48` |
| `+12h` | `0666h` on most models: a default 12-bit colour | *inferred* |
| `+1Ah` | offset of further data | `1ee8:63e3` |

**Vertex definitions:** 4 bytes per mirror pair, `[x, type, z, y]` (signed bytes; `EL2MATH3:0d3a`,
`0d58`). Even vertex numbers are the definition, odd ones its mirror partner (x negated for
plain points). The 22 types are in [MATH3.md](MATH3.md) ("The 3D routines"); derived types
name other vertices by their bytes (`z` byte, `y` byte, then `x` byte), and the partner is
built from the partners. The viewer places derived vertices in 3D without a frame (a midpoint
for the screen-midpoint types, the base point for the random type, t = ½ for interpolation):
*inferred*, for display only.

**Mesh commands.** A list of words interpreted by `1000:b207`: opcode = the low 5 bits, through
the 32-entry near table at `CS:2864`; the other bits are the command's parameter, for drawing
commands a 12-bit colour `(word >> 4) & FFFh` with two flag bits (`1000:51c2`). Operand words
per opcode, counted from each handler's reads (*observed*):

| Op | Handler | Operands | What it does |
|---|---|---|---|
| `00` | `7e62` | 0 | end |
| `01` | `492d` | 2 | not traced |
| `02` | `51bc` | 1 | line: vertices in the two bytes |
| `03` | `54c4` | 2 | triangle: vertices `CL`, `CH`, `DH`; normal `DL` |
| `04` | `5c6f` | 3 | quad: vertices `CL`, `CH`, `DL`, `DH`; normal in the third word |
| `05` | `67e7` | 1 + sub-commands | polygon (below) |
| `06` | `4265` | 0 | starts a depth group at vertex `word >> 5` (*inferred*) |
| `07` / `08` | `549c` / `5c42` | 2 / 3 | triangle / quad, then again with every vertex and the normal xor 1 (the mirror) |
| `09` | `71ed` | 2 | not traced |
| `0A` | `78d8` | 3 | draws another model (sub-object) |
| `0B` / `0C` | `7dff` / `7e85` | 1 | distance test; on failure skip `(word >> 5) × 2` bytes, or end the model if that is 0 |
| `0D`, `1D` | `7f82` | 1 | arithmetic on model variables |
| `0E` | `7c0c` | 3 | draws model `word >> 5` at the vertex in the first operand |
| `0F` | `80d0` | 0 | nothing |
| `10` / `11` | `81ab` / `812a` | 3 / 5 | not traced |
| `12`, `1C` | `8da6`, `76dd` | 1 | not traced |
| `13` / `14` | `7ec4` / `7f23` | 1 | flag tests with the same skip as `0B` |
| `17`, `1B` | `8ec1`, `7b6b` | 2 | not traced |
| `1A` | `8c8b` | 8 | not traced |
| `1F` | `93d5` | 2 + `(word >> 5)` words | a block the handler reads |
| `15`, `16`, `18`, `19`, `1E` | | ? | operands not traced: a walk stops here |

A polygon (`05`) is followed by sub-command words until one with sub-command 0; the sub-command
is bits 9–11 (`1000:6968`, table `CS:28a4`). The skip table `CS:2902` gives the extra words:
sub-command 1 two, 2, 4 and 6 one, 3 and 5 none (*observed*). Sub-commands 2 and 3 name a vertex
in their low byte (`1000:6a05`, `6b91`); sub-command 1 is *inferred* to be a curve whose four
control vertices are the bytes of its two extra words.

**Measured over the shipped tables:** the main table has 230 models (10 slots are empty); 178
walk to their end with the operand counts above. The others stop at an opcode whose operands are
not traced (`15` ×20, `18` ×10, `1E` ×9, `16` ×7, `19` ×5), and one walk runs off the end of the
image, so an operand count above is wrong somewhere on its path. The viewer follows every conditional (it draws every level of
detail at once) and draws with its own flat shading: a viewer's picture, not the game's renderer.

## 5. Text

The modules keep their text as `00h`-terminated strings with control codes inside
(`81h FDh 19h` before an `EL2M18` message, `FFh 30h nn` where a value is inserted).
`crates/formats::text` finds every run of text in a binary for the viewer. How the message tables
index the strings is open: `EL2M18:00a3` holds 12 words (`039Ch` …) that point to further word
tables (starting at odd offsets, e.g. `039Dh`) whose values (`3EB8h` …) do not resolve to
strings with a fixed base.

## 6. `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE`, `LATIS._0n`

All ten files begin with the bytes `11 00 C6`, and the rest has no readable text: they are
packed. The start menu offers "2 Mars" and "3 Lave" ([FINDINGS.md](FINDINGS.md) §5), so these are
*inferred* to be start positions (and the 2010-dated `LATIS._0n`, saved games). The reader is
not traced: the next step is to watch which file each menu choice opens on the runtime, then
trace that read.

## 7. The resource viewer (`apps/viewer`)

`frontier-viewer` lists every file and shows:

- **Program and overlays:** the MZ header and relocations, the export block (with the vector
  table range each slot falls in) or the module header, the strings, and a hex view. On
  `EL2ADLIB.OVL` it plays the 12 songs and 40 effects with the original driver's port
  (`crates/game/src/adlib.rs`) running on the file's own image, ticked at 202.27 Hz into the
  OPL2 model.
- **`EL2G.GRF`:** every entry as a picture, with its header.
- **`PIC000.GRF`:** the screenshot and its palette.
- **The `.ROL` songs:** tracks and events. They can be played on the OPL2 with one generic FM
  voice: a deviation for listening, since the game plays them only on an MT-32.
- **3D models:** both tables, each model drawn (flat shaded or wireframe, turn with the mouse),
  its header, vertex definitions and mesh commands, and where a walk stopped.
- **Position files and anything unknown:** a hex view.

Run it natively with `cargo run -p frontier-viewer --release [-- path/to/game]` or in the browser
with `tools/dev-web.sh --app viewer`.

## Open questions

- The operands of mesh opcodes `15`, `16`, `18`, `19`, `1E`, and what `01`, `09`, `10`–`12`,
  `17`, `1A`–`1C` draw.
- The palette the game sets for the GRF pictures (`EL2MCGA3:0234` and the colour allocator).
- The font used by `EL2MCGA3:1a1b` (slot `DS:0218`): not in `EL2G.GRF`.
- How the `EL2M18` message tables index their strings.
- The packing of the position files (§6).
