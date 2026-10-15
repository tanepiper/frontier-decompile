# `EL2MATH3`: the software floating-point library

This covers the maths overlay's leaf routines: what they compute, the number format they share,
and how the native ports were proven. Evidence: Ghidra 12.1.4 disassembly
(`tools/ghidra/ExportDisasm.java`, output in `analysis/`), the bytes of the main program's
divide-error handler, and execution of the original on the reference runtime
([RUNTIME.md](RUNTIME.md)). That covers fuzzed calls and shadow verification of every real call
during 60 s of play. The larger routines (`v0300`, `v0310`, `v0314`, `v0328` and others) are not
traced yet.

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

Not ported yet: `v02c8`/`v02cc` (they call `v031c`), `v0320`/`v031c` (a square-root-style lookup
in a table at `DS:6dff`, *inferred*), the `SUB SP` frame allocators `v0278`/`v0274`, and the large
routines.

## How parity was proven

1. **Fuzzing against the original** (`crates/hooks/tests/math3_parity.rs`). The game is booted on
   the runtime until `EL2MATH3` is resident and the `int 0` handler is installed. Each routine is
   then called 4,000 times through `Machine::call` with random and edge-value registers and the
   real DS. Every register, `DS:09dc` and the instruction count are compared. All 21 match. Inputs
   where the original never returns are skipped: 40 of 4,000 for `norm_unsigned`.
2. **Shadow verification in play** (`cargo run --release -p frontier-hooks --example native_boot
   -- game-data/original 60 --verify`). Every call of a ported routine during 60 virtual seconds
   (intro, menu, start at Ross 154, cockpit) computes the port's result, runs the original and
   compares registers, `DS:09dc` and the instruction count. Result: **49,740 calls, 0
   mismatches**. Another 24 calls had a timer tick due inside them and were left to the original.
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

- Stack contents below `SP`: the originals of `to_fixed32_raw`, `to_fixed64` and the
  divide-overflow path write words below the caller's stack pointer (a `PUSH`, the interrupt
  frame). The ports do not. No difference has been seen, but a caller reading stale stack would
  notice.
- Roles of the routines in the game (which callers do 3D, orbits, trade). These come from
  tracing the callers.
