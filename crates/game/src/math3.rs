//! `EL2MATH3`: the software floating-point library (docs/MATH3.md).
//!
//! The DOS build does its 3D and orbital arithmetic on pairs of a normalised signed mantissa
//! (16-bit in `AX`, or 32-bit in `DI:AX`) and a binary exponent (`DX`, second operand `BX`/`CX`).
//! These routines are reached through the main vector table slots `DS:0270`–`032b`
//! (docs/OVERLAYS.md §2) and use registers, not the stack, for arguments and results.
//!
//! Each port is a register-exact transliteration of the original instructions: it takes and
//! returns every register the routine writes, including scratch values callers may still read
//! (the code is machine-translated 68000 code). Flags are not modelled. 386 behaviour is kept,
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
pub fn mul(r: &mut Regs) {
    let p = (r.ax as i16 as i32) * (r.dx as i16 as i32);
    let p = (p as u32) << 1;
    r.ax = p as u16;
    r.dx = (p >> 16) as u16;
    r.bx = r.bx.wrapping_add(r.cx);
}

/// Port of `EL2MATH3:1d07` (slot `DS:0288`): divide mantissa by `BX`, renormalised. When `BX` is
/// 0 it only increments `CX` (the jump lands on the final `INC CX`). Otherwise `CX -= AX`, then `DX:AX = (DX << 16) >> 2`, `IDIV BX`. On a divide
/// overflow the main program's `int 0` handler resumes at `1d1f` with `DX:AX` unchanged. If the
/// quotient's high byte is in −64 < AH < 64 it is doubled (`CX` net unchanged), else `CX += 1`.
pub fn div_norm(r: &mut Regs) -> DivideResume {
    if r.bx == 0 {
        r.cx = r.cx.wrapping_add(1);
        return None;
    }
    r.cx = r.cx.wrapping_sub(r.ax);
    let (mut dx, mut ax) = (r.dx, 0u16);
    for _ in 0..2 {
        let (d, c) = sar1(dx);
        dx = d;
        ax = rcr1(ax, c).0;
    }
    match idiv16(dx, ax, r.bx) {
        Some((q, rem)) => {
            r.ax = q;
            r.dx = rem;
        }
        None => {
            r.ax = ax;
            r.dx = dx;
        }
    }
    let ah = (r.ax >> 8) as u8 as i8;
    if ah < 0x40 && ah > -0x40 {
        r.ax <<= 1;
    } else {
        r.cx = r.cx.wrapping_add(1);
    }
    Some(0x1d1f)
}

/// Port of `EL2MATH3:1d2e` (slot `DS:02b0`): `CX -= AX`, `DX:AX >>= 2` (arithmetic, 32-bit),
/// `IDIV BX`. Divide overflow (including `BX` = 0) resumes at the `retf` with `DX:AX` as shifted.
pub fn div(r: &mut Regs) -> DivideResume {
    r.cx = r.cx.wrapping_sub(r.ax);
    let (mut dx, mut ax) = (r.dx, r.ax);
    for _ in 0..2 {
        let (d, c) = sar1(dx);
        dx = d;
        ax = rcr1(ax, c).0;
    }
    match idiv16(dx, ax, r.bx) {
        Some((q, rem)) => {
            r.ax = q;
            r.dx = rem;
        }
        None => {
            r.ax = ax;
            r.dx = dx;
        }
    }
    Some(0x1d40)
}

/// Port of `EL2MATH3:1d41` (slot `DS:02d0`): subtract. `BX = −BX`, then [`add`].
pub fn sub(r: &mut Regs) {
    r.bx = r.bx.wrapping_neg();
    add(r);
}

/// Port of `EL2MATH3:1d43` (slot `DS:027c`): add two 16-bit floats, `AX`·2^`DX` + `BX`·2^`CX`.
/// The operand with the smaller exponent is shifted right (`SAR BX,CL`, count masked to 5 bits)
/// and added. Overflow shifts the carry back in and raises the exponent; otherwise a non-zero sum
/// is shifted left until it would overflow, then corrected by one.
pub fn add(r: &mut Regs) {
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
    }
    r.cx = r.dx.wrapping_sub(r.cx);
    r.bx = sar_cl(r.bx, r.cx as u8);
    let (sum, cf) = r.ax.overflowing_add(r.bx);
    let of = ((r.ax ^ sum) & (r.bx ^ sum) & 0x8000) != 0;
    r.ax = sum;
    if of {
        r.ax = rcr1(r.ax, cf).0;
        r.dx = r.dx.wrapping_add(1);
        return;
    }
    if r.ax == 0 {
        return;
    }
    normalise16(&mut r.ax, &mut r.dx);
}

/// The `DEC DX; SHL AX,1; JNO …; RCR AX,1; INC DX` normalisation loop shared by several routines.
fn normalise16(ax: &mut u16, dx: &mut u16) {
    loop {
        *dx = dx.wrapping_sub(1);
        let (v, cf, of) = shl1(*ax);
        *ax = v;
        if of {
            *ax = rcr1(*ax, cf).0;
            *dx = dx.wrapping_add(1);
            return;
        }
    }
}

/// Port of `EL2MATH3:1d63` (slot `DS:02d4`): 32-bit subtract. `BP:BX = −BP:BX`, then [`add32`].
pub fn sub32(r: &mut Regs) {
    let v = ((r.bp as u32) << 16 | r.bx as u32).wrapping_neg();
    r.bx = v as u16;
    r.bp = (v >> 16) as u16;
    add32(r);
}

/// Port of `EL2MATH3:1d6d` (slot `DS:0280`): add two 32-bit-mantissa floats, `DI:AX`·2^`DX` +
/// `BP:BX`·2^`CX`, with the same alignment and normalisation as [`add`] on 32 bits.
pub fn add32(r: &mut Regs) {
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
        std::mem::swap(&mut r.bp, &mut r.di);
    }
    r.cx = r.dx.wrapping_sub(r.cx);
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
    if of {
        let (di, c) = rcr1(r.di, cf);
        r.di = di;
        r.ax = rcr1(r.ax, c).0;
        r.dx = r.dx.wrapping_add(1);
        return;
    }
    if r.di == 0 && r.ax == 0 {
        return;
    }
    loop {
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
            return;
        }
    }
}

/// Port of `EL2MATH3:1da1` (slot `DS:0284`): add two unsigned 16-bit mantissas (`SHR BX,CL`),
/// halving once if the sum has bit 15 set.
pub fn add_unsigned(r: &mut Regs) {
    if (r.cx as i16) > (r.dx as i16) {
        std::mem::swap(&mut r.ax, &mut r.bx);
        std::mem::swap(&mut r.cx, &mut r.dx);
    }
    r.cx = r.dx.wrapping_sub(r.cx);
    r.bx = shr_cl(r.bx, r.cx as u8);
    r.ax = r.ax.wrapping_add(r.bx);
    if r.ax & 0x8000 != 0 {
        r.ax >>= 1;
        r.dx = r.dx.wrapping_add(1);
    }
}

/// Port of `EL2MATH3:1db6` (slot `DS:02c4`): float from the 32-bit integer `DX:AX`, unsigned-style.
/// `BX` = exponent: left-justify until bit 31 is set (`BX` counts down from −7), then `BX += 27h`
/// and `DX >>= 1` (the result mantissa). Zero gives `BX = FFF9h`.
pub fn from_u32(r: &mut Regs) {
    r.bx = 0xFFF9;
    if r.dx & 0x8000 == 0 {
        if r.dx == 0 && r.ax == 0 {
            return;
        }
        loop {
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
}

/// Port of `EL2MATH3:1dd2` (slot `DS:02bc`): float from the signed 32-bit integer `DX:AX`.
/// Shifts `DX:AX` left until the shift overflows, rotates the carry back into `DX`, `BX += 26h`.
pub fn from_i32(r: &mut Regs) {
    r.bx = 0xFFF9;
    if r.dx == 0 && r.ax == 0 {
        return;
    }
    loop {
        let (ax, c, _) = shl1(r.ax);
        r.ax = ax;
        let (dx, c2, of) = rcl1(r.dx, c);
        r.dx = dx;
        if of {
            r.dx = rcr1(r.dx, c2).0;
            r.bx = r.bx.wrapping_add(0x26);
            return;
        }
        r.bx = r.bx.wrapping_sub(1);
    }
}

/// Port of `EL2MATH3:1dec` (slot `DS:02c0`): normalise the signed 32-bit `DX:AX`, exponent in
/// `BX` from −6 downwards (32-bit mantissa result).
pub fn norm_i32(r: &mut Regs) {
    r.bx = 0xFFFA;
    if r.dx == 0 && r.ax == 0 {
        return;
    }
    normalise32(&mut r.ax, &mut r.dx, &mut r.bx);
}

/// `DEC n; SHL AX,1; RCL DX,1; JNO …; RCR DX,1; RCR AX,1` (and `INC n` when `inc` is set).
fn normalise32(ax: &mut u16, dx: &mut u16, n: &mut u16) {
    loop {
        *n = n.wrapping_sub(1);
        let (a, c, _) = shl1(*ax);
        *ax = a;
        let (d, c2, of) = rcl1(*dx, c);
        *dx = d;
        if of {
            let (d, c3) = rcr1(*dx, c2);
            *dx = d;
            *ax = rcr1(*ax, c3).0;
            return;
        }
    }
}

/// Port of `EL2MATH3:1e03` (slot `DS:028c`): float (`DX` mantissa, `CX` exponent) to a 32-bit
/// fixed-point `DX:AX`, shifting right by `1Fh − CX`. A negative shift saturates to `7FFF:FFFF`
/// or `8000:0000` by sign.
pub fn to_fixed32(r: &mut Regs) {
    r.ax = 0;
    r.cx = 0x1F_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        let sign = if r.dx & 0x8000 != 0 { 0xFFFF } else { 0 };
        r.dx = sign ^ 0x7FFF;
        r.ax = !sign;
        return;
    }
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let (d, c) = sar1(r.dx);
        r.dx = d;
        r.ax = rcr1(r.ax, c).0;
    }
    r.cx = 0;
}

/// Port of `EL2MATH3:1e22` (slot `DS:0290`): like [`to_fixed32`] without saturation; the shift
/// count `1Fh − CX` runs as an unsigned loop count and `CX` is left as that count.
pub fn to_fixed32_raw(r: &mut Regs) {
    r.ax = 0;
    r.cx = 0x1F_u16.wrapping_sub(r.cx);
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let (d, c) = sar1(r.dx);
        r.dx = d;
        r.ax = rcr1(r.ax, c).0;
    }
}

/// Port of `EL2MATH3:1e34` (slot `DS:029c`): float (`AX` mantissa, `CX` exponent) to a 16-bit
/// integer, `AX >>= 0Fh − CX`. A negative shift saturates `AX` to `7FFFh`/`8000h` by sign, keeps
/// the old `DX` and clears `CX`.
pub fn to_int16(r: &mut Regs) {
    r.cx = 0x0F_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        let sign = if r.ax & 0x8000 != 0 { 0xFFFF } else { 0 };
        r.ax = sign ^ 0x7FFF;
        r.cx = 0;
    }
    r.ax = sar_cl(r.ax, r.cx as u8);
}

/// Port of `EL2MATH3:1e4a` (slot `DS:0294`): unsigned mantissa (`DX`, shifted left once) to a
/// 32-bit `DX:AX` by a logical right shift of `20h − CX`; a negative count saturates to all ones.
pub fn to_fixed32_unsigned(r: &mut Regs) {
    r.dx <<= 1;
    r.ax = 0;
    r.cx = 0x20_u16.wrapping_sub(r.cx);
    if r.cx & 0x8000 != 0 {
        r.ax = 0xFFFF;
        r.dx = 0xFFFF;
        r.cx = 0;
        return;
    }
    let n = (r.cx as u32).min(SATURATE);
    for _ in 0..n {
        let c = r.dx & 1 != 0;
        r.dx >>= 1;
        r.ax = rcr1(r.ax, c).0;
    }
    r.cx = 0;
}

/// Port of `EL2MATH3:1e65` (slot `DS:0298`): float (`DX` mantissa, `CX` exponent) to a 64-bit
/// fixed-point value in `DX:CX:BX:AX`, shifting right by `3Eh − CX` + 1 (`DEC CX; JNS` loop).
/// `BP` is preserved.
pub fn to_fixed64(r: &mut Regs) {
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
}

/// Port of `EL2MATH3:1e80` (slot `DS:02a8`): normalise an unsigned 16-bit mantissa `AX` so bit 14
/// is the top set bit, adjusting the exponent `DX`.
pub fn norm_unsigned(r: &mut Regs) {
    if r.ax == 0 {
        return;
    }
    loop {
        r.dx = r.dx.wrapping_sub(1);
        r.ax <<= 1;
        if r.ax & 0x8000 != 0 {
            break;
        }
    }
    r.dx = r.dx.wrapping_add(1);
    r.ax >>= 1;
}

/// Port of `EL2MATH3:1e8d` (slot `DS:02a0`): normalise the signed 16-bit mantissa `AX`, exponent
/// `DX`.
pub fn norm16(r: &mut Regs) {
    if r.ax == 0 {
        return;
    }
    normalise16(&mut r.ax, &mut r.dx);
}

/// Port of `EL2MATH3:1e9a` (slot `DS:02a4`): normalise the signed 32-bit mantissa `DX:AX`,
/// exponent `CX`.
pub fn norm32(r: &mut Regs) {
    if r.dx == 0 && r.ax == 0 {
        return;
    }
    normalise32(&mut r.ax, &mut r.dx, &mut r.cx);
    r.cx = r.cx.wrapping_add(1);
}

/// Port of `EL2MATH3:1eaf` (slot `DS:02ac`): shift a positive `AX` left until bit 15 is set,
/// decrementing `DX` each time. Zero and negative `AX` are returned unchanged.
pub fn norm_positive(r: &mut Regs) {
    if r.ax == 0 || r.ax & 0x8000 != 0 {
        return;
    }
    loop {
        r.dx = r.dx.wrapping_sub(1);
        r.ax <<= 1;
        if r.ax & 0x8000 != 0 {
            break;
        }
    }
}

/// Every routine in this module: (overlay offset, vector slot, port).
pub type Port = fn(&mut Regs) -> DivideResume;

pub const PORTS: &[(u16, u16, &str, Port)] = &[
    (0x1cf5, 0x02b4, "mul", |r| { mul(r); None }),
    (0x1cfe, 0x02b8, "mul", |r| { mul(r); None }),
    (0x1d07, 0x0288, "div_norm", div_norm),
    (0x1d2e, 0x02b0, "div", div),
    (0x1d41, 0x02d0, "sub", |r| { sub(r); None }),
    (0x1d43, 0x027c, "add", |r| { add(r); None }),
    (0x1d63, 0x02d4, "sub32", |r| { sub32(r); None }),
    (0x1d6d, 0x0280, "add32", |r| { add32(r); None }),
    (0x1da1, 0x0284, "add_unsigned", |r| { add_unsigned(r); None }),
    (0x1db6, 0x02c4, "from_u32", |r| { from_u32(r); None }),
    (0x1dd2, 0x02bc, "from_i32", |r| { from_i32(r); None }),
    (0x1dec, 0x02c0, "norm_i32", |r| { norm_i32(r); None }),
    (0x1e03, 0x028c, "to_fixed32", |r| { to_fixed32(r); None }),
    (0x1e22, 0x0290, "to_fixed32_raw", |r| { to_fixed32_raw(r); None }),
    (0x1e34, 0x029c, "to_int16", |r| { to_int16(r); None }),
    (0x1e4a, 0x0294, "to_fixed32_unsigned", |r| { to_fixed32_unsigned(r); None }),
    (0x1e65, 0x0298, "to_fixed64", |r| { to_fixed64(r); None }),
    (0x1e80, 0x02a8, "norm_unsigned", |r| { norm_unsigned(r); None }),
    (0x1e8d, 0x02a0, "norm16", |r| { norm16(r); None }),
    (0x1e9a, 0x02a4, "norm32", |r| { norm32(r); None }),
    (0x1eaf, 0x02ac, "norm_positive", |r| { norm_positive(r); None }),
];
