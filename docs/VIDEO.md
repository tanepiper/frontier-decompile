# `EL2MCGA3`: the video driver

This covers the MCGA/VGA video driver overlay: its 40 exports, the memory it keeps, the draw
command stream the main program feeds it, the rules its rasterisers follow, the bitmap cache and
the texture mapper, the palette, and how the native port was proven. It ends with the interface
a draw-command layer and a WebGPU backend can use. Evidence: Ghidra 12.1.4 disassembly
(`tools/ghidra/ExportDisasm.java`, `ExportRange.java`), Capstone 5 (`cstool`) for code Ghidra
decodes at the wrong offset (jump-table targets, `66h`-prefixed instructions), and execution of the
original on the reference runtime ([RUNTIME.md](RUNTIME.md)): fuzzed calls, call counts in play
(`crates/hooks/examples/mcga3_census.rs`) and shadow verification of every call during 90 s of
play. Nothing here was compared with DOSBox pixel by pixel.

Addresses are `EL2MCGA3:offset` in the overlay's load image (Ghidra `1000:offset`), `DS:xxxx` in
the main data segment, `CS:xxxx` in the overlay's own segment, `FRONTIER:xxxx` in the main
program.

## 1. The exports, `DS:01d0`–`026f`

The driver fills the 40 slots from `DS:01d0` with far pointers ([OVERLAYS.md](OVERLAYS.md) §2).
Names are the port's (*inferred* from what each routine does). "Calls in 90 s" is from shadow
verification of a 90 s run (intro, start menu, start at Ross 154, cockpit); 0 means not reached.

| Slot | Address | Name | What it does | Calls in 90 s |
|---|---|---|---|---|
| `01d0` | `015b` | `char_width` | Byte at font offset 2Eh + 11·`AL` | 0 |
| `01d4` | `2f4e` | `disc_clipped` | Disc of diameter `DX` at (`BX`,`CX`) if any of it can be in the view | 6,380 |
| `01d8` | `2f5a` | `disc` | The same without the test | 15,120 |
| `01dc` | `3489` | `clear_view_7f` | Rows 0–167 to colour 7Fh | 3,172 |
| `01e0` | `349c` | `clear_view_0` | Rows 0–167 to colour 0 | 0 |
| `01e4` | `34c9` | `copy_panel` | Rows 168–199 from the target to `DS:09e4`, pointer hidden | 0 |
| `01e8` | `34ac` | `copy_screen` | The whole screen from `DS:09e4` to the target, pointer hidden | 0 |
| `01ec` | `06ba` | `fill_rect` | Rectangle (`AX`,`CL`)–(`BX`,`DL`) in colour `BP` | 4 |
| `01f0` | `072a` | `fill_rect_masked` | The same, sparing colours 28h–2Bh | 0 |
| `01f4` | `3399` | `fill_masks` | The model renderer's rows (`frame_5598`), §3.6; frees the frame | 0 |
| `01f8` | `3343` | `fill_masks_args` | The same, 8 words for `DS:07bc`… taken from the stack | 2,110 |
| `01fc` | `3108` | `fill_rows` | A polygon from per-row crossing lists (`frame_e76`), §3.5; frees the frame | 156,213 |
| `0200` | `5a20` | `colour_entry` | DAC entry of allocator slot `DI` | 1,018,288 |
| `0204` | `44d1` | `textured_quad` | Bitmap `DI` mapped onto a quadrilateral, §5.3 | 45,734 |
| `0208` | `3635` | `textured_triangle` | Bitmap `DI` mapped onto a triangle | 4,161 |
| `020c` | `05b8` | `init` | Start-up, §2 | 1 |
| `0210` | `28f9` | `line` | Line (`AX`,`BX`)–(`CX`,`DX`) | 233,948 |
| `0214` | `2dd1` | `blob` | A small shape from the table at `CS:2c11` | 3,315 |
| `0218` | `1a1b` | `glyph` | Character `AL` at (`CX`,`DX`) in colour `BL`; returns its advance | 227,255 |
| `021c` | `02ce` | `blit_pointer` | `blit` of bitmap `SI` through the pointer's descriptor | 1,262 |
| `0220` | `02d3` | `blit` | Bitmap `BX` at (`CX`,`DX`) through the target descriptor at `SS:DI` | 1,431 |
| `0224` | `076e` | `pixel` | Pixel (`AX`,`BL`) in colour `DL` | 0 |
| `0228` | `1e41` | `quad_clipped` | Flat quadrilateral, clipped to the view | 3 |
| `022c` | `1e74` | `quad` | Flat quadrilateral, inside the view | 169,872 |
| `0230` | `0583` | `restore_under` | Put the background under the mouse pointer back | 1,598 |
| `0234` | `5b50` | `upload_colours` | Give new colours DAC entries and upload them | 3,027 |
| `0238` | `59ce` | `colour` | Allocator slot for 12-bit colour `BX` | 989,957 |
| `023c` | `5a29` | `colours_tick` | Age the allocator; with `AL` ≠ 0 reset it first | 3,125 |
| `0240` | `04d8` | `save_under` | Save the background under the mouse pointer | 1,445 |
| `0244` | `5d4a` | `nothing` | `RETF` | 3,172 |
| `0248` | `142a` | `load_high_colours` | Reset the allocator, upload entries 80h–FFh from `AX:SI` | 0 |
| `024c` | `0811` | `set_palette` | Reset the allocator, upload palette `AL` (all 256 entries) | 1 |
| `0250` | `34e7` | `load_colours` | Reset the allocator, upload `CL` entries from `BL` | 0 |
| `0254` | `0681` | `set_pointer` | Make bitmap `BX` the mouse pointer | 1 |
| `0258` | `012b` | `text_width` | Width in pixels of the string at `DS:SI` | 0 |
| `025c` | `0796` | `flip` | Show the frame: pointer, back buffer to `A000h` | 3,172 |
| `0260` | `1b3f` | `triangle_clipped` | Flat triangle, clipped to the view | 0 |
| `0264` | `1b62` | `triangle` | Flat triangle, inside the view | 59,064 |
| `0268` | `0788` | `target_back` | Draw into the back buffer | 1 |
| `026c` | `078f` | `target_screen` | Draw into video memory | 0 |

Arguments are registers and main-data-segment variables; there is no stack frame except for the
three fills that take the caller's (`01f4`, `01f8`, `01fc`), which they free with `RETF n`.

## 2. Memory and start-up

`init` (`05b8`), observed in code and on the runtime:

1. Writes the row table at `DS:083a` (200 words, y·320).
2. Allocates 0F39h paragraphs with `int 21h` 48h for the bitmap cache (`DS:1fa7`); failure exits
   through `DS:002c` with code 1900h. Empties it (§5.1).
3. Sets mode 13h (`int 10h`).
4. Palette 0 (`set_palette`), then `066f`: the **back buffer is the EMS page frame segment**
   (`DS:0a1a`, E000h on the runtime). It becomes the target and the segment of both pointer
   descriptors (`SS:fd5b`, `SS:fd69`).
5. Opens `EL2G.GRF` (the name at the driver's paragraph 05D4h + 1Bh) through `DS:00c0` and reads
   the bitmap directory into `DS:5813` through `DS:0094`.
6. Pointer bitmap 0, shows the pointer, clears the back buffer, flips, clears it again.

| Where | What |
|---|---|
| `DS:09e2` | The target segment every primitive draws into. `target_back` copies `DS:09e6` into it, `target_screen` writes A000h. The primitives load `ES` from it |
| `DS:09e6` | The back buffer segment; `flip` copies it to A000h (64,000 bytes) |
| `DS:09e4` | The other segment of `copy_screen` / `copy_panel` |
| `DS:57fd` | Colour byte of the flat primitives |
| `DS:57bd`–`57cf` | The fills' row count, edge slopes and starts (16.16), span routine (§3.1) |
| `DS:57e9`, `57ed` | Fourth vertex of a quadrilateral; the line's \|dx\|, \|dy\| |
| `DS:891d` | Whether the fills swapped their edges into left/right order |
| `DS:1f23`–`1f2f` | The disc's centre, radius and row-table pointers |
| `DS:1fb1`–`1fb7` | The textured fills' colour table (texel → palette); `1fb1` is also their flat colour |
| `DS:0a0a`, `0a0c`, `0a0e` | Pointer bitmap and position |
| `DS:0a18`, `0a1a`, `0a20` | EMS handle, page frame segment, cache depth counter |
| `DS:5813` | Bitmap directory: per bitmap a dword (EMS offset, logical page) |
| `DS:0a22`, `0f21`, `0e1e`, `0f1f` | The colour allocator (§6) |
| `CS:0106`–`012a` | Pointer bitmap header and the saved background's size |
| `CS:010e`–`0122` | The blitter's bitmap header and clip results |
| `CS:351a`–`3589` | The textured fills' vertices, edges, texture coordinates and steps |
| `CS:05d7`·16 + 0Dh | Buffer for the background under the pointer |
| `CS:05de`·16 | Font: glyph rows from 184h (10 bytes a character), widths at 2Eh and 18Eh (11-byte records), palette pointers at 5 |
| `SS:0000`, `SS:fa06` | Offset of the saved background (FFFFh = none); pointer flags (bit 1 shown) |

## 3. Rasterisation

All the flat fills draw horizontal spans through one near routine; everything is integer and
16.16 fixed point.

### 3.1 The span routine (`1a86`, `1a8c`, `1a99`)

Row `DI`, from x = `BX`, `CX` pixels, colour `AH`, rows 320 bytes apart. `1a86` skips rows ≥ 168,
`1a8c` clips x to 0–319, `1a99` does neither. Stores bytes up to a dword boundary, dwords
(`REP STOSD`), then the tail bytes. A length of 0 still fills one pixel when 0 ≤ x < 320; a
negative length fills nothing.

### 3.2 Triangles and quadrilaterals (`1b62`, `1b3f`, `1e74`, `1e41`)

| Rule | Observed |
|---|---|
| Vertex order | Triangle: sorted by y (signed). Quadrilateral: its perimeter is 0–1–3–2; the vertices are cycled along it until vertex 0 is the topmost |
| Edge slope | dx·256 / dy with `IDIV`, the quotient shifted to 16.16. On overflow the `int 0` handler resumes at dx·64 / dy (quotient ×4), on a second overflow the dividend's low word ×4 |
| Edge start | x0 + ½ − slope/2, then each row first adds the slope: rows are sampled half a row down |
| Fill rule | The left edge is taken ½ pixel left, the right ½ pixel right; the span runs from the left edge's integer part for (right − left) pixels |
| Edge order | The two edges from the top are ordered by slope (signed compare of the 16.16 values); later edges by comparing the accumulators |
| Horizontal edges | dy = 0 stores a slope of dx·65536 + sign and no start: a later phase reads the previous start (stale memory) |
| Clipping | The clipped entries only switch the span routine to `1a86`. A polygon with every vertex outside the 320×168 view is not drawn at all, even if it covers it |

### 3.3 Lines (`28f9`)

Run-length Bresenham in eight variants. The error starts at −(major/2); a run of pixels is drawn as
one span on its row when the error turns positive. Leftward lines step x down before each pixel.
Steep lines draw one pixel per row. A line of one point draws nothing. Only x is clipped. The
comparisons after `ADD` test the true signed sum (overflow flag included).

### 3.4 Discs and blobs (`2f5a`, `2f4e`, `2dd1`)

Diameters below 6 are fixed shapes from the jump table at `CS:2e4e` (rows of 1, 2–2, 1–3–1, 3–3–3,
2–4–4–2, 1–3–5–3–1 pixels), drawn only when the centre is at least 2 pixels inside the view.
Larger discs are a midpoint circle that writes each row's (left x, width) for the upper and lower
halves into a table on the stack, then fills the rows inside the view, clamped to the screen
width. Blobs are rows of half-widths from `CS:2c11` (2w − 1 pixels each), ended by a negative byte.

### 3.5 Scanline crossing lists (`3108`)

The caller's `frame_e76` frame ([MATH3.md](MATH3.md)) holds, per row, the offset of the end of its
crossing list (0, 2, 4 or 6 crossings) and the crossings in arrays 150h bytes apart. Rows are
filled bottom up: the crossings sorted by insertion and filled in pairs, clamped to 0–320 unless
`[BP+0e74h]` ≠ 0 says the polygon is inside. An empty row at or above `[BP+0e76h]` ends the fill.

### 3.6 Polygon masks (`3399`, `3343`)

The model renderer's `frame_5598` frame holds, per row, a list of (x, polygon mask) crossings and
a mask word per row. Along a row the masks of the crossings passed are XOR-ed together (starting
from the rows below); each stretch is filled with the colour byte the combined mask indexes in
the frame (`[BP+mask+8]`), so overlapping polygons resolve by a lookup, not by drawing order. A
negative mask indexes below the frame, into the words the routine pushed; the port models those
pushes.

## 4. Text, screen and pointer

`glyph` draws 9 rows of bits (most significant leftmost, a row ends at its last set bit), unclipped,
y < 191, and returns the byte after the rows as the advance. `text_width` sums the signed widths
at font offset 18Eh + 11·(c − 20h), skipping bytes below 20h and from A0h. `fill_rect` fills by
bytes, words or dwords as the width is odd, twice odd or a multiple of 4; at least one row.

`flip` calls the main program's pointer shape (`DS:0140`) and position (`DS:004c`) code, saves
the back buffer under the pointer (`save_under`), draws the pointer through `DS:0100` (which calls
`blit_pointer`), copies the back buffer to A000h and puts the background back (`restore_under`).
`DS:09fe` gets `DS:09fc`, which is cleared (a frame counter, *inferred*).

## 5. Bitmaps and textures

### 5.1 The cache (`0173`)

Bitmap n's slot is the word at `cache:F000h + 2n` (FFFFh = not cached). On a miss the driver saves
the EMS mapping (`int 67h` 47h), maps the bitmap's logical page into physical page 0 (44h),
copies the size word and the bitmap from the page frame to the cache's free offset, and restores
the mapping (48h). The cache is a ring: the numbers of cached bitmaps from `F1C2h` to `F383h`, at
most E1h of them, data below F000h; the oldest are evicted until a new one fits. The miss path was
observed on the runtime (16 misses in 90 s, and 982 forced in the tests).

### 5.2 The blitter (`02d3`)

Bitmap header (8 bytes): flags (bit 0 transparent, colour 0 not drawn; bit 1 compressed), height,
width (word), x and y offsets. The target descriptor at `SS:DI`: far pointer, clip left, top,
width, bottom, gap after each row. Clipped against it, copied row by row (`0476`). A compressed
bitmap is first unpacked into a buffer on the stack: literal runs (a byte n < 80h, then n bytes)
and back references (a byte with bit 7 set: 1–16 bytes from up to 2 KB back).

### 5.3 Texture mapping (`3635`, `44d1`, `5448`)

The bitmap is copied into a buffer of 256-byte rows on the stack (`CS:357d`, 4100h bytes) through
the colour table at `DS:1fb1`, halved in both directions as often as the polygon's screen extent
allows (`CS:357c`): halve while the extent is below (width + height)·¾ / 2ᵏ. The quadrilateral
copies it in one of 8 orientations (`DI` bits 12–14, adjusted by its vertex rotation). Then, per
vertex order, a case sets the edges' screen x and texture (u, v) and their per-row steps (through
the divider `35cf`), and the row mapper fills each row with the texel at (u, v) interpolated
linearly from the row's ends (per-pixel steps by `IDIV` of (end − start) by the width: affine, no
perspective). For bitmaps other than D0h–D8h steep edges are padded with up to 16 pixels of the
flat colour `DS:1fb1` outside them. A non-convex quadrilateral is filled flat instead (`1e41`).
Measured quirk: with no halving, the shifts leave the flags of an earlier instruction for the
`JNE`s that guard against zero sizes, and orientation 0 then copies one extra texel column.

## 6. The palette

The allocator maps 12-bit colours (0RGBh) to DAC entries. Per entry 0–254 a dword at `DS:0a22`+4n:
state (1 to be freed, 2 new, 3 free, 4 in use, 5–6 recently used), DAC entry, colour; per colour
its slot at `DS:0f21` (FFh = none). Reset: everything free, the greys 000h, 111h … FFFh held by
slots 0Fh … 0 on entries 10h–1Fh. `colour` returns a known colour's slot (marking it recently used)
or takes the next free one; `colours_tick` ages the slots each frame; `upload_colours` hands new
colours entries 80h–FFh (the nearest known colour when none is free, found by stepping red, then
green, then blue down) and writes them to the DAC (`3C8h`, `3C9h`) after the next vertical retrace
(`3DAh` bit 3), interrupts off. The palette loaders write 768 or 384 bytes in chunks of 64
colours, each after a retrace, with interrupts on between chunks.

## 7. The draw-command stream (the hand-off from the main program)

The main program's 3D renderer does not call the driver directly: it leaves a **direct-threaded
command stream in the stack segment**, each item a handler offset followed by its operands, walked
with `LODSW; JMP AX` (`FRONTIER:2b1a`, `2b59`, …). Observed handlers (`FRONTIER:2b25`–`2f00`): a
scanline-list fill (colour slot → `colour_entry` → `DS:57fd` → `fill_rows`), the polygon-mask fill
(16 colour slots: 8 pushed, 8 into `DS:07bc`…, then `fill_masks_args` or `fill_masks` through
`DS:07e0`), line, triangle, quadrilateral, disc, blob, textured triangle and quadrilateral (7 colour
slots into `DS:1fb1`…, then the fill). `DS:1f99` gets each item's size. The depth order is the
order of the stream (`EL2MATH3`'s `init_list`, [MATH3.md](MATH3.md)); the driver draws in that order
(painter's algorithm), except inside `fill_masks`, which resolves overlaps by mask. Colours are
allocator slots until the handler converts them.

## 8. The port

`crates/game/src/video.rs` and `video/` (`raster.rs`, `shapes.rs`, `screen.rs`, `sprite.rs`,
`texture.rs`, `texture_cases.rs`, `palette.rs`, `driver.rs`). Each port takes the 386 state, a
memory bus and the VGA ports ([`Io`](../crates/game/src/video.rs)), runs the routine to its
`RETF`, and leaves every register (32-bit), segment, the interrupt flag and every byte written as
the original, with the original's instruction count. Port accesses carry the instruction count
before them, so the retrace waits end where the original's do. `crates/hooks/src/video.rs`
installs the ports (`Machine::hook_dev`, which also gives a hook the devices), staging memory in
a reusable 1 MB shadow and the VGA state on a copy, committed only if the call fits before the next
interrupt.

**Calls out of the driver.** Where the driver calls something else (the main program's pointer
code, DOS, the BIOS, the EMS driver, its own exports through a slot), the port ends at the call:
the routine called runs as itself and the code after the call is ported as its own entry (the
`name@offset` entries in `EXPORTS`, 34 of them). Every instruction of the driver's own code runs
natively; the `INT`s push the arithmetic flags as the instructions before left them.

**Left to the original** (declines, all rare): a direction flag set, a disc with no rows in view
(the original fills 65,536 rows), a palette search with no known colour below a new one (the
original loops for ever), a bitmap with no end to its data, and a texel read that wraps below the
texel buffer into the fill's own pushed words. None occurred in 90 s of play.

## 9. How parity was proven

1. **Fuzzing against the original** (`crates/hooks/tests/mcga3_parity.rs`): the game booted on the
   runtime, then each export and piece called with random and edge inputs (coordinates on, near
   and far off screen, random clip descriptors, colour states, palette data, frames for the
   scanline fills); the original runs with its memory writes and port accesses recorded, then the
   port on the same state. Registers, segments, every byte written (the frame buffer included),
   the port accesses in order and time, the VGA state and the instruction count are compared.
   600–20,000 calls per entry match. Two chain tests cover the bitmap cache's miss path across the
   real EMS driver: 1,500 blits (982 misses) and 400 textured fills (234 misses).
2. **Shadow verification in play** (`cargo run --release -p frontier-hooks --example native_boot
   -- game-data/original 90 --verify`): **2,968,622 calls, 0 mismatches** in 90 s; 7,624 more had
   a timer tick due inside (chiefly the palette uploads, whose retrace waits span ticks) and were
   left to the original; none for lack of a ported path.
3. **Identical frames**: with the maths, sound and video ports replacing the original
   (`--ports math3,adlib,video`), every 10 ms frame of 90 s matches a run with no ports (7,337,850
   ported calls). With every port in `install_all` as on main, the main program's random-number
   ports make the frames differ at 28.79 s, with or without the video ports.

Not reached in play, so resting on the fuzzing alone: `char_width`, `clear_view_0`, `copy_panel`,
`copy_screen`, `fill_rect_masked`, `fill_masks`, `load_colours`, `load_high_colours`, `pixel`,
`text_width`, `target_screen`, `triangle_clipped`, and the palette upload pieces.

## 10. The draw-command and WebGPU boundary (intended)

The plan for drawing at higher resolutions is a draw-command layer between the game and the
rasteriser, and a WebGPU backend that renders the commands. Nothing of it is built yet. The ports
are split so that it can be added without touching the exact path:

- **Where commands come from.** Either the driver's exports (each port's entry reads the original
  register interface into geometric inputs: vertices, a colour index, a texture number and
  orientation, the target segment, the clip rectangle) or, one level up, the command stream of §7.
  The stream is the better tap: it carries the model renderer's polygons before clipping and
  before the 16-bit projection, in depth order.
- **Command set.** `Span(y, x, len, colour)`, `Triangle(v×3, colour, clipped)`, `Quad(v×4,
  colour, clipped)`, `Line(v×2, colour)`, `Disc(centre, diameter, colour)`, `Blob(centre, shape,
  colour)`, `RowList(rows: crossings, colour)`, `MaskRows(rows: (x, mask), colours by mask)`,
  `TexturedTriangle(v×3, bitmap, colour table)`, `TexturedQuad(v×4, bitmap, orientation, colour
  table)`, `Glyph(char, x, y, colour)`, `Blit(bitmap, x, y, descriptor)`, `FillRect`, `Clear`,
  `CopyRect`, `Flip`, and palette updates (`Dac(entry, rgb)`).
- **Exact mode.** The commands run through the ported rasterisers into the 320×200 index buffer and
  the DAC, as now; frames stay identical to the original and the tests keep passing.
- **HD mode.** The backend renders the same commands at any resolution into an index texture (one
  `u8` per pixel, the palette index) and resolves it through a 256-entry palette texture in a
  second pass, so palette effects stay exact. It keeps the painter's order of the stream, draws
  `MaskRows` by the same mask lookup per pixel, and maps textures from the decoded `EL2G.GRF`
  bitmaps through the colour table. A toggle switches between the exact 320×200 buffer and HD.
  The vertices in HD would come from a higher-precision projection than `EL2MATH3`'s 16-bit one
  (plan step 4); every difference from the exact path is a `Deviation:` ([DEVIATIONS.md](DEVIATIONS.md)).

## Open questions

- The roles of the main program's pointer routines `DS:0140`, `004c`, `0100` (shape, position and
  hot-spots, drawing) in detail, and of `DS:09fc`/`09fe`.
- The full list of command-stream handlers and the stream's layout before `FRONTIER:2b1a`; who
  writes it (the model renderer in `EL2MATH3` and the main program).
- Why bitmaps D0h–D8h are not padded, and which bitmap DFh is (its halving is biased by one).
- Not compared with DOSBox frames yet.
