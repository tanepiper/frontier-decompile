//! The commodity market (`EL2M07`, docs/ECONOMY.md): prices and stocks made per system on arrival.
//!
//! The ports are register- and memory-exact and return the original's instruction counts; the
//! commodity and system-factor tables are read from the user's `EL2M07.OVL` in memory. Random
//! draws go through the native `random32` (slot `DS:012c`, [`crate::rng`]), so a market is a
//! function of the system and the shared generator's state.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDX, ESP};

fn near_ret(c: &mut Cpu, bus: &mut dyn Bus) {
    let sp = c.x(ESP);
    c.ip = bus.read16(lin(c.ss, sp));
    c.set_x(ESP, sp.wrapping_add(2));
}

/// `IMUL` (16 × 16 → 32, signed): (`DX`, `AX`).
fn imul(a: u16, b: u16) -> (u16, u16) {
    let p = (a as i16 as i32) * (b as i16 as i32);
    ((p >> 16) as u16, p as u16)
}

/// `SHL AX,1; RCL DX,1` on `DX:AX`: the new pair and the overflow flag the `RCL` leaves
/// (MSB of the new `DX` xor the bit shifted out of it).
fn shl32(dx: u16, ax: u16) -> (u16, u16, bool) {
    let c1 = ax & 0x8000 != 0;
    let ax2 = ax << 1;
    let c2 = dx & 0x8000 != 0;
    let dx2 = (dx << 1) | c1 as u16;
    (dx2, ax2, (dx2 & 0x8000 != 0) != c2)
}

/// `CALLF [DS:012c]` (`random32`) through the native port: the instructions including the `CALLF`.
fn call_random32(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> u64 {
    let sp = c.x(ESP);
    bus.write16(lin(c.ss, sp.wrapping_sub(2)), c.cs);
    bus.write16(lin(c.ss, sp.wrapping_sub(4)), ret_ip);
    c.set_x(ESP, sp.wrapping_sub(4));
    let (cs, ip) = (c.cs, c.ip);
    let n = crate::rng::random32_port(c, bus);
    c.cs = cs;
    c.ip = ip;
    1 + n
}

/// The system-factor tables in `EL2M07` the commodity formulas index by `DS:07bc`, `07c8`, `07c0`.
const F_261: u16 = 0x0261;
const F_251: u16 = 0x0251;
const F_275: u16 = 0x0275;
const F_285: u16 = 0x0285;
const F_299: u16 = 0x0299;

fn table(c: &Cpu, bus: &mut dyn Bus, base: u16, index_slot: u16) -> u16 {
    let bx = bus.read16(lin(c.ds, index_slot));
    bus.read16(lin(c.cs, base.wrapping_add(bx)))
}

/// Port of `EL2M07:0855`, *observed*: a commodity's price. With the commodity's coefficients at
/// `CS:BP` and its flag byte in `CL`: `p = [BP+2]·F261[DS:07bc]·F251[DS:07c8]` (Q15 products),
/// negated if `CL` bit 3, `+ [BP+0]`, doubled if `CL` bit 7, `· F275[DS:07c0]`, doubled with
/// saturation to 7FFFh; then `· (3E00h + random32 & 3FFh)` and doubled twice, saturating to
/// `7FFF:FFFF`. Result in `DX:AX`. A near routine.
pub fn price_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (cs, bp) = (c.cs, c.x(EBP));
    let cl = c.x(ECX) as u8;
    let k = |bus: &mut dyn Bus, o: u16| bus.read16(lin(cs, bp.wrapping_add(o)));
    let mut n: u64 = 13;
    let a = k(bus, 2);
    let (d, x) = imul(a, table(c, bus, F_261, 0x07bc));
    let (d, _, _) = shl32(d, x);
    let (d, x) = imul(d, table(c, bus, F_251, 0x07c8));
    let (mut ax, _, _) = shl32(d, x);
    if cl & 0x08 != 0 {
        ax = ax.wrapping_neg();
        n += 1;
    }
    ax = ax.wrapping_add(k(bus, 0));
    n += 3;
    if cl & 0x80 != 0 {
        ax <<= 1;
        n += 1;
    }
    let bx = bus.read16(lin(c.ds, 0x07c0));
    let (d, x) = imul(ax, bus.read16(lin(cs, F_275.wrapping_add(bx))));
    let (d, _, _) = shl32(d, x);
    let of = ((d << 1) & 0x8000 != 0) != (d & 0x8000 != 0);
    let mut ax = d << 1;
    n += 7;
    if of {
        ax = 0x7fff;
        n += 1;
    }
    // PUSH AX; CALLF [012c]; AND AX,3FFh; ADD AX,3E00h; POP BX; IMUL BX; SHL; RCL; JO
    let sp = c.x(ESP).wrapping_sub(2);
    bus.write16(lin(c.ss, sp), ax);
    c.set_x(ESP, sp);
    n += 1 + call_random32(c, bus, 0x08a4);
    c.set_x(ESP, sp.wrapping_add(2));
    let r = (c.x(EAX) & 0x3ff).wrapping_add(0x3e00);
    c.set_x(EBX, ax);
    let (d, x) = imul(r, ax);
    let (d, x, of1) = shl32(d, x);
    n += 7; // AND; ADD; POP BX; IMUL; SHL; RCL; JO
    let (dx, ax) = if of1 {
        n += 3;
        (0x7fff, 0xffff)
    } else {
        let (d2, x2, of2) = shl32(d, x);
        n += 3;
        if of2 {
            n += 3;
            (0x7fff, 0xffff)
        } else {
            n += 1;
            (d2, x2)
        }
    };
    c.set_x(EAX, ax);
    c.set_x(EDX, dx);
    near_ret(c, bus);
    n
}

/// Port of `EL2M07:08c0`, *observed*: a commodity's stock. `s = [BP+6]·F251[DS:07c8]` (Q15),
/// negated unless `CL` bit 3, `+ [BP+4]`, `· F285[DS:07bc]`, divided by 8 if `CL` bit 7,
/// `· F299[DS:07c0]` (Q15, kept in `BX`); then `DX = BX + |BX| · (4000h + random32 & 3FFFh)`
/// (Q15), 0 if that is negative. A near routine.
pub fn stock_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (cs, bp) = (c.cs, c.x(EBP));
    let cl = c.x(ECX) as u8;
    let k = |bus: &mut dyn Bus, o: u16| bus.read16(lin(cs, bp.wrapping_add(o)));
    let mut n: u64 = 8;
    let (d, x) = imul(k(bus, 6), table(c, bus, F_251, 0x07c8));
    let (mut ax, _, _) = shl32(d, x);
    if cl & 0x08 == 0 {
        ax = ax.wrapping_neg();
        n += 1;
    }
    ax = ax.wrapping_add(k(bus, 4));
    let (d, x) = imul(ax, table(c, bus, F_285, 0x07bc));
    let (mut ax, _, _) = shl32(d, x);
    n += 8;
    if cl & 0x80 != 0 {
        ax >>= 3;
        n += 1;
    }
    let (d, x) = imul(ax, table(c, bus, F_299, 0x07c0));
    let (bx, _, _) = shl32(d, x);
    c.set_x(EBX, bx);
    n += 5;
    n += call_random32(c, bus, 0x0908);
    let ax = c.x(EAX);
    let ah = ((ax >> 8) as u8 & 0x3f).wrapping_add(0x40);
    let ax = (ax & 0x00ff) | (ah as u16) << 8;
    let mut dx = bx;
    n += 5;
    if dx & 0x8000 != 0 {
        dx = dx.wrapping_neg();
        n += 1;
    }
    let (d, x) = imul(ax, dx);
    let (d, x, _) = shl32(d, x);
    let mut dx = d.wrapping_add(bx);
    n += 5;
    if dx & 0x8000 != 0 {
        dx = 0;
        n += 1;
    }
    c.set_x(EAX, x);
    c.set_x(EDX, dx);
    near_ret(c, bus);
    n + 1
}

/// Every ported routine of `EL2M07` (near routines: slot 0).
pub const PORTS_M07: &[(u16, u16, &str, Port)] = &[(0x0855, 0, "price", price_port), (0x08c0, 0, "stock", stock_port)];

#[allow(dead_code)]
const _UNUSED: u64 = DECLINE;
