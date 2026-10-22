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
commands a 12-bit colour `(word >> 4) & FFFh` with two flag bits (`1000:51c2`).

**Command lengths, measured.** `apps/viewer/examples/model_census.rs` runs the original on the
runtime and watches the interpreter's three command fetches (`LODSW` at `1000:b21e`, `b243`,
`b268`). Over 120 s of the intro it saw 2,196 distinct commands. The 1,247 that run straight
on to the next command all have the lengths below (`crates/formats::model::length`); the rest
are skips, calls and ends. `10`, `12` and `17` did not run; their counts are *observed* from
the handlers.

| Op | Handler | Length in words, with the command | What it does |
|---|---|---|---|
| `00` | `7e62` | 1 | end |
| `01` | `492d` | 3 | a disc at one vertex (*inferred*: projects one vertex, `1000:4991`) |
| `02` | `51bc` | 2 | line: vertices in the two bytes |
| `03` / `07` | `54c4` / `549c` | 3 | triangle: corners `CL`, `CH`, `DH`, normal `DL`; `07` then again mirrored (every byte xor 1) |
| `04` / `08` | `5c6f` / `5c42` | 4 | quad: corners in the order `CL`, `DL`, `CH`, `DH`, normal in the third word; `08` then mirrored |
| `05` | `67e7` | 2 + sub-commands | polygon (below) |
| `06` | `4265` | 1, or 2 when `word >> 5` is 700h–7FDh | depth group (*inferred*) |
| `09` | `71ed` | 3 | a thruster flame between two vertices with a size (*inferred*: projects both, `1000:726e`, `729c`; points along all six axes from each hull) |
| `0A` | `78d8` | 4 | text: word 1 the normal (low byte), the font (bits 12–15, a model of the second table, `1000:190a`: segment `[DS:07f8]`, offset `8106h`) and scale (bits 8–11); word 2 the origin vertex (low byte) and orientation (bits 8–13); word 3 a text number (below) |
| `0B` / `0C` | `7dff` / `7e85` | 2 | distance test; on failure skip `(word >> 5) × 2` bytes, or end the model if that is 0 |
| `0D`, `1D` | `7f82` | 2 | arithmetic on model variables (below) |
| `0E` | `7c0c` | 2, or 4 when the operand has bit 15 set | draws model `word >> 5` at the vertex in the operand's low byte, with the parent's colour, oriented by the operand's bits 8–13 as text is (`1000:75fb`: a permutation, bits 13–11 negate x, y, z; the Hawk's second wing is `2003h`, mirrored in x) |
| `0F`, `15`, `19`, `1E` | | 1 | not traced |
| `10` / `11` | `81ab` / `812a` | 4 / 6 | not traced |
| `12` | `8da6` | 2 | not traced |
| `1C` | `76dd` | 2 | builds the matrix for `0A` (below) |
| `13` / `14` | `7ec4` / `7f23` | 2 | flag tests with the same skip as `0B` |
| `16` | | 4 | not traced |
| `17` | `8ec1` | 3 | not traced |
| `1B` | `7b6b` | 3, or 5 when the first operand has bit 15 set (`1000:7c01`) | draws model `word >> 5` at the vertex in the first operand, with its own colour from the second (`(op << 1) & EFEh`, `Exxh` a texture, bit 4 adds the model colour) and a scale step in its top 4 bits |
| `18` | `8f60` | words up to and including `7F7Fh` | not traced |
| `1A` | `8c8b` | 2, or 9 when the operand has bit 15 set | sets the model colour (liveries, below) |
| `1F` | `93d5` | 3 + `(word >> 5)` | a block the handler reads (the planets use it) |

A polygon (`05`) is followed by sub-command words until one with sub-command 0; the sub-command
is bits 9–11 (`1000:6968`, table `CS:28a4`). The skip table `CS:2902` gives the extra words:
sub-command 1 two, 2, 4 and 6 one, 3 and 5 none (*observed*, and *measured* with the lengths
above). Sub-commands 2 and 3 name a vertex in their low byte (`1000:6a05`, `6b91`); sub-command 1
is *inferred* to be a curve whose four control vertices are the bytes of its two extra words.

**Corner order and normals, checked against the data.** A face's normal (at `normals + (k >> 1)
× 4 − 4` for normal k, odd k mirrored; bytes `[nx, a vertex on the face, nz, ny]`,
`EL2MATH3:1bcd`) names one of the face's own corners in 153 of 200 triangles with the layout
above, and in at most 87 with any other. With the quad corners in the order `CL`, `DL`, `CH`,
`DH`, 324 of the 325 quads with four distinct plain corners are flat and uncrossed; in the byte
order, 142.

**Measured over the shipped tables:** every model of both tables (230 in `FRONTIER.EXE`, 10
slots empty, and `EL2M08`'s) walks to its end.

**What the models are.** In the intro, the original looks up only `EL2M08`'s table (*measured*
with `model_census`): models 2–3 with the GameTek + Konami card, 6–16 with the title, 17, 38,
103 and 105–107 with a ship in flight, 67–93 with the starport, its buildings and the mountain,
25, 39 and 41–45 with a ship over water. For the main table:

**Ship names, measured.** A ship's name is in its own model: the header word `+1Ah` points
at a ship data block (`1ee8:63e3` reads it). The block starts with tagged items (`8080h` with
2 words, `80A0h` with 3; `8010h`, `8050h` and `80D0h` with byte pairs up to a word below
100h; a zero word ends the list). Then come a double word, a word, four words that begin with
the mass and the internal capacity in tonnes, and the name as text `4000h + n` in the list at
`1ea9:06aa`, the table the text expander uses for texts `4000h`–`7FFFh` ([TEXT.md](TEXT.md);
entry 0 "CORE", 4 "Mine", 12h "Eagle long range fighter", … 2Bh "Long-range Cruiser"). `crates/formats::model::ship_name` reads it.

Checked in the game with `model_census` on the runtime (Escape at 3 s, space at 5 s, start
option 1 at 8 s; landed at Merlin, Ross 154):

| Screen | Name on screen | Models the game drew | Text in the model's block |
|---|---|---|---|
| F3, ship equipment status | Eagle long range fighter | 16 (with parts 2 and 111) | `4012h`: 18, "Eagle long range fighter" |
| F4, Shipyard, New and Reconditioned Ships | Constrictor, Puma Clipper, Constrictor, Tiger Trader | 27 twice, 31, 36 | 27: "Constrictor"; 31: "Tiger Trader"; 36: "Puma Clipper" |
| the same, View on the Tiger Trader | Tiger Trader, 400 t, 320 t | 31 (with parts 2, 111, 120–127) | "Tiger Trader"; block words 400 and 320 |

The names do not follow the order of the list: Puma Clipper (text 33) is model 36 and Tiger
Trader (text 34) model 31. The blocks name one model each for every ship of the list from
Escape Capsule (9) to Long-range Cruiser (41), and also 60 and 92 (Long-range Cruiser) and
110 (Lynx Bulk Carrier), whose roles are not known. No block names ". IXI ." (text 17).

Other groups, for the main table:

| Models | What | Evidence |
|---|---|---|
| 2–8 | missile and mine bodies, and the missiles and mines drawn with them (5–8 draw model 2, 4 draws model 3; no ship data block) | *inferred* |
| 42–44 | orbital stations (42 a polyhedron, 43 a ring, 44 three pods) | *inferred* from drawing them |
| 45–58 | surface starports, cities and buildings; 49 is drawn as an object at Merlin | *inferred*; 49 *measured* |
| header scale 19 and up | planets, stars and terrain (most use `1F`) | *inferred* |
| the rest | parts drawn by other models through `0E`, and others | *observed* in the mesh lists |

**Model logic.** A model computes and tests variables while it is drawn (`1000:7f82`–`80cd`,
`7ec4`, `7f23`). An operand byte with bit 7 clear is a constant (bits 0–5, or bits 0–5 moved to
bits 10–15 when bit 6 is set); with bit 7 set it names variable n = bits 0–5: the model frame's
`[BP-40h + 2n]` when bit 6 is set, else the object's `[[BP-D8h] + 72h + 2n]`. `0D`/`1D` put
`a op b` into frame variable `(word >> 8) & 7`; the operation is `(word >> 4) & 15` through
`CS:28b2`: add, subtract, multiply, divide, shift right, shift left, maximum, a minimum
variant, multiply, arithmetic shift, an indexed object variable, two compares, `a·sin b`,
`a·cos b` (the table at `DS:5bff`, 2048 words per half turn) and AND. `13` skips when its value
is 0 or the bit `(operand >> 8) − 1` is clear; `14` skips when the value is not 0 and the bit
is set (or none is named). A sub-object's frame starts with the parent's first variable plus 1
and its next two (`1000:7d9e`).

*Measured*: the object variables when the original drew the shipyard's Tiger Trader (the hook
at `1000:b2db` in `model_census`): variable 0 counting down (a timer), 1 = `FFFFh`, 2–3 =
`2CA4h`, `1E44h`, 22–23 = `FFFFh`, 27–28 = `8888h`, 29–30 = `8282h`, the rest 0
(`crates/formats::model::SHOWN_SHIP`). With them the walker draws exactly the parts the
original looked up for that view (models 2, 111 three times, 120–123 and 127), and the Tiger
Trader's `1A` picks texture entry 9 as measured (`crates/formats/tests/original_data.rs`).

**Text and matrices.** `0A` draws a text with a font model of the second table (model 0 is the
font of the registrations): each character's glyph is the command list at the font's
`+20h + 2·(char − 20h)`, and the vertex named by the glyph's end word `>> 6` is where the next
character starts (`1000:7a94`–`7af4`). Text `3016h` is the object's registration (the string at
`[DS:07e4]`, the object's `+F6h`; text formatters, [TEXT.md](TEXT.md)); texts `4000h`– come from
the text table. The orientation is a row of `DS:5749` (six axis permutations; row 6 = the matrix
the last `1C` built) with three sign bits. `1C` builds that matrix: the identity's rows moved by
a permutation and negated (command bits 7–9 and 10–12), then, if the operand is not 0, two rows
turned by an angle (rows 1–2, 2–0 or 0–1 by command bits 5–6, `DS:57b5`). *Measured*: at
`1000:7a1e` the text frame's matrix is R·P for the parent's P, with R constant over 52 frames of
the turning Tiger Trader, and the walker's R for the Tiger Trader's registration equals it to
three decimals. The rows of R are the images of the font's x, y and z axes; its third row is the
face normal the text command names, so the registration lies on the hull face.

**Colours, textures and liveries.** A drawing command's colour field is `0RGBh`; bit 4 says
"use the model colour" and bit 8 "unshaded" (`1000:51c2`–`5211`). Colours `E00h`–`EFFh` are
textures: entry `((c >> 1) & 7) | ((c >> 5) & 7) << 3` of the table at `CS:25c4` (16 bytes per
entry: an `EL2G.GRF` bitmap number `D0h`–`DCh`, a 64 × 64 picture with texels 0–6, and seven
12-bit colour offsets, one per texel, added to the face's shade channel by channel and capped
at Fh, `1000:6037`–`60a8`). The drawing commands hand them to the textured fills
([VIDEO.md](VIDEO.md) §5.3, §7). The model colour is a double word `[BP-9Eh]` (colour, texture
entry): the object renderer starts it at `111h` (`1000:3abe`), a sub-object inherits its
parent's (`1000:7c2b`, `7daf`), and opcode `1A` sets it (`1000:8c8b`): its own colour field, or,
when its operand's high byte names a variable whose value v is 1–7, the v-th of the seven
colours that follow. That variable is the ship's paint: *measured* with `model_census` (hooks
at the seven sites that add the double word), the shipyard's Tiger Trader (model 31) is drawn
with `00090000h`, texture entry 9, which is the Tiger Trader's livery 2. The viewer offers the
livery as a choice (0 = each `1A`'s own colour).

The viewer evaluates the model logic with the measured Tiger Trader variables by default (all
0 on request; the distance tests take the near branch, *assumed*; every level of detail on
request), hides faces whose stored normal points away as `face_shade` does (back faces on
request), places sub-objects at their vertex scaled by 2^(their scale − the parent's), stretches
each texture over its whole face (the game picks one of 8 orientations), and lights with its own
base grey: a viewer's picture, not the game's renderer. `frontier-viewer --render main 31 out.png
--livery 2` draws the same picture into a file.

The ship names come from the text table at `1ea9:06aa` (texts `4000h`–`7FFFh`,
[TEXT.md](TEXT.md)).

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
- **3D models:** both tables, grouped by what the models are (ships and craft named from their
  data blocks, textured hulls with a livery choice,
  missiles, stations and starports, planets, parts), each drawn with its sub-objects (flat
  shaded or wireframe, thruster flames on request, turn with the mouse), with its header, vertex
  definitions and mesh commands.
- **Position files and anything unknown:** a hex view.

Run it natively with `cargo run -p frontier-viewer --release [-- path/to/game]` or in the browser
with `tools/dev-web.sh --app viewer`.

## Open questions

- What each object variable means (a ship's paint, landing gear, timers); the viewer has one
  measured set.
- What opcodes `0F`–`12`, `15`–`19` and `1E` draw (their lengths are known), and
  the second model table `0A` uses (`1000:190a`): the intro's ships draw their hulls through it.
- Which of models 2–8 is which missile or mine, and what models 39, 60, 92 and 110 are.
- The palette the game sets for the GRF pictures (`EL2MCGA3:0234` and the colour allocator).
- The font used by `EL2MCGA3:1a1b` (slot `DS:0218`): not in `EL2G.GRF`.
- How the `EL2M18` message tables index their strings.
- The packing of the position files (§6).
