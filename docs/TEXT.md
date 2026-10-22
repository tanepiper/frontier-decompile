# Text

This covers how the game stores and expands its text: the text ids, the main dispatcher, the
expander and its byte codes, and the modules' string handlers. Evidence: the Ghidra 12.1.4
disassembly listings, and execution on the reference runtime: every string of ten modules compared
with the original. The formatters (`3000h`–`3FFFh`) are not traced beyond the one that names use.

## Text ids and the dispatcher: `1000:e89d` (slot `DS:0014`)

A text id in `AX` is expanded to `SS:DI`. The dispatcher saves `SI` and `ES` and doubles `AX`, then
branches (*observed*):

| Id | Source |
|---|---|
| below `3000h` | The main program's table at `1000:e4dd` (`ES` = the main code segment) |
| `3000h`–`3FFFh` | A formatter, `CALL CS:[8915h + 2·id]`, with `DS:07c0`–`07cb` saved around it (numbers, dates, names, *inferred*). `3016h` (`1000:eaf3`) copies the string at the far pointer `[DS:07e4]` |
| `4000h`–`7FFFh` | The table at the far pointer `[DS:5703]` |
| `8000h` and up | A module's own strings: module slot `(id >> 9) & 3Fh` (from the doubled id), handler `+28`, with the module's lock count held around the call |

## The expander: `1000:e969` (slot `DS:005c`)

`ES:SI` points at a table of word offsets relative to the table, and `AX` is twice the string's
index. The string is copied to `SS:DI`, and `DI` ends just past its terminating 0.

| Byte | Effect |
|---|---|
| 01h–7Fh, 80h–FCh | copied |
| 00h | copied, then return |
| FDh | `1Eh` and the next byte: a one-byte control code |
| FEh | `1Fh` and the next word: a two-byte control code |
| FFh | the next word (big-endian) is a text id, expanded in place through the dispatcher |

## The modules' string handlers (handler `+28`)

Most modules use one shape: `AND AX,3FEh`, point `ES:SI` at the module's own table, `CALLF [DS:005c]`,
`RETF`. They differ in the table and in the register that loads `ES`: those loading it through `DX`
leave `DX` holding the module's segment. Ported in `frontier_game::text`:

| Handler | Table | `ES` loaded through |
|---|---|---|
| `EL2M00:4e19` | `0241` | `SI` |
| `EL2M03:3dd6` | `0255` | `DX` |
| `EL2M05:0170` | `00a3` | `DX` |
| `EL2M07:06ec` | `00a3` | `SI`; string 1Fh (`AX` = 3Eh) runs a formatter instead (not ported) |
| `EL2M10:0e17` | `00a3` | `DX` |
| `EL2M11:093c` | none: a lone `RETF` | |
| `EL2M14:2d84` | `030b` | `SI` |
| `EL2M15:0d1b` | `00a3` | `DX` |
| `EL2M17:0b60` | `014e` | `DX` |
| `EL2M18:4cfa` | `00a3` | `SI` |

`EL2M02:23f2`, the galaxy map's handler, is ported (`m02_text`). It is where stars get their
**procedural names**:
- **String 0:** star `DS:07d0` in sector (`DS:07c8`, `DS:07cc`). A catalogue sector (the 47 at
  `EL2M02:00a2`) takes the real name from `EL2M02:0504`. Any other star gets three of the 32
  syllables at `EL2M02:01c6` ("en", "la", "can", …), chosen by a hash of the sector and the index
  (`galaxy::star_name_seed`: adds and rotations by 3, 5, 4 and by the index), with the first
  letter made a capital.
- **String 1FFh:** the same for the star whose 32-bit system id is `DS:07c0`.
- **String 1FEh:** its sector relative to Sol's as ` (x,y)`.
- **Strings 1–1FDh:** the table at `EL2M02:0504`.

Other shapes, not ported yet:
- `EL2M09:11d1` makes a generated name from the far pointer `[DS:07e4]`, through `EL2M09:0f92`
  ([NAMES.md](NAMES.md)).
- `EL2M12:4c26` forwards to module 18's handler.

## The formatters (`3000h`–`3029h`, `1000:e9b2`–`ed40`)

Each formatter reads an argument slot (`DS:07c0`, `07c4`, `07c8`, sometimes `07e4`, `07e8`,
`1fc5`), which the dispatcher saves and restores around it. Short entry stubs load a slot and
jump to a shared body. Ported (`frontier_game::text`, *observed*):

| Ids | Body | Output |
|---|---|---|
| `3000h` | `e9b2` | nothing (`RET`) |
| `3001h`–`3003h`, `301Dh`, `301Eh` | `1000:b607` | signed decimal |
| `3004h`–`3006h`, `3007h`–`3009h`, `300Ah`–`300Ch` | `1000:b609` with `BL` = 1, 2, 3 | 1, 2 or 3 decimals: the point is moved in before the last digits |
| `3013h`–`3015h` | `1000:b5a3` | 8 hex digits (A–F as the font's codes 8Bh–90h) |
| `301Fh`–`3021h` | `eb3f` | a tonnage: the number and `t`, or `-` for 0 |
| `3016h`, `3017h` | `eaf3`, `eafc` | the strings at `[DS:07e4]`, `[DS:07e8]` |
| `3018h`–`301Ch` | `ec38`–`ec58` | the text id held in a slot, expanded through the dispatcher |
| `300Dh`–`300Fh`, `3029h` (`DS:1fc5`, the game date) | `ea14` | a date `d-Mon-yyyy` from a day count: 1461-day cycles, the first year a leap year, a day skipped after 28 February in the others, so one day-of-year table (`1000:e529`) and the month names (`1000:e4f9`) serve all |
| `3026h`–`3028h` | `ea14` | the same without the year |
| `3010h`–`3012h` | `eab9` | `h:mm:ss`: `divide32` (`EL2MATH3`, slot `02f4`) by 0:C22Eh, then `DIV 3600` and `DIV 60` |

The number printer (`1000:b609`) prints `-` for a negative value. It then finds the first power of
ten (10⁹ down) that fits, through a cascade of 32-bit compares, and finds each digit by
subtracting until a borrow. `BL` = 3, 2, 1 forces at least 4, 3, 2 digits. A final `STOSW` writes
the units digit and the 0.

| `3023h`–`3025h` | `ecc1` | a speed `v`: up to `4_705Dh`, `3v + v/2 − v/16` (×3.4375) with one decimal and `" km{"` (`1000:e805`); above that, `w = v/128`, `w + w/4 − w/32` with two decimals and `" km|"` (`1000:e80a`). `{` and `|` are the font's unit glyphs (*inferred*) |

Not ported: `3022h` (`ec60`, which calls slot `DS:0038`).

## The dispatcher's port

`frontier_game::text::dispatch_port` covers the main table, `[DS:5703]` and module strings (when
the module is resident and its handler is ported), and the `3016h` formatter. Other formatters
decline. The main code segment is taken as `DS − 1615h` (docs/OVERLAYS.md §2). Nesting deeper
than 16 strings declines, so a garbage table (`[DS:5703]` before the game sets it) cannot recurse
without end on the host.

## How parity was proven

`crates/hooks/tests/text_parity.rs` loads every module through its own thunk and asks each ported
handler for strings 0–1FFh. Each request runs the port and the original and compares registers,
every byte written and the instruction count. A string with an `FFh` escape other than `3016h` is
declined (the original runs it).

| Module | Compared | Declined | Mismatches |
|---|---|---|---|
| `EL2M02` (star names: random sectors, indices and system ids, a third from the catalogue) | 501 | 11 | 0 |
| `EL2M00` | 508 | 4 | 0 |
| `EL2M03` | 411 | 101 | 0 |
| `EL2M05` | 406 | 106 | 0 |
| `EL2M07` | 486 | 26 | 0 |
| `EL2M10` | 495 | 17 | 0 |
| `EL2M11` | 512 | 0 | 0 |
| `EL2M14` | 408 | 104 | 0 |
| `EL2M15` | 459 | 53 | 0 |
| `EL2M17` | 449 | 63 | 0 |
| `EL2M18` | 374 | 138 | 0 |

The dispatcher (`dispatcher_matches_original`), with all ten modules loaded, was asked for main
ids 0–2FDh, `3016h`, `[DS:5703]` ids `4000h`–`41FBh` and 64 strings of every module: Every
formatter `3000h`–`3029h` was asked 24 times with random arguments of every size and sign:
**1,882 compared, 0 mismatches**, and 133 declined (`3022h`, and module strings that reach
unported text).

## Open questions
- The formatters `3022h`–`3025h`.
- The control codes `1Eh` and `1Fh`: the renderer interprets them (colour, position, *inferred*).
