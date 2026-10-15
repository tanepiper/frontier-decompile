//! `EL2MATH3`: the software floating-point library (docs/MATH3.md).
//!
//! The DOS build does its 3D and orbital arithmetic on pairs of a normalised signed mantissa
//! (16-bit in `AX`, or 32-bit in `DI:AX`) and a binary exponent (`DX`, second operand `BX`/`CX`).
//! These routines are reached through the main vector table slots `DS:0270`–`032b`
//! (docs/OVERLAYS.md §2) and use registers, not the stack, for arguments and results.
//!
//! Each port is a register-exact transliteration of the original instructions: it takes and
//! returns every register and memory byte the routine writes, including scratch values callers may still read
//! (the code is machine-translated 68000 code) and how many instructions the original executes.
//! Flags are not modelled (docs/MATH3.md). 386 behaviour is kept,
//! notably shift counts in `CL` masked to 5 bits. Parity against the original running on the
//! reference runtime: `crates/hooks/tests/math3_parity.rs`.

mod geometry;
pub use geometry::*;

/// Main data segment offset of the divide-overflow resume address (docs/MATH3.md).
pub const DIVIDE_RESUME: u16 = 0x09dc;

/// The 386 state a port sees and leaves: general registers in encoding order (`EAX`, `ECX`,
/// `EDX`, `EBX`, `ESP`, `EBP`, `ESI`, `EDI`), the data segments, `CS:IP` (the routine's entry on the
/// way in, the caller's return address on the way out) and flags (read only: `DF`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub r: [u32; 8],
    pub cs: u16,
    pub ip: u16,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    pub fs: u16,
    pub gs: u16,
    pub flags: u32,
}

pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

/// Direction flag.
const DF: u32 = 0x0400;

impl Cpu {
    /// 16-bit register `i`.
    pub fn x(&self, i: usize) -> u16 {
        self.r[i] as u16
    }

    pub fn set_x(&mut self, i: usize, v: u16) {
        self.r[i] = (self.r[i] & 0xFFFF_0000) | v as u32;
    }

    /// Low byte of register `i` (`AL`, `CL`, `DL`, `BL`).
    pub fn l(&self, i: usize) -> u8 {
        self.r[i] as u8
    }

    pub fn set_l(&mut self, i: usize, v: u8) {
        self.r[i] = (self.r[i] & !0xFF) | v as u32;
    }

    /// High byte of register `i` (`AH`, `CH`, `DH`, `BH`).
    pub fn h(&self, i: usize) -> u8 {
        (self.r[i] >> 8) as u8
    }

    pub fn set_h(&mut self, i: usize, v: u8) {
        self.r[i] = (self.r[i] & !0xFF00) | (v as u32) << 8;
    }

    fn sp(&self) -> u16 {
        self.x(ESP)
    }

    fn pop16(&mut self, bus: &mut dyn Bus) -> u16 {
        let sp = self.sp();
        self.set_x(ESP, sp.wrapping_add(2));
        bus.read16(lin(self.ss, sp))
    }

    /// `RETF`.
    pub fn retf(&mut self, bus: &mut dyn Bus) {
        self.ip = self.pop16(bus);
        self.cs = self.pop16(bus);
    }
}

/// Real-mode linear address of `seg:off`, wrapped at 1 MB as on the reference runtime.
pub fn lin(seg: u16, off: u16) -> u32 {
    (((seg as u32) << 4) + off as u32) & 0xF_FFFF
}

/// Linear address of a 32-bit effective address (`67h` prefix): not wrapped at 64 KB.
pub fn lin32(seg: u16, off: u32) -> u32 {
    ((seg as u32) << 4).wrapping_add(off) & 0xF_FFFF
}

/// Memory as the routines see it, by linear address. Multi-byte accesses are little-endian over
/// consecutive linear addresses, as on the runtime.
pub trait Bus {
    fn read8(&mut self, a: u32) -> u8;
    fn write8(&mut self, a: u32, v: u8);

    fn read16(&mut self, a: u32) -> u16 {
        self.read8(a) as u16 | (self.read8(a + 1) as u16) << 8
    }

    fn write16(&mut self, a: u32, v: u16) {
        self.write8(a, v as u8);
        self.write8(a + 1, (v >> 8) as u8);
    }

    fn read32(&mut self, a: u32) -> u32 {
        self.read16(a) as u32 | (self.read16(a + 2) as u32) << 16
    }

    fn write32(&mut self, a: u32, v: u32) {
        self.write16(a, v as u16);
        self.write16(a + 2, (v >> 16) as u16);
    }
}

/// Instruction count a port returns when it does not handle this input (a path not ported yet,
/// or one where the original never returns). Nothing it did counts: the hook discards the staged
/// state and runs the original instead.
pub const DECLINE: u64 = u64::MAX;

/// A port of a whole routine, entry to `RETF`: updates the CPU and memory as the original
/// would and returns the original's instruction count for these inputs (see [`Exit::steps`]).
///
/// Memory below the final stack pointer (pushes and call frames the routine has popped again)
/// is not reproduced: it is dead, as any interrupt may overwrite it.
pub type Port = fn(&mut Cpu, &mut dyn Bus) -> u64;

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

impl Regs {
    pub fn load(c: &Cpu) -> Self {
        Regs { ax: c.x(EAX), bx: c.x(EBX), cx: c.x(ECX), dx: c.x(EDX), si: c.x(ESI), di: c.x(EDI), bp: c.x(EBP) }
    }

    pub fn store(&self, c: &mut Cpu) {
        c.set_x(EAX, self.ax);
        c.set_x(EBX, self.bx);
        c.set_x(ECX, self.cx);
        c.set_x(EDX, self.dx);
        c.set_x(ESI, self.si);
        c.set_x(EDI, self.di);
        c.set_x(EBP, self.bp);
    }
}

/// Run a register-only port as a whole routine: registers, the `DS:09dc` store and `RETF`.
fn with_regs(c: &mut Cpu, bus: &mut dyn Bus, f: fn(&mut Regs) -> Exit) -> u64 {
    let mut r = Regs::load(c);
    let e = f(&mut r);
    if e.steps == DECLINE {
        return DECLINE;
    }
    r.store(c);
    if let Some(v) = e.resume {
        bus.write16(lin(c.ds, DIVIDE_RESUME), v);
    }
    c.retf(bus);
    e.steps
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
/// is the top set bit, adjusting the exponent `DX`. The original never returns for `AX` = 8000h.
pub fn norm_unsigned(r: &mut Regs) -> Exit {
    if r.ax == 0 {
        return Exit::steps(3);
    }
    if r.ax == 0x8000 {
        // The first shift clears AX, and the loop never ends.
        return Exit::steps(DECLINE);
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

// ---- routines that use memory -------------------------------------------------------------

/// Main data segment offset of the square-root table: 2048 words, entry `k` = 2048·√(k/2)
/// (observed values: 0, 1448, 2048, 2508, 2896, …).
pub const SQRT_TABLE: u16 = 0x6dff;
/// Main data segment offset of the sine table: 2048 words for a half turn, entry `k` =
/// 32768·sin(k·π/2048) (observed values: 0, 50, 100, 150, 201, …; 16384 at `k` = 170 is
/// *inferred* from the shape).
pub const SINE_TABLE: u16 = 0x5bff;

/// Body of `EL2MATH3:0967` up to and including its `RETF` (which the callers' own `CALLF`
/// balances): `AX` = √(`DX:AX`) by table lookup, `DX` = the table index used. Returns the
/// instruction count.
fn sqrt32_body(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (ax, dx) = (c.x(EAX), c.x(EDX));
    let dh = (dx >> 8) as u8;
    // (index, right shift of the table value, instructions)
    let (idx, shift, steps) = if dh < 0x04 {
        if dx >= 0x40 {
            ((dx << 2 | ax >> 14) & 0xFFFE, 3, 11)
        } else if dx as u8 >= 0x04 {
            ((dx << 6 | ax >> 10) & 0xFFFE, 5, 13)
        } else {
            ((ax >> 6 | dx << 10) & 0xFFFE, 7, 14)
        }
    } else if dh >= 0x40 {
        ((dx >> 4) & 0xFFFE, 0, 10)
    } else if dh >= 0x10 {
        ((dx >> 2) & 0xFFFE, 1, 13)
    } else {
        (dx & 0xFFFE, 2, 12)
    };
    c.set_x(EDX, idx);
    let v = bus.read16(lin(c.ds, SQRT_TABLE.wrapping_add(idx)));
    c.set_x(EAX, v >> shift);
    steps
}

/// Port of `EL2MATH3:0967` (slot `DS:031c`): square root of the unsigned 32-bit `DX:AX`. The
/// top non-zero bits pick a 2048-entry table index ([`SQRT_TABLE`]) by one of six shifts and the
/// entry is shifted back by half the scale: `AX` ≈ √(`DX:AX`) · 2^(-?) per range (the result is
/// what the table gives, not a computed root). `DX` is left as the index, `BX` unchanged.
pub fn sqrt32(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let steps = sqrt32_body(c, bus);
    c.retf(bus);
    steps
}

/// Port of `EL2MATH3:0952` (slot `DS:0320`): vector length. `DX:AX` = `AX`² + `BX`² + `CX`²
/// (signed 16-bit squares, 32-bit sum), then falls into [`sqrt32`]. `BX` is left as the low word
/// of `AX`² + `BX`², `BP` is preserved.
pub fn length3(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let sq = |v: u16| ((v as i16 as i32) * (v as i16 as i32)) as u32;
    let ab = sq(c.x(EAX)).wrapping_add(sq(c.x(EBX)));
    let sum = ab.wrapping_add(sq(c.x(ECX)));
    c.set_x(EBX, ab as u16);
    c.set_x(EAX, sum as u16);
    c.set_x(EDX, (sum >> 16) as u16);
    let steps = 12 + sqrt32_body(c, bus);
    c.retf(bus);
    steps
}

/// Port of `EL2MATH3:0922` (slot `DS:02c8`): square root of a float (`AX` mantissa, taken as
/// positive, `DX` exponent). The mantissa is doubled, the exponent halved (`SAR`); an odd
/// exponent halves the mantissa back and rounds the exponent up. Then `DX:AX` = mantissa:0 goes
/// through `CALLF [031c]` ([`sqrt32`]) and the root is halved. `DX` = the new exponent.
pub fn sqrt_float(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut ax = c.x(EAX) << 1;
    let dx0 = c.x(EDX);
    let mut dx = ((dx0 as i16) >> 1) as u16;
    let mut steps = 3;
    if dx0 & 1 != 0 {
        ax >>= 1;
        dx = dx.wrapping_add(1);
        steps += 2;
    }
    c.set_x(EDX, ax);
    c.set_x(EAX, 0);
    steps += 4 + sqrt32_body(c, bus);
    c.set_x(EAX, c.x(EAX) >> 1);
    c.set_x(EDX, dx);
    c.retf(bus);
    steps + 3
}

/// Port of `EL2MATH3:0938` (slot `DS:02cc`): square root of a 32-bit-mantissa float (`DX:AX`,
/// taken as positive, exponent `CX`). As [`sqrt_float`] on 32 bits: `DX:AX <<= 1`, `CX` halved,
/// an odd exponent shifts back and rounds up. Then `CALLF [031c]` ([`sqrt32`]), the root is
/// halved and normalised with `CALLF [02a8]` ([`norm_unsigned`]), exponent in `DX`. `CX` keeps
/// the halved exponent.
pub fn sqrt_float32(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut m = ((c.x(EDX) as u32) << 16 | c.x(EAX) as u32) << 1;
    let cx0 = c.x(ECX);
    let mut cx = ((cx0 as i16) >> 1) as u16;
    let mut steps = 4;
    if cx0 & 1 != 0 {
        m >>= 1;
        cx = cx.wrapping_add(1);
        steps += 3;
    }
    c.set_x(EAX, m as u16);
    c.set_x(EDX, (m >> 16) as u16);
    c.set_x(ECX, cx);
    steps += 1 + sqrt32_body(c, bus);
    c.set_x(EAX, c.x(EAX) >> 1);
    c.set_x(EDX, cx);
    let mut r = Regs::load(c);
    steps += 3 + norm_unsigned(&mut r).steps;
    r.store(c);
    c.retf(bus);
    steps + 1
}

/// `POP EAX; SUB SP,size; MOV BP,SP; PUSH EAX; PUSH saved`: a stack frame for the caller, with
/// the far return address re-pushed below it. Returns `BP`.
fn take_frame(c: &mut Cpu, bus: &mut dyn Bus, size: u16, saved: u16) -> u16 {
    let sp = c.sp();
    let ret = bus.read32(lin(c.ss, sp));
    let bp = sp.wrapping_add(4).wrapping_sub(size);
    c.set_x(EBP, bp);
    bus.write32(lin(c.ss, bp.wrapping_sub(4)), ret);
    c.set_x(ESP, bp.wrapping_sub(6));
    bus.write16(lin(c.ss, bp.wrapping_sub(6)), saved);
    bp
}

/// Port of `EL2MATH3:0ba1` (slot `DS:0278`): give the caller a 0E76h-byte stack frame. `BP` =
/// `SP` = the frame, its first 150h bytes zeroed (`REP STOSD` through `ES` = `SS`, so `DF`
/// applies), then `[BP+E72h]` = `00A8h`, `[BP+E74h]` = `[BP+E70h]` = `FFFFh`. Leaves `EAX` = 0,
/// `CX` = 0 and `DI` past the cleared block. (With `DF` set the clear runs downwards over the
/// saved `ES` and return address, as in the original.)
pub fn frame_e76(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let bp = take_frame(c, bus, 0x0E76, c.es);
    let step: u16 = if c.flags & DF != 0 { 4u16.wrapping_neg() } else { 4 };
    let mut di = bp;
    for _ in 0..0x54 {
        bus.write32(lin(c.ss, di), 0);
        di = di.wrapping_add(step);
    }
    c.es = c.pop16(bus);
    bus.write16(lin(c.ss, bp.wrapping_add(0x0E72)), 0x00A8);
    bus.write16(lin(c.ss, bp.wrapping_add(0x0E74)), 0xFFFF);
    bus.write16(lin(c.ss, bp.wrapping_add(0x0E70)), 0xFFFF);
    c.r[EAX] = 0;
    c.set_x(ECX, 0);
    c.set_x(EDI, di);
    c.retf(bus);
    16
}

/// Port of `EL2MATH3:0bcf` (slot `DS:0274`): give the caller a 5598h-byte stack frame. `BP` =
/// `SP` = the frame. 21 passes clear, at `BP+46h` + 400h·k, eight words 80h apart, and the 16
/// bytes at `BP+5596h` − 10h·(k+1). Then `[BP]` = `00A8h`, `[BP+2]` = `FFFFh`. Leaves `EAX` =
/// 0, `CX` = 0, `DI` = `BP+5446h`; `SI` is preserved.
pub fn frame_5598(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let bp = take_frame(c, bus, 0x5598, c.x(ESI));
    let mut di = bp.wrapping_add(0x5596);
    let mut si = bp.wrapping_add(0x46);
    for _ in 0..0x15 {
        for j in 0..8u16 {
            bus.write16(lin(c.ss, si.wrapping_add(j * 0x80)), 0);
        }
        si = si.wrapping_add(0x400);
        di = di.wrapping_sub(0x10);
        for j in 0..4u16 {
            bus.write32(lin(c.ss, di.wrapping_add(j * 4)), 0);
        }
    }
    bus.write16(lin(c.ss, bp), 0x00A8);
    bus.write16(lin(c.ss, bp.wrapping_add(2)), 0xFFFF);
    c.r[EAX] = 0;
    c.set_x(ECX, 0);
    c.set_x(EDI, di);
    let si = c.pop16(bus);
    c.set_x(ESI, si);
    c.retf(bus);
    9 + 0x15 * 15 + 4
}

/// `MOV DI,[angle]; (ADD DI,4000h;) BTR DI,15; SHR DI,3; AND DI,FFFEh; MOV r,[DI+5BFFh]` and
/// `NEG r` for the second half turn: the sine (or, with the quarter-turn offset, cosine) of a
/// 16-bit angle from [`SINE_TABLE`]. Returns (value, `DI`, instructions).
fn sine(c: &Cpu, bus: &mut dyn Bus, angle: u16, cosine: bool) -> (u16, u16, u64) {
    let mut di = bus.read16(lin(c.ds, angle));
    let mut steps = 6;
    if cosine {
        di = di.wrapping_add(0x4000);
        steps += 1;
    }
    let negative = di & 0x8000 != 0;
    di = ((di & 0x7FFF) >> 3) & 0xFFFE;
    let v = bus.read16(lin(c.ds, di.wrapping_add(SINE_TABLE)));
    if negative {
        (v.wrapping_neg(), di, steps + 2)
    } else {
        (v, di, steps)
    }
}

/// Main data segment offsets of the two angles the rotation routines read.
pub const ANGLE_A: u16 = 0x1f35;
pub const ANGLE_B: u16 = 0x1f37;

/// The four lookups shared by `EL2MATH3:09e9` and `0ac9`: `AX` = sin a, `BX` = sin b, `CX` =
/// cos a, `BP` = cos b for the angles at `DS:1f35` (a) and `DS:1f37` (b); `DI` from the last.
fn sines(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (sa, _, n1) = sine(c, bus, ANGLE_A, false);
    let (sb, _, n2) = sine(c, bus, ANGLE_B, false);
    let (ca, _, n3) = sine(c, bus, ANGLE_A, true);
    let (cb, di, n4) = sine(c, bus, ANGLE_B, true);
    c.set_x(EAX, sa);
    c.set_x(EBX, sb);
    c.set_x(ECX, ca);
    c.set_x(EBP, cb);
    c.set_x(EDI, di);
    n1 + n2 + n3 + n4
}

/// `IMUL` of two Q15 values then `SHLD DX,AX,1`: (`DX`, `AX`) = (high word of the product
/// doubled, low word of the product).
fn qmul(a: u16, b: u16) -> (u16, u16) {
    let p = (a as i16 as i32 * b as i16 as i32) as u32;
    (((p >> 16) as u16) << 1 | (p as u16) >> 15, p as u16)
}

/// The products both rotation routines form, in their order: (sa·sb, ca·cb, sa·cb, sb·ca),
/// leaving `DX`/`AX` as the last `IMUL`/`SHLD` and `NEG` leave them.
fn rotation_products(c: &mut Cpu) -> [u16; 4] {
    let (sa, sb, ca, cb) = (c.x(EAX), c.x(EBX), c.x(ECX), c.x(EBP));
    let p1 = qmul(sa, sb).0;
    let p2 = qmul(ca, cb).0;
    let p3 = qmul(sa, cb).0.wrapping_neg();
    let (p4, lo) = qmul(sb, ca);
    let p4 = p4.wrapping_neg();
    c.set_x(EAX, lo);
    c.set_x(EDX, p4);
    [p1, p2, p3, p4]
}

/// Port of `EL2MATH3:09e9` (slot `DS:02e0`): build a 3×3 Q15 rotation matrix at `ES:SI` (words
/// 0–10h) from the angles at `DS:1f35` (a) and `DS:1f37` (b), using [`SINE_TABLE`]:
///
/// | `+0` −sb·ca | `+2` ca·cb | `+4` −sa |
/// | `+6` cb | `+8` sb | `+A` 0 |
/// | `+C` sa·sb | `+E` −sa·cb | `+10` −ca |
///
/// (a row-major reading is *assumed*.) Registers: `AX` = sin a… as left by the last product.
pub fn rotation_02e0(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let steps = sines(c, bus);
    let (sa, sb, ca, cb) = (c.x(EAX), c.x(EBX), c.x(ECX), c.x(EBP));
    let [p1, p2, p3, p4] = rotation_products(c);
    let at = |k: u16| lin(c.es, c.x(ESI).wrapping_add(k));
    bus.write16(at(0x4), sa.wrapping_neg());
    bus.write16(at(0x8), sb);
    bus.write16(at(0x10), ca.wrapping_neg());
    bus.write16(at(0x6), cb);
    bus.write16(at(0xa), 0);
    bus.write16(at(0xc), p1);
    bus.write16(at(0x2), p2);
    bus.write16(at(0xe), p3);
    bus.write16(at(0x0), p4);
    c.retf(bus);
    steps + 26
}

/// Port of `EL2MATH3:0ac9` (slot `DS:02dc`): the companion of [`rotation_02e0`] with another
/// layout at `ES:SI`:
///
/// | `+0` ca·cb | `+2` sa | `+4` −sb·ca |
/// | `+6` −sa·cb | `+8` ca | `+A` sa·sb |
/// | `+C` sb | `+E` 0 | `+10` cb |
pub fn rotation_02dc(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let steps = sines(c, bus);
    let (sa, sb, ca, cb) = (c.x(EAX), c.x(EBX), c.x(ECX), c.x(EBP));
    let [p1, p2, p3, p4] = rotation_products(c);
    let at = |k: u16| lin(c.es, c.x(ESI).wrapping_add(k));
    bus.write16(at(0x2), sa);
    bus.write16(at(0xc), sb);
    bus.write16(at(0x8), ca);
    bus.write16(at(0x10), cb);
    bus.write16(at(0xe), 0);
    bus.write16(at(0xa), p1);
    bus.write16(at(0x0), p2);
    bus.write16(at(0x6), p3);
    bus.write16(at(0x4), p4);
    c.retf(bus);
    steps + 24
}

/// Every routine in this module: (overlay offset, vector slot, name, port).
pub const PORTS: &[(u16, u16, &str, Port)] = &[
    (0x1cf5, 0x02b4, "mul", |c, b| with_regs(c, b, mul)),
    (0x1cfe, 0x02b8, "mul", |c, b| with_regs(c, b, mul)),
    (0x1d07, 0x0288, "div_norm", |c, b| with_regs(c, b, div_norm)),
    (0x1d2e, 0x02b0, "div", |c, b| with_regs(c, b, div)),
    (0x1d41, 0x02d0, "sub", |c, b| with_regs(c, b, sub)),
    (0x1d43, 0x027c, "add", |c, b| with_regs(c, b, add)),
    (0x1d63, 0x02d4, "sub32", |c, b| with_regs(c, b, sub32)),
    (0x1d6d, 0x0280, "add32", |c, b| with_regs(c, b, add32)),
    (0x1da1, 0x0284, "add_unsigned", |c, b| with_regs(c, b, add_unsigned)),
    (0x1db6, 0x02c4, "from_u32", |c, b| with_regs(c, b, from_u32)),
    (0x1dd2, 0x02bc, "from_i32", |c, b| with_regs(c, b, from_i32)),
    (0x1dec, 0x02c0, "norm_i32", |c, b| with_regs(c, b, norm_i32)),
    (0x1e03, 0x028c, "to_fixed32", |c, b| with_regs(c, b, to_fixed32)),
    (0x1e22, 0x0290, "to_fixed32_raw", |c, b| with_regs(c, b, to_fixed32_raw)),
    (0x1e34, 0x029c, "to_int16", |c, b| with_regs(c, b, to_int16)),
    (0x1e4a, 0x0294, "to_fixed32_unsigned", |c, b| with_regs(c, b, to_fixed32_unsigned)),
    (0x1e65, 0x0298, "to_fixed64", |c, b| with_regs(c, b, to_fixed64)),
    (0x1e80, 0x02a8, "norm_unsigned", |c, b| with_regs(c, b, norm_unsigned)),
    (0x1e8d, 0x02a0, "norm16", |c, b| with_regs(c, b, norm16)),
    (0x1e9a, 0x02a4, "norm32", |c, b| with_regs(c, b, norm32)),
    (0x1eaf, 0x02ac, "norm_positive", |c, b| with_regs(c, b, norm_positive)),
    (0x0967, 0x031c, "sqrt32", sqrt32),
    (0x0952, 0x0320, "length3", length3),
    (0x0922, 0x02c8, "sqrt_float", sqrt_float),
    (0x0938, 0x02cc, "sqrt_float32", sqrt_float32),
    (0x0ba1, 0x0278, "frame_e76", frame_e76),
    (0x0bcf, 0x0274, "frame_5598", frame_5598),
    (0x09e9, 0x02e0, "rotation_02e0", rotation_02e0),
    (0x0ac9, 0x02dc, "rotation_02dc", rotation_02dc),
    (0x19c2, 0x0314, "project", project),
    (0x1ba7, 0x030c, "init_list", init_list),
    (0x0d2b, 0x0300, "transform_vertex", transform_vertex),
    (0x0ccc, 0x02fc, "project_vertex", project_vertex),
    (0x0c35, 0x0304, "vertex_position", vertex_position),
];
