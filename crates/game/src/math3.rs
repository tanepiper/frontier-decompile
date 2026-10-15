//! `EL2MATH3`: the software floating-point library (docs/MATH3.md).
//!
//! The DOS build does its 3D and orbital arithmetic on pairs of a normalised signed mantissa
//! (16-bit in `AX`, or 32-bit in `DI:AX`) and a binary exponent (`DX`, second operand `BX`/`CX`).
//! These routines are reached through the main vector table slots `DS:0270`–`032b`
//! (docs/OVERLAYS.md §2) and use registers, not the stack, for arguments and results.
//!
//! Each port is a register-exact transliteration of the original instructions: it takes and
//! returns every register the routine writes, including scratch values callers may still read
//! (the code is machine-translated 68000 code) and how many instructions the original executes.
//! Flags are not modelled (docs/MATH3.md). 386 behaviour is kept,
//! notably shift counts in `CL` masked to 5 bits. Parity against the original running on the
//! reference runtime: `crates/hooks/tests/math3_parity.rs`.

/// The 16-bit registers a routine reads and writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub bp: u16,
}

/// What a routine wrote to `DS:09dc`, the divide-overflow resume offset that the main program's
/// `int 0` handler (`1000:cdeb`) jumps to. `None` if the routine does not touch it.
pub type DivideResume = Option<u16>;

/// How a call ended, beyond the registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exit {
    /// The `DS:09dc` store, if any.
    pub resume: DivideResume,
    /// Instructions the original executes, from its first through its `RETF` (including the
    /// main program's `int 0` handler after a divide overflow). A hook charges this so the
    /// runtime's clock, and with it every interrupt, stays where the original would put it.
    pub steps: u64,
}

impl Exit {
    fn steps(steps: u64) -> Self {
        Exit { resume: None, steps }
    }
}

/// Instructions in the main program's `int 0` handler (`1000:cdeb`: `STI`, `PUSH AX`,
/// `PUSH BP`, `MOV BP,SP`, `MOV AX,[09dc]`, `MOV [BP+4],AX`, `POP BP`, `POP AX`, `IRET`).
pub const DIVIDE_HANDLER_STEPS: u64 = 9;

// ---- single-instruction helpers ------------------------------------------------------------

/// `SHL r,1`: (result, CF, OF).
fn shl1(v: u16) -> (u16, bool, bool) {
    let r = v << 1;
    let cf = v & 0x8000 != 0;
    (r, cf, (r & 0x8000 != 0) != cf)
}

/// `RCL r,1`: (result, CF, OF).
fn rcl1(v: u16, cf: bool) -> (u16, bool, bool) {
    let r = (v << 1) | cf as u16;
    let ncf = v & 0x8000 != 0;
    (r, ncf, (r & 0x8000 != 0) != ncf)
}

/// `RCR r,1`: (result, CF).
fn rcr1(v: u16, cf: bool) -> (u16, bool) {
    ((v >> 1) | (cf as u16) << 15, v & 1 != 0)
}

/// `SAR r,1`: (result, CF).
fn sar1(v: u16) -> (u16, bool) {
    (((v as i16) >> 1) as u16, v & 1 != 0)
}

/// `SAR r,CL` / `SHR r,CL` with the 386's 5-bit count mask.
fn sar_cl(v: u16, cl: u8) -> u16 {
    let n = (cl & 31) as u32;
    ((v as i16 as i32) >> n.min(31)) as u16
}

fn shr_cl(v: u16, cl: u8) -> u16 {
    let n = (cl & 31) as u32;
    if n >= 16 { 0 } else { v >> n }
}

/// `IDIV BX` on `DX:AX`; `None` on divide error (zero divisor or quotient out of range), where
/// the hardware leaves `AX` and `DX` unchanged.
fn idiv16(dx: u16, ax: u16, bx: u16) -> Option<(u16, u16)> {
    let n = ((dx as u32) << 16 | ax as u32) as i32 as i64;
    let d = bx as i16 as i64;
    if d == 0 {
        return None;
    }
    let q = n / d;
    if q < i16::MIN as i64 || q > i16::MAX as i64 {
        return None;
    }
    Some((q as u16, (n % d) as u16))
}

/// Shifting more than this many times leaves a sign-filled 16/32/64-bit value unchanged, so
/// long original loops are capped without changing the result.
const SATURATE: u32 = 70;

// ---- ports --------------------------------------------------------------------------------

/// Port of `EL2MATH3:1cf5` (slot `DS:02b4`, also `EL2MATH3:1cfe` for slot `DS:02b8`):
/// multiply. `DX:AX = (AX × DX) << 1` (signed 16 × 16 → 32, the product of two Q15 mantissas),
/// `BX += CX` (the exponents).
pub fn mul(r: &mut Regs) -> Exit {
    let p = (r.ax as i16 as i32) * (r.dx as i16 as i32);
    let p = (p as u32) << 1;
    r.ax = p as u16;
    r.dx = (p >> 16) as u16;
    r.bx = r.bx.wrapping_add(r.cx);
    Exit::steps(5)
}

/// `DX:AX >>= 2` (two `SAR DX,1; RCR AX,1`) then `IDIV BX`; on a divide error the registers
/// keep the shifted values. Returns whether it overflowed.
fn shift2_idiv(r: &mut Regs, mut dx: u16, mut ax: u16) -> bool {
    for _ in 0..2 {
        let (d, c) = sar1(dx);
        dx = d;
        ax = rcr1(ax, c).0;
    }
    match idiv16(dx, ax, r.bx) {
        Some((q, rem)) => {
            r.ax = q;
            r.dx = rem;
            false
        }
        None => {
            r.ax = ax;
            r.dx = dx;
            true
        }
    }
}

/// Port of `EL2MATH3:1d07` (slot `DS:0288`): divide mantissa by `BX`, renormalised. When `BX` is
/// 0 it only increments `CX` (the jump lands on the final `INC CX`). Otherwise `CX -= AX`, then
/// `DX:AX = (DX << 16) >> 2`, `IDIV BX`. On a divide overflow the main program's `int 0` handler
/// resumes at `1d1f` with `DX:AX` unchanged. If the quotient's high byte is in −64 < AH < 64 it
/// is doubled (`CX` net unchanged), else `CX += 1`.
pub fn div_norm(r: &mut Regs) -> Exit {
    if r.bx == 0 {
        r.cx = r.cx.wrapping_add(1);
        return Exit::steps(4);
    }
    r.cx = r.cx.wrapping_sub(r.ax);
    let overflow = shift2_idiv(r, r.dx, 0);
    let mut steps = 10 + if overflow { DIVIDE_HANDLER_STEPS } else { 0 };
    let ah = (r.ax >> 8) as u8 as i8;
    if ah >= 0x40 {
        r.cx = r.cx.wrapping_add(1);
        steps += 4;
    } else if ah <= -0x40 {
        r.cx = r.cx.wrapping_add(1);
        steps += 6;
    } else {
        r.ax <<= 1;
        steps += 8;
    }
    Exit { resume: Some(0x1d1f), steps }
}

/// Port of `EL2MATH3:1d2e` (slot `DS:02b0`): `CX -= AX`, `DX:AX >>= 2` (arithmetic, 32-bit),
/// `IDIV BX`. Divide overflow (including `BX` = 0) resumes at the `retf` with `DX:AX` as shifted.
pub fn div(r: &mut Regs) -> Exit {
    r.cx = r.cx.wrapping_sub(r.ax);
    let overflow = shift2_idiv(r, r.dx, r.ax);
    Exit { resume: Some(0x1d40), steps: 8 + if overflow { DIVIDE_HANDLER_STEPS } else { 0 } }
}

/// Port of `EL2MATH3:1d41` (slot `DS:02d0`): subtract. `BX = −BX`, then [`add`].
pub fn sub(r: &mut Regs) -> Exit {
    r.bx = r.bx.wrapping_neg();
    Exit::steps(1 + add(r).steps)
}

/// Port of `EL2MATH3:1d43` (slot `DS:027c`): add two 16-bit floats, `AX`·2^`DX` + `BX`·2^`CX`.
/// The operand with the smaller exponent is shifted right (`SAR BX,CL`, count masked to 5 bits)
/// and added. Overflow shifts the carry back in and raises the exponent; otherwise a non-zero sum
/// is shifted left until it would overflow, then corrected by one.
pub fn add(r: &mut Regs) -> Exit {
    let mut steps = 7;
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
        steps += 2;
    }
    r.cx = r.dx.wrapping_sub(r.cx);
    r.bx = sar_cl(r.bx, r.cx as u8);
    let (sum, cf) = r.ax.overflowing_add(r.bx);
    let of = ((r.ax ^ sum) & (r.bx ^ sum) & 0x8000) != 0;
    r.ax = sum;
    if of {
        r.ax = rcr1(r.ax, cf).0;
        r.dx = r.dx.wrapping_add(1);
        return Exit::steps(steps + 3);
    }
    if r.ax == 0 {
        return Exit::steps(steps + 2);
    }
    Exit::steps(steps + 1 + normalise16(&mut r.ax, &mut r.dx) + 1)
}

/// The `DEC DX; SHL AX,1; JNO …; RCR AX,1; INC DX` normalisation loop shared by several routines.
/// Returns its instruction count.
fn normalise16(ax: &mut u16, dx: &mut u16) -> u64 {
    let mut steps = 0;
    loop {
        steps += 3;
        *dx = dx.wrapping_sub(1);
        let (v, cf, of) = shl1(*ax);
        *ax = v;
        if of {
            *ax = rcr1(*ax, cf).0;
            *dx = dx.wrapping_add(1);
            return steps + 2;
        }
    }
}

/// Port of `EL2MATH3:1d63` (slot `DS:02d4`): 32-bit subtract. `BP:BX = −BP:BX`, then [`add32`].
pub fn sub32(r: &mut Regs) -> Exit {
    let v = ((r.bp as u32) << 16 | r.bx as u32).wrapping_neg();
    r.bx = v as u16;
    r.bp = (v >> 16) as u16;
    Exit::steps(4 + add32(r).steps)
}

/// Port of `EL2MATH3:1d6d` (slot `DS:0280`): add two 32-bit-mantissa floats, `DI:AX`·2^`DX` +
/// `BP:BX`·2^`CX`, with the same alignment and normalisation as [`add`] on 32 bits.
pub fn add32(r: &mut Regs) -> Exit {
    let mut steps = 5;
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
        std::mem::swap(&mut r.bp, &mut r.di);
        steps += 3;
    }
    r.cx = r.dx.wrapping_sub(r.cx);
    steps += 3 * r.cx as u64;
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let (bp, c) = sar1(r.bp);
        r.bp = bp;
        r.bx = rcr1(r.bx, c).0;
    }
    if r.cx != 0 {
        r.cx = 0; // LOOP leaves CX = 0
    }
    let (lo, c1) = r.ax.overflowing_add(r.bx);
    let full = r.di as u32 + r.bp as u32 + c1 as u32;
    let hi = full as u16;
    let cf = full > 0xFFFF;
    let of = ((r.di ^ hi) & (r.bp ^ hi) & 0x8000) != 0;
    r.ax = lo;
    r.di = hi;
    steps += 3;
    if of {
        let (di, c) = rcr1(r.di, cf);
        r.di = di;
        r.ax = rcr1(r.ax, c).0;
        r.dx = r.dx.wrapping_add(1);
        return Exit::steps(steps + 4);
    }
    steps += if r.di != 0 { 1 } else { 3 };
    if r.di == 0 && r.ax == 0 {
        return Exit::steps(steps + 1);
    }
    loop {
        steps += 4;
        r.dx = r.dx.wrapping_sub(1);
        let (ax, c, _) = shl1(r.ax);
        r.ax = ax;
        let (di, c2, of) = rcl1(r.di, c);
        r.di = di;
        if of {
            let (di, c3) = rcr1(r.di, c2);
            r.di = di;
            r.ax = rcr1(r.ax, c3).0;
            r.dx = r.dx.wrapping_add(1);
            return Exit::steps(steps + 4);
        }
    }
}

/// Port of `EL2MATH3:1da1` (slot `DS:0284`): add two unsigned 16-bit mantissas (`SHR BX,CL`),
/// halving once if the sum has bit 15 set.
pub fn add_unsigned(r: &mut Regs) -> Exit {
    let mut steps = 7;
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
        steps += 2;
    }
    r.cx = r.dx.wrapping_sub(r.cx);
    r.bx = shr_cl(r.bx, r.cx as u8);
    r.ax = r.ax.wrapping_add(r.bx);
    if r.ax & 0x8000 != 0 {
        r.ax >>= 1;
        r.dx = r.dx.wrapping_add(1);
        steps += 2;
    }
    Exit::steps(steps + 1)
}

/// Port of `EL2MATH3:1db6` (slot `DS:02c4`): float from the 32-bit integer `DX:AX`, unsigned-style.
/// `BX` = exponent: left-justify until bit 31 is set (`BX` counts down from −7), then `BX += 27h`
/// and `DX >>= 1` (the result mantissa). Zero gives `BX = FFF9h`.
pub fn from_u32(r: &mut Regs) -> Exit {
    r.bx = 0xFFF9;
    let mut steps = 3;
    if r.dx & 0x8000 == 0 {
        steps += if r.dx != 0 { 1 } else { 3 };
        if r.dx == 0 && r.ax == 0 {
            return Exit::steps(steps + 1);
        }
        loop {
            steps += 5;
            r.bx = r.bx.wrapping_sub(1);
            let (ax, c, _) = shl1(r.ax);
            r.ax = ax;
            r.dx = rcl1(r.dx, c).0;
            if r.dx & 0x8000 != 0 {
                break;
            }
        }
    }
    r.bx = r.bx.wrapping_add(0x27);
    r.dx >>= 1;
    Exit::steps(steps + 3)
}

/// Port of `EL2MATH3:1dd2` (slot `DS:02bc`): float from the signed 32-bit integer `DX:AX`.
/// Shifts `DX:AX` left until the shift overflows, rotates the carry back into `DX`, `BX += 26h`.
pub fn from_i32(r: &mut Regs) -> Exit {
    r.bx = 0xFFF9;
    let mut steps = if r.dx != 0 { 3 } else { 5 };
    if r.dx == 0 && r.ax == 0 {
        return Exit::steps(steps + 1);
    }
    loop {
        steps += 3;
        let (ax, c, _) = shl1(r.ax);
        r.ax = ax;
        let (dx, c2, of) = rcl1(r.dx, c);
        r.dx = dx;
        if of {
            r.dx = rcr1(r.dx, c2).0;
            r.bx = r.bx.wrapping_add(0x26);
            return Exit::steps(steps + 3);
        }
        r.bx = r.bx.wrapping_sub(1);
        steps += 2;
    }
}

/// Port of `EL2MATH3:1dec` (slot `DS:02c0`): normalise the signed 32-bit `DX:AX`, exponent in
/// `BX` from −6 downwards (32-bit mantissa result).
pub fn norm_i32(r: &mut Regs) -> Exit {
    r.bx = 0xFFFA;
    let steps = if r.dx != 0 { 3 } else { 5 };
    if r.dx == 0 && r.ax == 0 {
        return Exit::steps(steps + 1);
    }
    Exit::steps(steps + normalise32(&mut r.ax, &mut r.dx, &mut r.bx) + 1)
}

/// `DEC n; SHL AX,1; RCL DX,1; JNO …; RCR DX,1; RCR AX,1`. Returns its instruction count.
fn normalise32(ax: &mut u16, dx: &mut u16, n: &mut u16) -> u64 {
    let mut steps = 0;
    loop {
        steps += 4;
        *n = n.wrapping_sub(1);
        let (a, c, _) = shl1(*ax);
        *ax = a;
        let (d, c2, of) = rcl1(*dx, c);
        *dx = d;
        if of {
            let (d, c3) = rcr1(*dx, c2);
            *dx = d;
            *ax = rcr1(*ax, c3).0;
            return steps + 2;
        }
    }
}

/// Port of `EL2MATH3:1e03` (slot `DS:028c`): float (`DX` mantissa, `CX` exponent) to a 32-bit
/// fixed-point `DX:AX`, shifting right by `1Fh − CX`. A negative shift saturates to `7FFF:FFFF`
/// or `8000:0000` by sign.
pub fn to_fixed32(r: &mut Regs) -> Exit {
    r.ax = 0;
    r.cx = 0x1F_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        let sign = if r.dx & 0x8000 != 0 { 0xFFFF } else { 0 };
        r.dx = sign ^ 0x7FFF;
        r.ax = !sign;
        return Exit::steps(11);
    }
    let steps = 6 + 3 * r.cx as u64;
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let (d, c) = sar1(r.dx);
        r.dx = d;
        r.ax = rcr1(r.ax, c).0;
    }
    r.cx = 0;
    Exit::steps(steps)
}

/// Port of `EL2MATH3:1e22` (slot `DS:0290`): like [`to_fixed32`] without saturation; the shift
/// count `1Fh − CX` runs as an unsigned loop count and `CX` is left as that count.
pub fn to_fixed32_raw(r: &mut Regs) -> Exit {
    r.ax = 0;
    r.cx = 0x1F_u16.wrapping_sub(r.cx);
    if r.cx == 0 {
        return Exit::steps(5);
    }
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let (d, c) = sar1(r.dx);
        r.dx = d;
        r.ax = rcr1(r.ax, c).0;
    }
    Exit::steps(7 + 3 * r.cx as u64)
}

/// Port of `EL2MATH3:1e34` (slot `DS:029c`): float (`AX` mantissa, `CX` exponent) to a 16-bit
/// integer, `AX >>= 0Fh − CX`. A negative shift saturates `AX` to `7FFFh`/`8000h` by sign, keeps
/// the old `DX` and clears `CX`.
pub fn to_int16(r: &mut Regs) -> Exit {
    let mut steps = 5;
    r.cx = 0x0F_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        let sign = if r.ax & 0x8000 != 0 { 0xFFFF } else { 0 };
        r.ax = sign ^ 0x7FFF;
        r.cx = 0;
        steps += 6;
    }
    r.ax = sar_cl(r.ax, r.cx as u8);
    Exit::steps(steps)
}

/// Port of `EL2MATH3:1e4a` (slot `DS:0294`): unsigned mantissa (`DX`, shifted left once) to a
/// 32-bit `DX:AX` by a logical right shift of `20h − CX`; a negative count saturates to all ones.
pub fn to_fixed32_unsigned(r: &mut Regs) -> Exit {
    r.dx <<= 1;
    r.ax = 0;
    r.cx = 0x20_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        r.ax = 0xFFFF;
        r.dx = 0xFFFF;
        r.cx = 0;
        return Exit::steps(10);
    }
    let steps = 7 + 3 * r.cx as u64;
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let c = r.dx & 1 != 0;
        r.dx >>= 1;
        r.ax = rcr1(r.ax, c).0;
    }
    r.cx = 0;
    Exit::steps(steps)
}

/// Port of `EL2MATH3:1e65` (slot `DS:0298`): float (`DX` mantissa, `CX` exponent) to a 64-bit
/// fixed-point value in `DX:CX:BX:AX`, shifting right by `3Eh − CX` + 1 (`DEC CX; JNS` loop).
/// `BP` is preserved.
pub fn to_fixed64(r: &mut Regs) -> Exit {
    let (mut bx, mut ax, mut bp) = (0u16, 0u16, 0u16);
    let c0 = 0x3E_u16.wrapping_sub(r.cx);
    // DEC CX; JNS runs once for a negative count, count+1 times otherwise (0x8000 wraps).
    let iters: u32 = if c0 & 0x8000 == 0 { c0 as u32 + 1 } else if c0 == 0x8000 { 0x8001 } else { 1 };
    for _ in 0..iters.min(SATURATE) {
        let (dx, c) = sar1(r.dx);
        r.dx = dx;
        let (b, c) = rcr1(bp, c);
        bp = b;
        let (b, c) = rcr1(bx, c);
        bx = b;
        ax = rcr1(ax, c).0;
    }
    r.ax = ax;
    r.bx = bx;
    r.cx = bp;
    Exit::steps(9 + 6 * iters as u64)
}

/// Port of `EL2MATH3:1e80` (slot `DS:02a8`): normalise an unsigned 16-bit mantissa `AX` so bit 14
/// is the top set bit, adjusting the exponent `DX`.
pub fn norm_unsigned(r: &mut Regs) -> Exit {
    if r.ax == 0 {
        return Exit::steps(3);
    }
    let mut steps = 2;
    loop {
        steps += 3;
        r.dx = r.dx.wrapping_sub(1);
        r.ax <<= 1;
        if r.ax & 0x8000 != 0 {
            break;
        }
    }
    r.dx = r.dx.wrapping_add(1);
    r.ax >>= 1;
    Exit::steps(steps + 3)
}

/// Port of `EL2MATH3:1e8d` (slot `DS:02a0`): normalise the signed 16-bit mantissa `AX`, exponent
/// `DX`.
pub fn norm16(r: &mut Regs) -> Exit {
    if r.ax == 0 {
        return Exit::steps(3);
    }
    Exit::steps(2 + normalise16(&mut r.ax, &mut r.dx) + 1)
}

/// Port of `EL2MATH3:1e9a` (slot `DS:02a4`): normalise the signed 32-bit mantissa `DX:AX`,
/// exponent `CX`.
pub fn norm32(r: &mut Regs) -> Exit {
    let steps = if r.dx != 0 { 2 } else { 4 };
    if r.dx == 0 && r.ax == 0 {
        return Exit::steps(steps + 1);
    }
    let steps = steps + normalise32(&mut r.ax, &mut r.dx, &mut r.cx);
    r.cx = r.cx.wrapping_add(1);
    Exit::steps(steps + 2)
}

/// Port of `EL2MATH3:1eaf` (slot `DS:02ac`): shift a positive `AX` left until bit 15 is set,
/// decrementing `DX` each time. Zero and negative `AX` are returned unchanged.
pub fn norm_positive(r: &mut Regs) -> Exit {
    if r.ax == 0 {
        return Exit::steps(3);
    }
    if r.ax & 0x8000 != 0 {
        return Exit::steps(4);
    }
    let mut steps = 3;
    loop {
        steps += 3;
        r.dx = r.dx.wrapping_sub(1);
        r.ax <<= 1;
        if r.ax & 0x8000 != 0 {
            break;
        }
    }
    Exit::steps(steps + 1)
}

/// A port: updates the registers and reports how the original's call ended.
pub type Port = fn(&mut Regs) -> Exit;

/// Every routine in this module: (overlay offset, vector slot, name, port).
pub const PORTS: &[(u16, u16, &str, Port)] = &[
    (0x1cf5, 0x02b4, "mul", mul),
    (0x1cfe, 0x02b8, "mul", mul),
    (0x1d07, 0x0288, "div_norm", div_norm),
    (0x1d2e, 0x02b0, "div", div),
    (0x1d41, 0x02d0, "sub", sub),
    (0x1d43, 0x027c, "add", add),
    (0x1d63, 0x02d4, "sub32", sub32),
    (0x1d6d, 0x0280, "add32", add32),
    (0x1da1, 0x0284, "add_unsigned", add_unsigned),
    (0x1db6, 0x02c4, "from_u32", from_u32),
    (0x1dd2, 0x02bc, "from_i32", from_i32),
    (0x1dec, 0x02c0, "norm_i32", norm_i32),
    (0x1e03, 0x028c, "to_fixed32", to_fixed32),
    (0x1e22, 0x0290, "to_fixed32_raw", to_fixed32_raw),
    (0x1e34, 0x029c, "to_int16", to_int16),
    (0x1e4a, 0x0294, "to_fixed32_unsigned", to_fixed32_unsigned),
    (0x1e65, 0x0298, "to_fixed64", to_fixed64),
    (0x1e80, 0x02a8, "norm_unsigned", norm_unsigned),
    (0x1e8d, 0x02a0, "norm16", norm16),
    (0x1e9a, 0x02a4, "norm32", norm32),
    (0x1eaf, 0x02ac, "norm_positive", norm_positive),
];
