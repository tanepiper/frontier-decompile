# `EL2MATH3`: the software floating-point library

This covers the maths overlay's leaf routines: what they compute, the number format they share,
and how the native ports were proven. Evidence: Ghidra 12.1.4 disassembly
(`tools/ghidra/ExportDisasm.java`, output in `analysis/`), the bytes of the main program's
divide-error handler, and execution of the original on the reference runtime
([RUNTIME.md](RUNTIME.md)). That covers fuzzed calls and shadow verification of every real call
during 60 s of play. The larger routines (`v0300`, `v0310`, `v0314`, `v0328` and others) are not
ported yet.

## Number format

The DOS build does its maths with paired registers: a **signed mantissa** (16-bit in `AX`, or
32-bit in `DI:AX`) and a **binary exponent** (`DX`). A second operand sits in `BX` (or `BP:BX`)
with its exponent in `CX`. Results are normalised by shifting left until one more shift would
overflow, so the mantissa uses 15 (or 31) significant bits plus the sign. Arguments and results
are in registers, not on the stack. Callers reach the routines through the main vector table
(`DS:0270`–`032b`, [OVERLAYS.md](OVERLAYS.md) §2) with `lcall [slot]`.

## Divide overflow: `DS:09dc`

Routines that divide store a resume offset in `DS:09dc` just before `IDIV`. The main program's
`int 0` handler at `1000:cdeb` is installed for both `int 0` and `int 4` by `1000:cdf9` and
restored by `1000:ce30`. It enables interrupts, replaces the return IP in the interrupt frame
with `[DS:09dc]`, and returns. A divide overflow therefore continues at the chosen offset in the
same code segment, with `AX` and `DX` as they were before the `IDIV`, since the 386 leaves them
unchanged on `#DE`. The handler assumes DS is the main data segment. The main program also uses
this mechanism: ten `mov word [09dc], imm` sites in `FRONTIER.EXE`, for example `1000:0293` and
`1000:1831`.

## Ported routines

All are in `crates/game/src/math3.rs`, register-exact: every register the original writes,
including scratch values, plus the `DS:09dc` store. Each port also returns the number of
instructions the original executes for those inputs, through its `RETF`, including the 9
instructions of the `int 0` handler after a divide overflow. The runtime charges that count, so
the ports do not move interrupts ([RUNTIME.md](RUNTIME.md)).

| Slot | Address | Port | What it does |
|---|---|---|---|
| `02b4`, `02b8` | `EL2MATH3:1cf5`, `1cfe` | `mul` | `DX:AX = (AX × DX) << 1` (Q15 × Q15), `BX += CX` |
| `0288` | `EL2MATH3:1d07` | `div_norm` | `BX` = 0: `CX += 1` only. Else `CX -= AX`, `DX:AX = (DX << 16) >> 2`, `IDIV BX` (resume `1d1f`). Then doubles `AX` if −64 < `AH` < 64, else `CX += 1` |
| `02b0` | `EL2MATH3:1d2e` | `div` | `CX -= AX`, `DX:AX >>= 2`, `IDIV BX` (resume `1d40`) |
| `02d0` | `EL2MATH3:1d41` | `sub` | `NEG BX`, then falls into `add` |
| `027c` | `EL2MATH3:1d43` | `add` | 16-bit float add. Aligns by `SAR BX,CL`, with the 386's 5-bit count mask (an exponent gap of 33 shifts by 1), then adds and normalises |
| `02d4` | `EL2MATH3:1d63` | `sub32` | Negates `BP:BX`, then falls into `add32` |
| `0280` | `EL2MATH3:1d6d` | `add32` | 32-bit-mantissa float add (`DI:AX`, `BP:BX`) |
| `0284` | `EL2MATH3:1da1` | `add_unsigned` | Unsigned mantissa add (`SHR`), halves once on bit 15 |
| `02c4` | `EL2MATH3:1db6` | `from_u32` | 32-bit integer → float, exponent `BX` (−7 base, `+27h`) |
| `02bc` | `EL2MATH3:1dd2` | `from_i32` | Signed 32-bit integer → float (`+26h`) |
| `02c0` | `EL2MATH3:1dec` | `norm_i32` | Normalise signed `DX:AX`, exponent from −6 |
| `028c` | `EL2MATH3:1e03` | `to_fixed32` | Float → 32-bit fixed point, shift `1Fh − CX`. A negative shift saturates to `7FFF:FFFF` or `8000:0000` |
| `0290` | `EL2MATH3:1e22` | `to_fixed32_raw` | Same with no saturation. The count is an unsigned loop count |
| `029c` | `EL2MATH3:1e34` | `to_int16` | Float → 16-bit integer, saturating to `7FFFh`/`8000h` |
| `0294` | `EL2MATH3:1e4a` | `to_fixed32_unsigned` | Unsigned mantissa → 32-bit fixed point, saturating to all ones |
| `0298` | `EL2MATH3:1e65` | `to_fixed64` | Float → 64-bit fixed point in `DX:CX:BX:AX`. `BP` is preserved |
| `02a8` | `EL2MATH3:1e80` | `norm_unsigned` | Normalise unsigned `AX`. **Never returns** if `AX` has bit 15 set (observed: the loop clears `AX`) |
| `02a0` | `EL2MATH3:1e8d` | `norm16` | Normalise signed `AX`, exponent `DX` |
| `02a4` | `EL2MATH3:1e9a` | `norm32` | Normalise signed `DX:AX`, exponent `CX` |
| `02ac` | `EL2MATH3:1eaf` | `norm_positive` | Shift positive `AX` left until bit 15 is set |
| `031c` | `EL2MATH3:0967` | `sqrt32` | √(`DX:AX`) by table: the top bits pick an index into the square-root table at `DS:6dff` by one of six shifts, and the entry is shifted back. `DX` = index |
| `0320` | `EL2MATH3:0952` | `length3` | Vector length: `DX:AX` = `AX`² + `BX`² + `CX`², then falls into `sqrt32` |
| `02c8` | `EL2MATH3:0922` | `sqrt_float` | √ of a 16-bit float: mantissa doubled, exponent halved (odd: mantissa halved back, exponent rounded up), `CALLF [031c]`, root halved |
| `02cc` | `EL2MATH3:0938` | `sqrt_float32` | The same for a 32-bit mantissa, then `CALLF [02a8]` to normalise |
| `0278` | `EL2MATH3:0ba1` | `frame_e76` | Gives the caller a 0E76h-byte stack frame (`BP` = `SP`), first 150h bytes zeroed (`REP STOSD`), `[BP+E70h..E75h]` = `FFFF 00A8 FFFF` |
| `0274` | `EL2MATH3:0bcf` | `frame_5598` | Gives the caller a 5598h-byte stack frame, clearing 21 × (8 words 80h apart + 16 bytes), `[BP]` = `00A8`, `[BP+2]` = `FFFF` |
| `02e0` | `EL2MATH3:09e9` | `rotation_02e0` | 3×3 Q15 rotation matrix at `ES:SI` from the angles at `DS:1f35` and `DS:1f37`, using the sine table at `DS:5bff` |
| `02dc` | `EL2MATH3:0ac9` | `rotation_02dc` | The same products in another layout |
| `0314` | `EL2MATH3:19c2` | `project` | Perspective projection of (`EAX`, `EBX`, `ECX`), see below |
| `030c` | `EL2MATH3:1ba7` | `init_list` | Starts the list at `SS:[DS:1f3b]`: a dword from `[BP-62h]`, then `ES:[SI+8]`/2 − 2 words of `8080h` |
| `0300` | `EL2MATH3:0d2b` | `transform_vertex` | Transforms a model vertex into its record (partly ported, see below) |
| `02fc` | `EL2MATH3:0ccc` | `project_vertex` | Transforms if needed, then projects a vertex record |
| `0304` | `EL2MATH3:0c35` | `vertex_position` | World position of a vertex, transforming it first if needed |

The two tables are static data in the main program's data segment. `DS:6dff`: 2048 words,
entry *k* = 2048·√(*k*/2) (0, 1448, 2048, 2508, 2896, …). `DS:5bff`: 2048 words for a half turn
of sine (0, 50, 100, 150, 201, …); bit 15 of the angle selects the negative half, and adding
4000h gives the cosine. The two square roots of floats reach `sqrt32` through `CALLF [031c]`;
the ports assume the slot holds `EL2MATH3:0967`, as it does once the overlay is loaded.

Not ported yet: `v0308`, `v0310`, `v0318`, `v0324`, `v0328`, `v02d8`, `v02e4`, `v02e8`, `v02ec`,
`v02f0`, `v02f4`, `v02f8`, `v0270`, and the vertex types above 10 in `v0300`.

A port may also *decline* an input (`math3::DECLINE`): a path not ported yet, or one where the
original never returns. The hook then discards what the port staged and runs the original, so
a routine can be ported a path at a time.

## The 3D routines

These work for a caller that has taken a 5598h-byte frame from `frame_5598` (`BP`). Fields of
that frame, as offsets from `BP` (names *inferred* from use):

| Offset | Use |
|---|---|
| −DCh | Far pointer to the model's vertex definitions (`LFS`) |
| −D4h | Scale: coordinates are shifted right by 17h minus this |
| −9Ah | Byte: the mark a vertex projected in this frame gets |
| −44h | Words: records that negative vertex numbers copy from |
| −24h … −14h | 3×3 matrix of words (m0 … m8) |
| −10h, −0Ch, −8 | Translation dwords |

Vertex records are 20h bytes in the stack segment, record 0 at `SS:[DS:1f39]`. A vertex `AX` is
`index × 20h`; odd records are the mirror partners of even ones and share their definition (two
words per pair at `FS:[SI + index×2]`: type byte `BH`, then parameters). Record fields: `+0`
screen x, y; `+4/+8/+C` world position (dwords); `+12h` state word (0 = not done, FFFFh =
transformed, the frame mark in the low byte once projected); `+14h/+18h/+1Ch` rotated position.

`transform_vertex` dispatches on the signed type byte:

| Type | Address | Vertex |
|---|---|---|
| ≤ 2 | `0d58` | Definition bytes (x, y, z)×100h rotated by the matrix and scaled; the partner is the same with x negated |
| 3–4 | `0eca` | Screen midpoint of two projected vertices, depth the nearer (each partner from the definition word's two bytes, then those xor 1) |
| 5–6 | `0fd7` | The negation of another pair |
| 7–8 | `1072` | Another pair plus a random offset per axis from the RNG at `DS:1fd9`, `SAR` by the definition's low byte |
| 9–10 | `1158` | As ≤ 2, but the partner subtracts the x column once instead of twice |
| > 10 | `123b` | More combinations (averages, sums, a scaled interpolation, a call into `v0328`): not ported yet |
| negative `AX` | `0c72` | Copy a record named by the frame's table at `BP-44h` |

Then the record's world position is the rotated one plus the translation. `project_vertex` also
projects it: depth below 40h gives the screen position `8002h, 8002h`, otherwise `project` and
an offset to the screen centre (x + A0h, 54h − y), and the frame mark.

`project` brings x and y within ±z by `SAR`-ing all three (two loops, chosen by unsigned compares
against z and −z), then picks a reciprocal from the 1024-word table at `EL2MATH3` offset `0122`
at one of four depth scales and multiplies. At the two nearest scales x and y are halved together
until both are within ±1F00h. A negative depth that stays at or above 8000h makes the original
loop for ever (the port declines it).

### Port interface

A port takes the 386 state (all eight 32-bit registers, `DS`, `ES`, `SS`, `CS:IP`, flags) and a
memory bus by linear address, and runs the whole routine through its `RETF`. In the running
game the bus stages writes, so a port that declines (an interrupt due inside) leaves no trace.
Memory below the final stack pointer is not reproduced: it is dead, since any interrupt may
overwrite it. Return addresses and saved registers that a routine pushes and pops again within
its frame *are* modelled through memory, because the original reads them back (the frame
allocators clear downwards over their own return address when `DF` is set).

## How parity was proven

1. **Fuzzing against the original** (`crates/hooks/tests/math3_parity.rs`). The game is booted on
   the runtime until `EL2MATH3` is resident and the `int 0` handler is installed. Each routine is
   then called 4,000 times with random and edge-value 32-bit registers, random angles at
   `DS:1f35`/`1f37`, `DF` set one time in eight, the real DS, and scratch segments for the stack
   and `ES`. The original runs first with the runtime's memory write log on; its writes are then
   undone and the port runs on the same state. All registers, segments, the return address,
   every byte either wrote and the instruction count are compared. All 34 match. (With random
   registers the 3D routines run on garbage frames; that is still a valid comparison.) Inputs where
   the original never returns are skipped: 38 of 4,000 for `norm_unsigned` (`AX` bit 15 set),
   about 1 in 8 for `frame_e76` (`DF` set: it clears its own return address).
2. **Shadow verification in play** (`cargo run --release -p frontier-hooks --example native_boot
   -- game-data/original 60 --verify`). Every call of a ported routine during 60 virtual seconds
   (intro, menu, start at Ross 154, cockpit) computes the port's result, runs the original and
   compares the same things. Result: **332,598 calls, 0 mismatches**; 104 more had a timer tick
   due inside them and were left to the original. The most frequent are `length3` (187,791),
   `frame_e76` (85,156), `mul` (10,593), `div_norm` and `to_int16` (about 7,300 each).
3. **Identical frames.** The same run without `--verify`, with the ports replacing the original,
   gives frames identical to a run with no ports at every 10 ms slice for 60 s.

## Flags

The ports do not model flags. A static scan shows that no caller needs them. Every `lcall [slot]`
to the 21 ported slots and to `02c8`/`02cc` in `FRONTIER.EXE` and all overlays was found by byte
pattern, which also finds code that Ghidra did not put in a function (36 of the sites, in `EL2M00`,
`EL2M02`, `EL2M03` and `EL2M05`). Each site was disassembled forward, following jumps and near
calls, until every flag was overwritten:

| Outcome | Sites |
|---|---|
| All flags overwritten before any flag read | 46 |
| The next flag-relevant instruction is another `lcall` into `EL2MATH3`, whose routines all start by writing the flags (`OR`, `CMP`, `IMUL`, `NEG`, `SHL`, `ADD` after two `NOT`s) | 47 |
| `RETF` straight after the call: `v02cc` ends with `lcall [02a8]; retf`, and its own 3 call sites are covered by the rows above | 1 |

Far calls through a computed address (`CALLF [BP+n]`, about 25 in the listing) are not covered,
since their targets are not known statically. The 60 s run with identical frames (above) agrees
with the scan.

## Open questions

- Which axis conventions the two rotation matrices use (`02e0`, `02dc`); the layouts are known,
  the meaning comes from their callers.
- Roles of the routines in the game (which callers do 3D, orbits, trade). These come from
  tracing the callers.
