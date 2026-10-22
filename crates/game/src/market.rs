//! The commodity market (`EL2M07`, docs/ECONOMY.md): prices and stocks made per system on arrival.
//!
//! The ports are register- and memory-exact and return the original's instruction counts; the
//! commodity and system-factor tables are read from the user's `EL2M07.OVL` in memory. Random
//! draws go through the native `random32` (slot `DS:012c`, [`crate::rng`]), so a market is a
//! function of the system and the shared generator's state.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDX, EDI, ESI, ESP};

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

fn push16(c: &mut Cpu, bus: &mut dyn Bus, v: u16) {
    let sp = c.x(ESP).wrapping_sub(2);
    bus.write16(lin(c.ss, sp), v);
    c.set_x(ESP, sp);
}

fn pop16(c: &mut Cpu, bus: &mut dyn Bus) -> u16 {
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    bus.read16(lin(c.ss, sp))
}

fn push32(c: &mut Cpu, bus: &mut dyn Bus, v: u32) {
    let sp = c.x(ESP).wrapping_sub(4);
    bus.write32(lin(c.ss, sp), v);
    c.set_x(ESP, sp);
}

fn pop32(c: &mut Cpu, bus: &mut dyn Bus) -> u32 {
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(4));
    bus.read32(lin(c.ss, sp))
}

fn r16(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u16 {
    bus.read16(lin(c.ds, off))
}

fn w16(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u16) {
    bus.write16(lin(c.ds, off), v)
}

fn r32(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u32 {
    bus.read32(lin(c.ds, off))
}

fn w32(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u32) {
    bus.write32(lin(c.ds, off), v)
}

/// `CALLF [slot]` to a native port of a main-program routine; the count includes the `CALLF`.
fn callf(c: &mut Cpu, bus: &mut dyn Bus, port: Port, ret_ip: u16) -> u64 {
    let (cs, ip) = (c.cs, c.ip);
    push16(c, bus, cs);
    push16(c, bus, ret_ip);
    let n = port(c, bus);
    c.cs = cs;
    c.ip = ip;
    1 + n
}

/// `INC [lock]; MOV SI,slot; LES BP,[SI+396h]; CALLF ES:[BP+20h]; DEC [lock]`: handler `+20` of
/// the module in `slot`, run by its native port when the module is resident and its handler is
/// at `handler` (else `None`: the original would load the module). Returns the five
/// instructions and the handler's.
fn module_h20(c: &mut Cpu, bus: &mut dyn Bus, slot: u16, lock: u16, handler: u16, port: Port, ret_ip: u16) -> Option<u64> {
    let ds = c.ds;
    let hdr_off = bus.read16(lin(ds, 0x0396 + slot));
    let hdr_seg = bus.read16(lin(ds, 0x0398 + slot));
    let off = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x20)));
    let seg = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x22)));
    if off != handler {
        return None;
    }
    let l = bus.read8(lin(ds, lock));
    bus.write8(lin(ds, lock), l.wrapping_add(1));
    c.set_x(ESI, slot);
    c.set_x(EBP, hdr_off);
    c.es = hdr_seg;
    let (cs, ip) = (c.cs, c.ip);
    push16(c, bus, cs);
    push16(c, bus, ret_ip);
    c.cs = seg;
    c.ip = off;
    let n = port(c, bus);
    if n == DECLINE {
        return None;
    }
    c.cs = cs;
    c.ip = ip;
    let l = bus.read8(lin(ds, lock));
    bus.write8(lin(ds, lock), l.wrapping_sub(1));
    Some(5 + n)
}

/// Module 2's handler `+20` ([`crate::galaxy::nearby_port`]), lock byte `DS:0598`.
fn nearby(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> Option<u64> {
    module_h20(c, bus, 0x10, 0x0598, 0x3ddd, crate::galaxy::nearby_port, ret_ip)
}

/// Module 6's handler `+20` ([`crate::sysdata::system_data_port`]), lock byte `DS:059c`.
fn system_data(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> Option<u64> {
    module_h20(c, bus, 0x30, 0x059c, 0x3eb9, crate::sysdata::system_data_port, ret_ip)
}

/// Port of `EL2M07:0b03`, *observed*: one bulletin-board advert. `random32` picks one of 16
/// entries of the jump table at `EL2M07:059b` (`BX` = `AX & 1Eh`), with `DS:07d8` = `DS:43a3` −
/// `DS:43a5` (0 if negative); `DS:07d4` is kept. A near routine, called `DS:43a1` + 1 times by
/// the market builder. Declines when module 2 or 6 is not resident.
pub fn advert_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    advert(c, bus).unwrap_or(DECLINE)
}

fn advert(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let v = r16(c, bus, 0x07d4);
    push16(c, bus, v);
    let mut n = 1 + call_random32(c, bus, 0x0b0b);
    let bx = c.x(EAX) & 0x1e;
    c.set_x(EBX, bx);
    let (a, b) = (r16(c, bus, 0x43a3), r16(c, bus, 0x43a5));
    let ax = if a < b {
        n += 1;
        0
    } else {
        a - b
    };
    c.set_x(EAX, ax);
    w16(c, bus, 0x07d8, ax);
    n += 6; // MOV BX,AX; AND; MOV AX; SUB; JNC; MOV [7d8]
    let target = bus.read16(lin(c.cs, 0x059b + bx));
    push16(c, bus, 0x0b23);
    n += 1 + advert_kind(c, bus, target)?;
    let v = pop16(c, bus);
    w16(c, bus, 0x07d4, v);
    near_ret(c, bus);
    Some(n + 2)
}

/// The entries of the jump table at `EL2M07:059b`, each ending in `RET`.
fn advert_kind(c: &mut Cpu, bus: &mut dyn Bus, target: u16) -> Option<u64> {
    let room = r16(c, bus, 0x07d8);
    let ret = |c: &mut Cpu, bus: &mut dyn Bus, n: u64| {
        near_ret(c, bus);
        Some(n + 1)
    };
    match target {
        // 0ce9: random(8) of the adverts from string 9957h.
        0x0ce9 => {
            c.set_x(EAX, 8);
            w16(c, bus, 0x07cc, 0x9957);
            Some(3 + transport(c, bus)?)
        }
        // 0cf4: random(4) from 995Fh, when DS:07d8 > 10h.
        0x0cf4 => {
            if room <= 0x10 {
                return ret(c, bus, 2);
            }
            c.set_x(EAX, 4);
            w16(c, bus, 0x07cc, 0x995f);
            Some(5 + transport(c, bus)?)
        }
        // 0d0d: random(4) from 9963h, when DS:07d8 > 51h.
        0x0d0d => {
            if room <= 0x51 {
                return ret(c, bus, 2);
            }
            c.set_x(EAX, 4);
            w16(c, bus, 0x07cc, 0x9963);
            Some(4 + transport(c, bus)?)
        }
        0x0bb0 => military(c, bus, 0x100, 3, 0x067b, 0x99a5, 0x9856),
        0x0c19 => military(c, bus, 0x271, 1, 0x068b, 0x994f, 0x9854),
        0x0c81 => military(c, bus, 0x510, 1, 0x069b, 0x994f, 0x9855),
        _ => None,
    }
}

/// `EL2M07:0d1d`, *observed*: a delivery or passenger advert (*inferred*). `DS:07cc` = the first
/// string, `AX` = how many strings to choose from. The destination is a random system within
/// `random(3)` sectors of the current one (`DS:208b`), through module 2; it must have byte `+4`
/// of its system record at least 3, and the same allegiance (byte `+0Ah` >> 6) as `DS:2243`. The
/// record gets string 9853h, the destination in `DS:07c0`, a deadline `DS:1fc5` + 10 +
/// `random(14)` days in `DS:07c4`, a reward `50·(2·r3 + string + chosen − 9957h + 3)` in `DS:07c8`,
/// the string in `DS:07cc` and a `random32` draw in `DS:07dc` ([`store_advert`]).
fn transport(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let mut n = callf(c, bus, crate::rng::random_port, 0x0d21);
    let v = c.x(EAX);
    push16(c, bus, v);
    let v = r16(c, bus, 0x07cc);
    push16(c, bus, v);
    let here = r32(c, bus, 0x208b);
    c.r[EAX] = here;
    w32(c, bus, 0x07c0, here);
    c.set_x(EAX, 3);
    n += 5 + callf(c, bus, crate::rng::random_port, 0x0d35);
    let r3 = c.x(EAX);
    push16(c, bus, r3);
    w16(c, bus, 0x07c4, r3);
    c.set_x(EAX, 1);
    n += 3 + nearby(c, bus, 0x0d4b)?;
    n += 2;
    let sp = c.x(ESP);
    if c.x(EAX) == 0 {
        // 0d09: ADD SP,6; RET.
        c.set_x(ESP, sp.wrapping_add(6));
        near_ret(c, bus);
        return Some(n + 2);
    }
    let id = r32(c, bus, 0x07bc);
    c.r[EAX] = id;
    push32(c, bus, id);
    w32(c, bus, 0x07c0, id);
    c.set_l(EAX, 1);
    n += 4 + system_data(c, bus, 0x0d6e)?;
    n += 2;
    let refuse = |c: &mut Cpu, bus: &mut dyn Bus, n: u64| {
        // 0d06: ADD SP,4; ADD SP,6; RET.
        let sp = c.x(ESP);
        c.set_x(ESP, sp.wrapping_add(10));
        near_ret(c, bus);
        Some(n + 3)
    };
    if c.l(EAX) < 3 {
        return refuse(c, bus, n);
    }
    let v = r16(c, bus, 0x07c4) >> 6;
    w16(c, bus, 0x07c4, v);
    c.set_x(EAX, v);
    n += 4;
    if v != r16(c, bus, 0x2243) {
        return refuse(c, bus, n);
    }
    n += call_random32(c, bus, 0x0d88) + 1;
    let r = c.r[EAX];
    w32(c, bus, 0x07dc, r);
    c.set_x(EAX, 0x0e);
    n += 1 + callf(c, bus, crate::rng::random_port, 0x0d93);
    let days = c.x(EAX).wrapping_add(0x0a);
    let due = (days as i16 as i32 as u32).wrapping_add(r32(c, bus, 0x1fc5));
    c.r[EAX] = due;
    w32(c, bus, 0x07c4, due);
    n += 4;
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    let v = pop16(c, bus);
    w16(c, bus, 0x07c8, v);
    let v = pop16(c, bus);
    w16(c, bus, 0x07cc, v);
    let chosen = pop16(c, bus);
    let s = r16(c, bus, 0x07cc).wrapping_add(chosen);
    w16(c, bus, 0x07cc, s);
    let a = r16(c, bus, 0x07c8).wrapping_shl(1).wrapping_add(s).wrapping_sub(0x9957).wrapping_add(3);
    let p = a as u32 * 0x32;
    c.set_x(EAX, p as u16);
    c.set_x(EDX, (p >> 16) as u16);
    w16(c, bus, 0x07c8, p as u16);
    w16(c, bus, 0x07ca, (p >> 16) as u16);
    c.set_x(EAX, 0x9853);
    n += 5 + 9 + 1;
    Some(n + store_advert(c, bus))
}

/// `EL2M07:0bb0`, `0c19`, `0c81`, *observed*: a military advert (*inferred*), when `DS:07d8` is
/// above `room`. The destination comes from [`military_target`] and needs byte `+4` of its
/// record at least `min`. The record gets string `code`, a deadline `DS:1fc5` + 14 + (a
/// `random32` draw & 3Fh) days, a reward from the 8-word table at `EL2M07:table` and the string
/// `base` + an index from 0 to 7.
fn military(c: &mut Cpu, bus: &mut dyn Bus, room: u16, min: u8, table: u16, base: u16, code: u16) -> Option<u64> {
    let ret_at = |c: &mut Cpu, bus: &mut dyn Bus, n: u64| {
        near_ret(c, bus);
        Some(n + 1)
    };
    if r16(c, bus, 0x07d8) <= room {
        return ret_at(c, bus, 2);
    }
    let ret_ip = match table {
        0x067b => 0x0bbd,
        0x068b => 0x0c26,
        _ => 0x0c8e,
    };
    push16(c, bus, ret_ip);
    let mut n = 3 + military_target(c, bus)?;
    n += 2;
    if bus.read8(lin(c.ds, 0x07bc)) < min {
        return ret_at(c, bus, n);
    }
    let v = r32(c, bus, 0x07c0);
    push32(c, bus, v);
    n += 1 + call_random32(c, bus, ret_ip.wrapping_add(0x10));
    let r = c.r[EAX];
    w32(c, bus, 0x07dc, r);
    let k = c.x(EDX) & 0x0e;
    c.set_x(EDX, k);
    w16(c, bus, 0x07cc, k);
    n += 3 + call_random32(c, bus, ret_ip.wrapping_add(0x1f));
    let edx = c.r[EDX] & 0x3f;
    c.r[EDX] = edx;
    let ebx = r32(c, bus, 0x1fc5).wrapping_add(0x0e).wrapping_add(edx);
    c.r[EBX] = ebx;
    w32(c, bus, 0x07c4, ebx);
    n += 5;
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    c.set_x(EBX, k);
    let reward = bus.read16(lin(c.cs, table.wrapping_add(k)));
    c.set_x(EAX, reward);
    w16(c, bus, 0x07c8, reward);
    w16(c, bus, 0x07ca, 0);
    w16(c, bus, 0x07cc, (k >> 1).wrapping_add(base));
    c.set_x(EAX, code);
    n += 1 + 4 + 2 + 2;
    Some(n + store_advert(c, bus))
}

/// `EL2M07:0b28`, *observed*: a random system within `random(3)` sectors of the capital of
/// allegiance `DS:2243` (the ids at `EL2M07:066f`), through module 2; when there is one, `DS:07c0`
/// = its id and `DS:07bc` = byte `+4` of its system record (module 6, `AL` = 1).
/// `DS:07d0`–`07df` are kept. A near routine.
fn military_target(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let bx = r16(c, bus, 0x2243).wrapping_shl(2);
    c.set_x(EBX, bx);
    let capital = bus.read32(lin(c.cs, 0x066f + bx));
    c.r[EAX] = capital;
    w32(c, bus, 0x07c0, capital);
    c.set_x(EAX, 3);
    let mut n = 5 + callf(c, bus, crate::rng::random_port, 0x0b40);
    let r = c.x(EAX);
    w16(c, bus, 0x07c4, r);
    c.set_x(EAX, 1);
    n += 2 + nearby(c, bus, 0x0b55)?;
    n += 2;
    if c.x(EAX) == 0 {
        near_ret(c, bus);
        return Some(n + 1);
    }
    let id = r32(c, bus, 0x07bc);
    push32(c, bus, id);
    c.r[EAX] = id;
    w32(c, bus, 0x07c0, id);
    c.set_l(EAX, 1);
    for off in [0x07d0, 0x07d4, 0x07d8, 0x07dc] {
        let v = r32(c, bus, off);
        push32(c, bus, v);
    }
    n += 8 + system_data(c, bus, 0x0b8f)?;
    let v = c.x(EAX);
    w16(c, bus, 0x07bc, v);
    for off in [0x07dc, 0x07d8, 0x07d4, 0x07d0, 0x07c0] {
        let v = pop32(c, bus);
        w32(c, bus, off, v);
    }
    near_ret(c, bus);
    Some(n + 7)
}

/// `EL2M07:0dd3`, *observed*: the first free 18h-byte slot of the 25 from `DS:43a7` (a zero first
/// dword) gets `AX`, then the dwords `DS:07c0`, `07c4`, `07c8`, `07cc`, `07dc`, and a zero dword
/// after it (the next slot's first). Nothing when all are full. Ends in `RET`.
fn store_advert(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let ax = c.x(EAX);
    let mut si: u16 = 0x438f;
    let mut cx: u16 = 0x18;
    let mut n = 2;
    loop {
        si = si.wrapping_add(0x18);
        n += 3;
        if r32(c, bus, si) == 0 {
            break;
        }
        cx = cx.wrapping_sub(1);
        n += 2;
        if cx & 0x8000 != 0 {
            c.set_x(ESI, si);
            c.set_x(ECX, cx);
            near_ret(c, bus);
            return n + 2;
        }
    }
    c.set_x(ESI, si);
    c.set_x(ECX, cx);
    w16(c, bus, si, ax);
    let mut last = 0;
    for (k, off) in [0x07c0u16, 0x07c4, 0x07c8, 0x07cc, 0x07dc].iter().enumerate() {
        last = r32(c, bus, *off);
        w32(c, bus, si.wrapping_add(4 + 4 * k as u16), last);
    }
    c.r[EAX] = last;
    w32(c, bus, si.wrapping_add(0x18), 0);
    near_ret(c, bus);
    n + 13
}

/// Port of `EL2M07:101a`, *observed*: maybe add one entry to a station's list. A `random32` draw
/// with bit 3 of `AL` clear, and fewer than 14 entries in the word at `DS:[SI+1d8h]`, adds the
/// word from [`list_pick`] and `DS:07bc` at `DS:DI`, moves `DI` on by 4 and counts the entry.
/// `DS:07d0` is the list's kind (0, 1 and 9 from the builder). A near routine.
pub fn station_list_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut n = call_random32(c, bus, 0x101e);
    n += 2;
    if c.l(EAX) & 0x08 != 0 {
        near_ret(c, bus);
        return n + 1;
    }
    let si = c.x(ESI);
    let count = r16(c, bus, si.wrapping_add(0x1d8));
    n += 2;
    if count >= 0x0e {
        near_ret(c, bus);
        return n + 1;
    }
    push16(c, bus, 0x102c);
    n += 1 + list_pick(c, bus);
    let di = c.x(EDI);
    w16(c, bus, di, c.x(EAX));
    let v = r16(c, bus, 0x07bc);
    c.set_x(EAX, v);
    w16(c, bus, di.wrapping_add(2), v);
    c.set_x(EDI, di.wrapping_add(4));
    let count = r16(c, bus, si.wrapping_add(0x1d8));
    w16(c, bus, si.wrapping_add(0x1d8), count.wrapping_add(1));
    near_ret(c, bus);
    n + 6
}

/// `EL2M07:103c`, *observed*: one entry for a list of kind `k` = `DS:07d0`. The pair of words at
/// `EL2M07:0567` + 4·(`k`, plus 2·`DS:2243` when `k` < 2) gives a scale (to `DS:07c8`) and a table.
/// A `random32` draw `EAX`:`EDX` goes to `DS:07bc`:`07c0`, or `DS:07cc` and its halves swapped
/// when `k` = 12. For `k` = 1 with `DS:2239` below 4, `DS:07bc` = `DS:07bc`/2 + the high word of
/// `DS:2239`·(`DS:07c0`/8). The result is the table's word at 2·high(scale·`DS:07bc`), in `AX`.
/// A near routine.
fn list_pick(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let k = r16(c, bus, 0x07d0);
    let mut n = 3;
    let mut bx: u16 = 0;
    if k < 2 {
        bx = r16(c, bus, 0x2243).wrapping_shl(1);
        n += 2;
    }
    bx = bx.wrapping_add(k).wrapping_shl(2);
    let scale = bus.read16(lin(c.cs, 0x0567u16.wrapping_add(bx)));
    c.set_x(EAX, scale);
    w16(c, bus, 0x07c8, scale);
    let mut bx = bus.read16(lin(c.cs, 0x0569u16.wrapping_add(bx)));
    n += 5 + call_random32(c, bus, 0x1063);
    w32(c, bus, 0x07bc, c.r[EAX]);
    w32(c, bus, 0x07c0, c.r[EDX]);
    n += 4;
    if k == 0x0c {
        let v = r32(c, bus, 0x07cc);
        w32(c, bus, 0x07bc, v);
        let v = v.rotate_right(16);
        c.r[EAX] = v;
        w32(c, bus, 0x07c0, v);
        n += 4;
    }
    n += 2;
    if k == 1 {
        let ax = r16(c, bus, 0x2239);
        c.set_x(EAX, ax);
        n += 3;
        if ax < 4 {
            let v = r16(c, bus, 0x07c0) >> 3;
            w16(c, bus, 0x07c0, v);
            let p = ax as u32 * v as u32;
            c.set_x(EAX, p as u16);
            c.set_x(EDX, (p >> 16) as u16);
            let b = (r16(c, bus, 0x07bc) >> 1).wrapping_add((p >> 16) as u16);
            w16(c, bus, 0x07bc, b);
            n += 4;
        }
    }
    let p = scale as u32 * r16(c, bus, 0x07bc) as u32;
    let dx = ((p >> 16) as u16).wrapping_shl(1);
    c.set_x(EDX, dx);
    bx = bx.wrapping_add(dx);
    c.set_x(EBX, bx);
    c.set_x(EAX, bus.read16(lin(c.cs, bx)));
    near_ret(c, bus);
    n + 6
}

/// Port of `EL2M07:0923`, *observed*: one station event. A `random32` draw picks one of 32 entries
/// of the jump table at `EL2M07:03a1` (`EAX & 3Eh`), with `DS:07c0` = the draw's `EDX`;
/// `DS:07d4` and `DS:07d8` are kept. `SI` is the station record, `DI` the next free event entry
/// (16 bytes each from `[SI+7Eh]`, counted in `[SI+19Eh]`). A near routine. Declines when module
/// 2 or 6 is not resident, or the slot `DS:00b4` is not the main program's object lookup.
pub fn event_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    event(c, bus).unwrap_or(DECLINE)
}

fn event(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let mut n = call_random32(c, bus, 0x0927);
    let k = c.r[EAX] & 0x3e;
    c.r[EAX] = k;
    w32(c, bus, 0x07c0, c.r[EDX]);
    for off in [0x07d4, 0x07d8] {
        let v = r16(c, bus, off);
        push16(c, bus, v);
    }
    let target = bus.read16(lin(c.cs, 0x03a1 + k as u16));
    push16(c, bus, 0x0940);
    n += 5 + event_kind(c, bus, target)?;
    for off in [0x07d8, 0x07d4] {
        let v = pop16(c, bus);
        w16(c, bus, off, v);
    }
    near_ret(c, bus);
    Some(n + 3)
}

/// `BTS word [SI+7Dh],bit`: sets the bit and returns its old value.
fn bts_flag(c: &Cpu, bus: &mut dyn Bus, bit: u16) -> bool {
    let a = c.x(ESI).wrapping_add(0x7d);
    let v = r16(c, bus, a);
    w16(c, bus, a, v | 1 << bit);
    v & 1 << bit != 0
}

/// The commodity bytes from the system record (`DS:2219`, 31 of them): the highest index with bit
/// 7 set, scanning down from 30 (`MOV BX,1Eh` … `DEC BX; JNS`). Returns the instructions and
/// whether one was found.
fn scan_flagged(c: &mut Cpu, bus: &mut dyn Bus) -> (u64, bool) {
    let mut bx: u16 = 0x1e;
    let mut n = 1;
    loop {
        n += 2;
        if bus.read8(lin(c.ds, 0x2219u16.wrapping_add(bx))) & 0x80 != 0 {
            c.set_x(EBX, bx);
            return (n, true);
        }
        bx = bx.wrapping_sub(1);
        n += 2;
        if bx & 0x8000 != 0 {
            c.set_x(EBX, bx);
            return (n, false);
        }
    }
}

/// The entries of the jump table at `EL2M07:03a1`, each ending in `RET` (most through the
/// writer [`write_event`]).
fn event_kind(c: &mut Cpu, bus: &mut dyn Bus, target: u16) -> Option<u64> {
    let ret = |c: &mut Cpu, bus: &mut dyn Bus, n: u64| {
        near_ret(c, bus);
        Some(n + 1)
    };
    let si = c.x(ESI);
    match target {
        // 12dc: once per station (flag bit 0), when a commodity is flagged: string 9828h with the
        // station object's ES:[SI+76h] rotated left by 17 in DS:07c0 and 07e4.
        0x12dc => {
            let f = bus.read8(lin(c.ds, si.wrapping_add(0x7d)));
            if f & 1 != 0 {
                return ret(c, bus, 2);
            }
            bus.write8(lin(c.ds, si.wrapping_add(0x7d)), f | 1);
            push16(c, bus, si);
            let (m, found) = scan_flagged(c, bus);
            let mut n = 4 + m;
            if !found {
                // JMP 1327; ADD SP,2; RET.
                let sp = c.x(ESP);
                c.set_x(ESP, sp.wrapping_add(2));
                return ret(c, bus, n + 2);
            }
            let slot_off = r16(c, bus, 0x00b4);
            if slot_off != 0x085a {
                return None;
            }
            let bx = bus.read8(lin(c.ds, si.wrapping_add(0x7c))) as u16;
            let p = r32(c, bus, 0x56fb);
            c.r[EAX] = p;
            w32(c, bus, 0x07ec, p);
            // CALLF [00b4] = FRONTIER.EXE 1000:085a: SHL BX,1; LES SI,[07ec]; ADD SI,[BX+06d6]; RETF.
            let bx = bx << 1;
            c.set_x(EBX, bx);
            c.es = r16(c, bus, 0x07ee);
            let obj = r16(c, bus, 0x07ec).wrapping_add(r16(c, bus, 0x06d6u16.wrapping_add(bx)));
            let v = bus.read32(lin(c.es, obj.wrapping_add(0x76))).rotate_left(17);
            c.r[EAX] = v;
            w32(c, bus, 0x07c0, v);
            w32(c, bus, 0x07e4, v);
            let si = pop16(c, bus);
            c.set_x(ESI, si);
            w32(c, bus, 0x07c4, 0);
            c.set_x(EAX, 0x9828);
            n += 3 + 5 + 8;
            Some(n + write_event(c, bus))
        }
        // 132b: once per station (flag bit 1), when a commodity is flagged: string 9828h with
        // DS:07c4 = FFFFFFFFh.
        0x132b => {
            let mut n = call_random32(c, bus, 0x132f);
            let edx = c.r[EDX];
            w32(c, bus, 0x07c0, edx);
            let (m, found) = scan_flagged(c, bus);
            n += 1 + m;
            if !found {
                return ret(c, bus, n + 1);
            }
            n += 2;
            if bts_flag(c, bus, 1) {
                return ret(c, bus, n);
            }
            w32(c, bus, 0x07c4, 0xffff_ffff);
            w32(c, bus, 0x07e4, edx);
            c.set_x(EAX, 0x9828);
            Some(n + 4 + write_event(c, bus))
        }
        // 139f: once per station, by allegiance: 2 → 9826h (flag bit 2), 1 → 9827h (bit 3).
        0x139f => {
            let al = r16(c, bus, 0x2243);
            let mut n = 2;
            let (bit, code) = if al == 2 {
                (2, 0x9826)
            } else {
                n += 2;
                if al != 1 {
                    return ret(c, bus, n);
                }
                (3, 0x9827)
            };
            n += 2;
            if bts_flag(c, bus, bit) {
                return ret(c, bus, n);
            }
            c.set_x(EAX, code);
            Some(n + 2 + write_event(c, bus))
        }
        // 10b2: take the newest of the four dwords DS:460f, 460b, 4607, 4603 (clearing it) into
        // DS:07e4, then a passenger or delivery item (*inferred*) to a neighbour.
        0x10b2 => {
            w32(c, bus, 0x07cc, 2);
            let mut n = 1;
            let mut got = false;
            for (k, off) in [0x460fu16, 0x460b, 0x4607, 0x4603].iter().enumerate() {
                let v = r32(c, bus, *off);
                c.r[EAX] = v;
                w32(c, bus, 0x07e4, v);
                w32(c, bus, *off, 0);
                n += 5;
                if v != 0 {
                    got = true;
                    if k == 3 {
                        w16(c, bus, 0x07cc, 1);
                        n += 1;
                    }
                    break;
                }
            }
            if !got {
                return ret(c, bus, n);
            }
            Some(n + neighbour_item(c, bus)?)
        }
        // 118e: the same with a random32 draw in DS:07e4 and DS:07cc = 0.
        0x118e => {
            let mut n = call_random32(c, bus, 0x1192);
            w32(c, bus, 0x07e4, c.r[EAX]);
            w16(c, bus, 0x07cc, 0);
            n += 3;
            Some(n + neighbour_item(c, bus)?)
        }
        0x11ac => trip(c, bus),
        // 135e: a commodity with no stock (the random(12)-th such from the top) at twice its
        // price: string 9829h, DS:07c4 = 8E00h + the commodity.
        0x135e => {
            let mut cx: u16 = 0x1f;
            c.set_x(EAX, 0x0c);
            let mut n = 2 + callf(c, bus, crate::rng::random_port, 0x1368);
            let mut ax = c.x(EAX);
            loop {
                let bx = cx << 2;
                c.set_x(EBX, bx);
                n += 4;
                if r16(c, bus, bx.wrapping_add(si)) == 0 {
                    ax = ax.wrapping_sub(1);
                    n += 2;
                    if ax & 0x8000 != 0 {
                        c.set_x(EAX, ax);
                        c.set_x(ECX, cx);
                        let price = r16(c, bus, bx.wrapping_add(si).wrapping_add(2));
                        let v = price << 1;
                        w16(c, bus, 0x07c8, v);
                        c.set_x(EAX, v);
                        n += 4;
                        if (price ^ v) & 0x8000 != 0 {
                            return ret(c, bus, n);
                        }
                        w16(c, bus, 0x07c4, cx.wrapping_add(0x8e00));
                        let v = r32(c, bus, 0x1fd9);
                        c.r[EAX] = v;
                        w32(c, bus, 0x07e4, v);
                        c.set_x(EAX, 0x9829);
                        return Some(n + 7 + write_event(c, bus));
                    }
                }
                cx = cx.wrapping_sub(1);
                n += 2;
                if cx & 0x8000 != 0 {
                    c.set_x(EAX, ax);
                    c.set_x(ECX, cx);
                    return ret(c, bus, n + 1);
                }
            }
        }
        0x13cd => contract(c, bus),
        // 14d0: string 9822h + (a random32 draw >> 16) & 3, the draw in DS:07e4.
        0x14d0 => {
            let mut n = call_random32(c, bus, 0x14d4);
            let v = c.r[EAX];
            w32(c, bus, 0x07e4, v);
            c.r[EAX] = (v >> 16) & 3;
            c.set_x(EAX, ((v >> 16) as u16 & 3).wrapping_add(0x9822));
            n += 5;
            Some(n + write_event(c, bus))
        }
        // 14e4 (reached only through the table): string 982Bh + bit 16 of a random32 draw,
        // DS:07c4 = 99C0h + its low 3 bits.
        0x14e4 => {
            let mut n = call_random32(c, bus, 0x14e8);
            let v = c.r[EAX];
            w32(c, bus, 0x07c0, c.r[EDX]);
            w32(c, bus, 0x07e4, v);
            w16(c, bus, 0x07c4, (v as u16 & 7).wrapping_add(0x99c0));
            c.r[EAX] = (v & 0xffff_0000 | (v & 7)) >> 16;
            c.set_x(EAX, ((v >> 16) as u16 & 1).wrapping_add(0x982b));
            n += 9;
            Some(n + write_event(c, bus))
        }
        _ => None,
    }
}

/// `EL2M07:111b`, *observed*: an item to a neighbouring system (`random` within 1 sector of
/// `DS:208b`, through module 2): string 981Ch (981Dh when `DS:07e6` is negative), `DS:07c0` = the
/// system, reward `DS:07c8` = 40·(`DS:07e4` & 3 + 2). Ends in `RET`.
fn neighbour_item(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let v = r16(c, bus, 0x07cc);
    push16(c, bus, v);
    let here = r32(c, bus, 0x208b);
    c.r[EAX] = here;
    w32(c, bus, 0x07c0, here);
    w16(c, bus, 0x07c4, 1);
    c.set_x(EAX, 1);
    let (di, si) = (c.x(EDI), c.x(ESI));
    push16(c, bus, di);
    push16(c, bus, si);
    for off in [0x07d4, 0x07d8] {
        let v = r16(c, bus, off);
        push16(c, bus, v);
    }
    let mut n = 9 + nearby(c, bus, 0x1149)?;
    for off in [0x07d8, 0x07d4] {
        let v = pop16(c, bus);
        w16(c, bus, off, v);
    }
    let si = pop16(c, bus);
    c.set_x(ESI, si);
    let di = pop16(c, bus);
    c.set_x(EDI, di);
    let v = pop16(c, bus);
    w16(c, bus, 0x07cc, v);
    n += 7;
    if c.x(EAX) == 0 {
        near_ret(c, bus);
        return Some(n + 1);
    }
    let id = r32(c, bus, 0x07bc);
    c.r[EAX] = id;
    w32(c, bus, 0x07c0, id);
    let mut code = 0x981c;
    n += 5;
    if r16(c, bus, 0x07e6) & 0x8000 != 0 {
        code = 0x981d;
        n += 1;
    }
    let p = ((r16(c, bus, 0x07e4) & 3) + 2) as u32 * 0x28;
    w16(c, bus, 0x07c8, p as u16);
    w16(c, bus, 0x07ca, (p >> 16) as u16);
    c.set_x(EDX, (p >> 16) as u16);
    c.set_x(EAX, code);
    n += 10;
    Some(n + write_event(c, bus))
}

/// `EL2M07:11ac`, *observed*: up to four tries for a system within `random(3)` sectors of
/// `DS:208b` (module 2) whose record byte `+4` is at least 3 (module 6). A found one gets string
/// 9819h or 981Ah (bit 12 of a `random32` draw), with a reward from the tables at `EL2M07:03e1`,
/// `0401` and `0421` (by the draw, the sector range and `DS:223b`), and a kind in `DS:07cc`; a
/// draw past 10h in the kind's high bits also stores the draw in `DS:4603`. Ends in `RET`.
fn trip(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    w16(c, bus, 0x07d8, 3);
    let mut n = 1;
    loop {
        let here = r32(c, bus, 0x208b);
        c.r[EAX] = here;
        w32(c, bus, 0x07c0, here);
        c.set_x(EAX, 3);
        n += 3 + callf(c, bus, crate::rng::random_port, 0x11c1);
        let r3 = c.x(EAX);
        push16(c, bus, r3);
        w16(c, bus, 0x07c4, r3);
        c.set_x(EAX, 1);
        let (di, si) = (c.x(EDI), c.x(ESI));
        push16(c, bus, di);
        push16(c, bus, si);
        for off in [0x07d4, 0x07d8] {
            let v = r16(c, bus, off);
            push16(c, bus, v);
        }
        n += 7 + nearby(c, bus, 0x11e1)?;
        let restore = |c: &mut Cpu, bus: &mut dyn Bus| {
            for off in [0x07d8, 0x07d4] {
                let v = pop16(c, bus);
                w16(c, bus, off, v);
            }
            let si = pop16(c, bus);
            c.set_x(ESI, si);
            let di = pop16(c, bus);
            c.set_x(EDI, di);
        };
        restore(c, bus);
        n += 6;
        let mut drop = 2u16;
        if c.x(EAX) != 0 {
            let id = r32(c, bus, 0x07bc);
            push32(c, bus, id);
            c.r[EAX] = id;
            w32(c, bus, 0x07c0, id);
            c.set_l(EAX, 1);
            let (di, si) = (c.x(EDI), c.x(ESI));
            push16(c, bus, di);
            push16(c, bus, si);
            for off in [0x07d4, 0x07d8] {
                let v = r16(c, bus, off);
                push16(c, bus, v);
            }
            n += 8 + system_data(c, bus, 0x121b)?;
            restore(c, bus);
            n += 6;
            if c.l(EAX) >= 3 {
                break;
            }
            drop = 6;
            n += 1; // ADD SP,4
        }
        let sp = c.x(ESP);
        c.set_x(ESP, sp.wrapping_add(drop));
        let cnt = r16(c, bus, 0x07d8).wrapping_sub(1);
        w16(c, bus, 0x07d8, cnt);
        n += 3;
        if cnt & 0x8000 != 0 {
            near_ret(c, bus);
            return Some(n + 1);
        }
    }
    // Found: the stack holds the target id (dword) above the sector range (word).
    n += call_random32(c, bus, 0x1233);
    let (eax, edx) = (c.r[EAX], c.r[EDX]);
    w32(c, bus, 0x07bc, eax);
    w32(c, bus, 0x07e4, eax);
    w32(c, bus, 0x07c0, edx);
    let r3 = bus.read16(crate::math3::lin32(c.ss, c.r[ESP].wrapping_add(4)));
    let base = r3.wrapping_add(3);
    w16(c, bus, 0x07c8, base);
    n += 6;
    let mut bx = (edx as u16) & 0x1c;
    let cs = c.cs;
    let v = bus.read16(lin(cs, 0x03e1 + bx));
    w16(c, bus, 0x07c4, v);
    let p = bus.read16(lin(cs, 0x03e3 + bx)) as u32 * base as u32;
    w16(c, bus, 0x07c8, p as u16);
    n += 9;
    if bx >= 0x1c {
        bx |= 0x2000;
        n += 1;
    }
    let lo = r16(c, bus, 0x07c0);
    let sq = lo as u32 * lo as u32;
    let hi = (sq >> 16) as u16;
    let q = hi as u32 * hi as u32;
    let dx = (((q >> 16) as u16) >> 3) & 0x1e00;
    let kind = bx.wrapping_add(dx);
    w16(c, bus, 0x07cc, kind);
    let bl = dx >> 8;
    c.set_x(EBX, bl);
    let add = bus.read16(lin(cs, 0x0401 + bl));
    let rw = r16(c, bus, 0x07c8).wrapping_add(add);
    w16(c, bus, 0x07c8, rw);
    c.set_x(EAX, add);
    c.set_x(EDX, dx);
    n += 14;
    if bl >= 0x10 {
        n += 2;
        if bus.read8(lin(c.ds, 0x07cd)) & 0x20 == 0 {
            let v = r32(c, bus, 0x07e4);
            c.r[EAX] = v;
            w32(c, bus, 0x4603, v);
            n += 2;
        }
    }
    let bx = ((r16(c, bus, 0x07bc) & 7).wrapping_add(r16(c, bus, 0x223b))).wrapping_shl(1);
    c.set_x(EBX, bx);
    let f = bus.read16(lin(cs, 0x0421u16.wrapping_add(bx)));
    let p = f as u32 * r16(c, bus, 0x07c8) as u32;
    w16(c, bus, 0x07c8, p as u16);
    c.set_x(EDX, (p >> 16) as u16);
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    let code = if r16(c, bus, 0x07e4) & 0x1000 == 0 { 0x9819 } else { 0x981a };
    c.set_x(EAX, code);
    n += 11 + 2;
    Some(n + write_event(c, bus))
}

/// `EL2M07:13cd`, *observed*: up to four tries for a system within `random(3)` sectors of
/// `DS:208b` (module 2) whose record byte `+4` is at least 3 (module 6). A found one gets string
/// 981Eh + (`DS:07be` & 3), a reward 1000·range + `EL2M07:06ab`[k], text `999Dh` +
/// `EL2M07:06ad`[k] (k from a `random32` draw & 3Ch) and a date from `DS:1fc5` in `DS:07cc`.
/// Ends in `RET`.
fn contract(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    w16(c, bus, 0x07d8, 3);
    let mut n = 1;
    loop {
        let here = r32(c, bus, 0x208b);
        c.r[EAX] = here;
        w32(c, bus, 0x07c0, here);
        c.set_x(EAX, 3);
        n += 3 + callf(c, bus, crate::rng::random_port, 0x13e2);
        let r3 = c.x(EAX);
        push16(c, bus, r3);
        let (di, si) = (c.x(EDI), c.x(ESI));
        push16(c, bus, di);
        push16(c, bus, si);
        w16(c, bus, 0x07c4, r3);
        c.set_x(EAX, 1);
        n += 5 + nearby(c, bus, 0x13fa)?;
        let si = pop16(c, bus);
        c.set_x(ESI, si);
        let di = pop16(c, bus);
        c.set_x(EDI, di);
        n += 4;
        let mut drop = 2;
        if c.x(EAX) != 0 {
            let id = r32(c, bus, 0x07bc);
            c.r[EAX] = id;
            push32(c, bus, id);
            w32(c, bus, 0x07c0, id);
            c.set_l(EAX, 1);
            for off in [0x07d0, 0x07d4, 0x07d8] {
                let v = r32(c, bus, off);
                push32(c, bus, v);
            }
            let (di, si) = (c.x(EDI), c.x(ESI));
            push16(c, bus, di);
            push16(c, bus, si);
            n += 9 + system_data(c, bus, 0x1432)?;
            let si = pop16(c, bus);
            c.set_x(ESI, si);
            let di = pop16(c, bus);
            c.set_x(EDI, di);
            for off in [0x07d8, 0x07d4, 0x07d0] {
                let v = pop32(c, bus);
                w32(c, bus, off, v);
            }
            n += 7;
            if c.l(EAX) >= 3 {
                break;
            }
            drop = 6;
            n += 1;
        }
        let sp = c.x(ESP);
        c.set_x(ESP, sp.wrapping_add(drop));
        let cnt = r16(c, bus, 0x07d8).wrapping_sub(1);
        w16(c, bus, 0x07d8, cnt);
        n += 3;
        if cnt & 0x8000 != 0 {
            near_ret(c, bus);
            return Some(n + 1);
        }
    }
    n += call_random32(c, bus, 0x144f);
    w32(c, bus, 0x07e4, c.r[EAX]);
    let k = c.x(EDX) & 0x3c;
    c.set_x(EDX, k);
    w16(c, bus, 0x07cc, k);
    n += 3 + call_random32(c, bus, 0x145e);
    w32(c, bus, 0x07bc, c.r[EAX]);
    let date = r32(c, bus, 0x1fc5).wrapping_sub(0x0011_d5a0);
    let dl = c.x(EDX) & 0x3f;
    c.set_x(EDX, dl);
    let d = (date as u16).wrapping_add(7).wrapping_add(dl);
    c.r[EAX] = date & 0xffff_0000 | d as u32;
    w16(c, bus, 0x07d0, d);
    n += 7;
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    let r3 = pop16(c, bus);
    let p = r3 as u32 * 0x3e8;
    c.set_x(EDX, (p >> 16) as u16);
    w16(c, bus, 0x07c8, p as u16);
    n += 5;
    let bx = r16(c, bus, 0x07cc);
    c.set_x(EBX, bx);
    let cs = c.cs;
    let add = bus.read16(lin(cs, 0x06abu16.wrapping_add(bx)));
    let rw = (p as u16).wrapping_add(add);
    w16(c, bus, 0x07c8, rw);
    let t = bus.read16(lin(cs, 0x06adu16.wrapping_add(bx))).wrapping_add(0x999d);
    w16(c, bus, 0x07c4, t);
    let v = r16(c, bus, 0x07d0);
    w16(c, bus, 0x07cc, v);
    let code = (r16(c, bus, 0x07be) & 3).wrapping_add(0x981e);
    c.set_x(EAX, code);
    n += 6 + 6;
    Some(n + write_event(c, bus))
}

/// `EL2M07:1506`, *observed*: when the station has fewer than 18 events (`[SI+19Eh]`, signed),
/// count one and write 16 bytes at `DS:DI`: `AX`, the words `DS:07c4`, `07c8`, `07cc`, the dwords
/// `DS:07c0` and `07e4`; `DI` moves on by 16. Ends in `RET`.
fn write_event(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let si = c.x(ESI);
    let count = r16(c, bus, si.wrapping_add(0x19e));
    if (count as i16) >= 0x12 {
        near_ret(c, bus);
        return 3;
    }
    w16(c, bus, si.wrapping_add(0x19e), count.wrapping_add(1));
    let di = c.x(EDI);
    w16(c, bus, di, c.x(EAX));
    for (k, off) in [0x07c4u16, 0x07c8, 0x07cc].iter().enumerate() {
        let v = r16(c, bus, *off);
        w16(c, bus, di.wrapping_add(2 + 2 * k as u16), v);
        c.set_x(EAX, v);
    }
    let a = r32(c, bus, 0x07c0);
    w32(c, bus, di.wrapping_add(8), a);
    let e = r32(c, bus, 0x07e4);
    w32(c, bus, di.wrapping_add(0x0c), e);
    c.r[EAX] = e;
    c.set_x(EDI, di.wrapping_add(0x10));
    near_ret(c, bus);
    16
}

/// Port of `EL2M07:0949`, *observed*: build the markets, events and lists of every station in
/// system `DS:07c0` (kept), then the bulletin board. A near routine, called by module 7's handler
/// `+0c` on arrival.
/// 1. System data `AL` = 0 (module 6): `DS:2241`, `223d`, `4613`, `223f` = `DS:07bc`, `07c8`,
///    `07cc`, `07d0`; `DS:2243` = `DS:07d8` >> 6 (the allegiance).
/// 2. `AL` = 1: `DS:2239` = `AX`, `DS:223b` = `BX`, and their doubles in `DS:07bc`/`07c0` (the
///    factor indices of [`price_port`] and [`stock_port`]); the 31 commodity bytes of the record
///    go to `DS:2219`.
/// 3. For each of the `DS:2247` stations (1DAh-byte records from `DS:224d`): the 31 prices and
///    stocks (`DS:07c8` = 2·(byte & 7), coefficients from `EL2M07:02a9`); 4·`DS:2239` + 1 calls of
///    [`event_port`]; `DS:2249` = the station object's `ES:[obj+76h]` with its halves swapped,
///    and flag bit 4 unless `ES:[obj+118h]` bit 5; then the lists of kinds 9, 0 and 1
///    ([`station_list_port`]).
/// 4. `DS:43a7` = 0; `AL` = 2: `DS:43a5` = `DS:07d4`, `DS:43a1` = 8·`DS:07c4` + 8, `DS:43a3` =
///    `DS:07d8`; `DS:43a1` + 1 calls of [`advert_port`].
pub fn builder_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    builder(c, bus).unwrap_or(DECLINE)
}

fn builder(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    if r16(c, bus, 0x00b4) != 0x085a {
        return None;
    }
    let id = r32(c, bus, 0x07c0);
    push32(c, bus, id);
    c.set_x(EAX, 0);
    let mut n = 2 + system_data(c, bus, 0x095f)?;
    for (from, to) in [(0x07bc, 0x2241), (0x07c8, 0x223d), (0x07cc, 0x4613), (0x07d0, 0x223f)] {
        let v = r16(c, bus, from);
        c.set_x(EAX, v);
        w16(c, bus, to, v);
    }
    let a = r16(c, bus, 0x07d8) >> 6;
    c.set_x(EAX, a);
    w16(c, bus, 0x2243, a);
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    push32(c, bus, v);
    c.set_l(EAX, 1);
    n += 11 + 3 + system_data(c, bus, 0x099f)?;
    let ax = c.x(EAX);
    w16(c, bus, 0x2239, ax);
    c.set_x(EAX, ax << 1);
    w16(c, bus, 0x07bc, ax << 1);
    let bx = c.x(EBX);
    w16(c, bus, 0x223b, bx);
    c.set_x(EBX, bx << 1);
    w16(c, bus, 0x07c0, bx << 1);
    let es = c.es;
    let mut di = c.x(EDI);
    for k in 0..0x1f {
        let b = bus.read8(lin(es, di));
        c.set_l(EAX, b);
        bus.write8(lin(c.ds, 0x2219 + k), b);
        di = di.wrapping_add(1);
    }
    c.set_x(EDI, di);
    c.set_x(ESI, 0x2219 + 0x1f);
    c.set_x(ECX, 0);
    n += 8 + 5 * 0x1f;
    let stations = r16(c, bus, 0x2247);
    c.set_x(ECX, stations);
    n += 3;
    if stations != 0 {
        w16(c, bus, 0x07d8, stations);
        let mut si: u16 = 0x224d;
        c.set_x(ESI, si);
        n += 2;
        loop {
            n += station(c, bus, si)?;
            si = c.x(ESI).wrapping_add(0x1da);
            c.set_x(ESI, si);
            let left = r16(c, bus, 0x07d8).wrapping_sub(1);
            w16(c, bus, 0x07d8, left);
            n += 5;
            if left == 0 {
                break;
            }
        }
    }
    w32(c, bus, 0x43a7, 0);
    let v = pop32(c, bus);
    w32(c, bus, 0x07c0, v);
    c.set_l(EAX, 2);
    n += 3 + system_data(c, bus, 0x0ada)?;
    let v = r16(c, bus, 0x07d4);
    w16(c, bus, 0x43a5, v);
    let ads = (r16(c, bus, 0x07c4) << 3).wrapping_add(8);
    w16(c, bus, 0x43a1, ads);
    w16(c, bus, 0x07d4, ads);
    let v = r16(c, bus, 0x07d8);
    c.set_x(EAX, v);
    w16(c, bus, 0x43a3, v);
    n += 9;
    loop {
        push16(c, bus, 0x0afc);
        n += 1 + advert(c, bus)?;
        let left = r16(c, bus, 0x07d4).wrapping_sub(1);
        w16(c, bus, 0x07d4, left);
        n += 2;
        if left & 0x8000 != 0 {
            break;
        }
    }
    near_ret(c, bus);
    Some(n + 1)
}

/// One station of [`builder`] (`EL2M07:09d5` to `0aa7`), with `SI` at its record.
fn station(c: &mut Cpu, bus: &mut dyn Bus, si: u16) -> Option<u64> {
    c.set_x(EDI, si);
    c.set_x(EBP, 0x02a9);
    c.set_x(EBX, 0x2219);
    w16(c, bus, 0x07d4, 0x1e);
    let mut n = 4;
    loop {
        let bx = c.x(EBX);
        let cl = bus.read8(lin(c.ds, bx));
        c.set_l(ECX, cl);
        c.set_x(EBX, bx.wrapping_add(1));
        let dx = ((cl & 7) as u16) << 1;
        c.set_x(EDX, dx);
        w16(c, bus, 0x07c8, dx);
        push16(c, bus, bx.wrapping_add(1));
        push16(c, bus, 0x09f5);
        n += 8 + price_port(c, bus);
        let di = c.x(EDI);
        w16(c, bus, di.wrapping_add(2), c.x(EDX));
        push16(c, bus, 0x09fb);
        n += 2 + stock_port(c, bus);
        w16(c, bus, di, c.x(EDX));
        let bx = pop16(c, bus);
        c.set_x(EBX, bx);
        c.set_x(EDI, di.wrapping_add(4));
        let bp = c.x(EBP);
        c.set_x(EBP, bp.wrapping_add(8));
        let left = r16(c, bus, 0x07d4).wrapping_sub(1);
        w16(c, bus, 0x07d4, left);
        n += 6;
        if left & 0x8000 != 0 {
            break;
        }
    }
    for off in [0x07bc, 0x07c0] {
        let v = r16(c, bus, off);
        push16(c, bus, v);
    }
    c.set_x(EDI, si.wrapping_add(0x7e));
    w16(c, bus, si.wrapping_add(0x19e), 0);
    bus.write8(lin(c.ds, si.wrapping_add(0x7d)), 0);
    let cx = r16(c, bus, 0x2239).wrapping_shl(2);
    c.set_x(ECX, cx);
    w16(c, bus, 0x07d4, cx);
    n += 8;
    loop {
        push16(c, bus, 0x0a2d);
        n += 1 + event(c, bus)?;
        let left = r16(c, bus, 0x07d4).wrapping_sub(1);
        w16(c, bus, 0x07d4, left);
        n += 2;
        if left & 0x8000 != 0 {
            break;
        }
    }
    // PUSH SI; MOVZX BX,[SI+7Ch]; [07ec] = [56fb]; CALLF [00b4] (SHL BX,1; LES SI,[07ec];
    // ADD SI,[BX+06d6]; RETF); DS:2249 = ES:[SI+76h] with its halves swapped.
    let si = c.x(ESI);
    let bx = (bus.read8(lin(c.ds, si.wrapping_add(0x7c))) as u16) << 1;
    c.set_x(EBX, bx);
    let p = r32(c, bus, 0x56fb);
    w32(c, bus, 0x07ec, p);
    c.es = r16(c, bus, 0x07ee);
    let obj = r16(c, bus, 0x07ec).wrapping_add(r16(c, bus, 0x06d6u16.wrapping_add(bx)));
    let v = bus.read32(lin(c.es, obj.wrapping_add(0x76))).rotate_right(16);
    c.r[EAX] = v;
    w32(c, bus, 0x2249, v);
    let docked = bus.read8(lin(c.es, obj.wrapping_add(0x118))) & 0x20 != 0;
    n += 15;
    if !docked {
        let f = bus.read8(lin(c.ds, si.wrapping_add(0x7d)));
        bus.write8(lin(c.ds, si.wrapping_add(0x7d)), f | 0x10);
        n += 1;
    }
    c.set_x(EDI, si.wrapping_add(0x1a0));
    w16(c, bus, si.wrapping_add(0x1d8), 0);
    let mut cx = r16(c, bus, 0x2247).wrapping_sub(1);
    c.set_x(ECX, cx);
    n += 5;
    let mut lists: Vec<(u16, u16)> = Vec::new();
    if cx & 0x8000 == 0 {
        cx >>= 1;
        n += 1;
        lists.push((9, cx));
    }
    let a = r16(c, bus, 0x223d).wrapping_shl(1);
    let b = r16(c, bus, 0x223f).wrapping_shl(1);
    for (kind, start, pre) in lists.into_iter().map(|(k, s)| (k, s, 0u64)).chain([(0u16, a, 2u64), (1, b, 2)]) {
        n += pre;
        let mut cx = start;
        loop {
            push16(c, bus, cx);
            w16(c, bus, 0x07d0, kind);
            push16(c, bus, 0x0a7b);
            n += 3 + station_list_port(c, bus);
            cx = pop16(c, bus).wrapping_sub(1);
            c.set_x(ECX, cx);
            n += 3;
            if cx & 0x8000 != 0 {
                break;
            }
        }
    }
    let v = pop16(c, bus);
    w16(c, bus, 0x07c0, v);
    let v = pop16(c, bus);
    w16(c, bus, 0x07bc, v);
    Some(n)
}

/// Port of `EL2M07:0818` with its callback `0831`, *observed*: count the stations. `DS:2247` = 0,
/// then the main program's object iterator (`CALLF [0000]` = `1000:0cff`) walks the 73h slot
/// bytes of the object block `[DS:56fb]` from the top. Each non-zero slot without bit 5 gets its
/// object from `1000:084f` (`[56f7]` + the word at `DS:06d6` + 2·slot) and the callback: an object
/// with bit 4 of its byte `+118h` is a station; its byte `+5Dh` goes to `+7Ch` of station record
/// `DS:2247` (from `DS:224d`, 1DAh bytes each), and the count goes up while it is below 18. The
/// iterator leaves `[DS:07ec]` and `[DS:56f7]` = the block and `DS:07c0` = the last slot's word.
/// A near routine. Declines when the slots `DS:0000` and `DS:00b0` are not those routines.
pub fn station_count_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    station_count(c, bus).unwrap_or(DECLINE)
}

fn station_count(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    if r16(c, bus, 0x0000) != 0x0cff || r16(c, bus, 0x00b0) != 0x084f {
        return None;
    }
    w16(c, bus, 0x2247, 0);
    let si = r16(c, bus, 0x56fb);
    let es = r16(c, bus, 0x56fd);
    c.set_x(ESI, si);
    c.es = es;
    w16(c, bus, 0x07e8, 0x0831);
    w16(c, bus, 0x07ea, c.cs);
    // The iterator, 1000:0cff.
    w16(c, bus, 0x07ec, si);
    w16(c, bus, 0x07ee, es);
    w16(c, bus, 0x56f7, si);
    w16(c, bus, 0x56f9, es);
    let mut n: u64 = 5 + 5;
    let mut bx: u16 = 0x72;
    loop {
        let b = bus.read8(lin(es, bx.wrapping_add(si)));
        n += 2;
        if b != 0 {
            n += 2;
            if b & 0x20 == 0 {
                let w = bus.read16(lin(es, bx.wrapping_add(si)));
                w16(c, bus, 0x07c0, w);
                // CALLF [00b0] = 1000:084f: SHL BX,1; LES SI,[56f7]; ADD SI,[BX+06d6]; RETF.
                let obj_es = r16(c, bus, 0x56f9);
                let obj = r16(c, bus, 0x56f7).wrapping_add(r16(c, bus, 0x06d6u16.wrapping_add(bx << 1)));
                c.set_x(EBX, bx << 1);
                c.set_x(ESI, obj);
                c.es = obj_es;
                n += 5 + 5 + 1 + station_callback(c, bus) + 3;
                c.set_x(ESI, si);
                c.es = es;
            }
        }
        bx = bx.wrapping_sub(1);
        c.set_x(EBX, bx);
        n += 2;
        if bx & 0x8000 != 0 {
            break;
        }
    }
    near_ret(c, bus);
    Some(n + 1 + 1)
}

/// `EL2M07:0831`, the station callback of [`station_count_port`], through its `RETF`.
fn station_callback(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (es, si) = (c.es, c.x(ESI));
    if bus.read8(lin(es, si.wrapping_add(0x118))) & 0x10 == 0 {
        return 3;
    }
    let count = r16(c, bus, 0x2247);
    let mut n = 5;
    if count < 0x12 {
        w16(c, bus, 0x2247, count + 1);
        n += 1;
    }
    let p = count as u32 * 0x1da;
    c.set_x(EDX, (p >> 16) as u16);
    c.set_x(EBX, p as u16);
    let b = bus.read8(lin(es, si.wrapping_add(0x5d)));
    c.set_x(EAX, (p as u16 & 0xff00) | b as u16);
    bus.write8(lin(c.ds, (p as u16).wrapping_add(0x22c9)), b);
    n + 6
}

/// `(new + 3·old) >> 2` in 32 bits, as `EL2M07:0ed3`–`0ef3` computes it with `CWD`, `ADC` and
/// two `SAR`/`RCR` pairs: (`AX`, `DX`, `BX` = 2·old).
fn blend(new: u16, old: u16) -> (u16, u16, u16) {
    let sum = new as i16 as i32 + 3 * (old as i16 as i32);
    let r = sum >> 2;
    (r as u16, (r >> 16) as u16, old.wrapping_shl(1))
}

/// Port of `EL2M07:0e1a`, *observed*: the daily update of the current system's markets
/// (module 7's handler `+0c` with `AX` = 0Fh).
/// 1. The first advert whose deadline is before the date + 4 (`DS:1fc5`) is removed; the
///    adverts after it move up one slot.
/// 2. Three times: when `random(25)` is at most `DS:43a1`, a new advert ([`advert_port`]).
/// 3. Each station: every price and stock moves to (new + 3·old)/4 with a new draw
///    ([`price_port`], [`stock_port`]); when `random(7)` < `DS:2239`, a random event is cleared
///    (with its station flag) and replaced by a new one ([`event_port`]); `random(14)` removes a
///    list entry; and `random(9)` < `DS:223d`, `random(9)` < `DS:223f`, `random(27)` < `DS:2247`
///    each add one to the lists of kinds 0, 1 and 9 ([`station_list_port`]).
///
/// A near routine.
pub fn daily_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    daily(c, bus).unwrap_or(DECLINE)
}

fn daily(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let soon = r32(c, bus, 0x1fc5).wrapping_add(4);
    c.r[EAX] = soon;
    let mut si: u16 = 0x438f;
    let mut n = 3;
    loop {
        si = si.wrapping_add(0x18);
        n += 3;
        if r32(c, bus, si) == 0 {
            break;
        }
        n += 2;
        if soon < r32(c, bus, si.wrapping_add(8)) {
            continue;
        }
        si = si.wrapping_add(0x18);
        n += 1;
        loop {
            for o in [4u16, 8, 0x0c, 0x10, 0x14] {
                let v = r32(c, bus, si.wrapping_add(o));
                w32(c, bus, si.wrapping_add(o).wrapping_sub(0x18), v);
            }
            let v = r32(c, bus, si);
            w32(c, bus, si.wrapping_sub(0x18), v);
            c.r[EAX] = v;
            si = si.wrapping_add(0x18);
            n += 15;
            if v == 0 {
                break;
            }
        }
        break;
    }
    c.set_x(ESI, si);
    w16(c, bus, 0x07d4, 2);
    n += 1;
    loop {
        c.set_x(EAX, 0x19);
        n += 1 + callf(c, bus, crate::rng::random_port, 0x0e7b) + 2;
        let make = c.x(EAX) <= r16(c, bus, 0x43a1);
        if make {
            push16(c, bus, 0x0e8c);
            n += 1 + advert(c, bus)?;
        }
        let left = r16(c, bus, 0x07d4).wrapping_sub(1);
        w16(c, bus, 0x07d4, left);
        n += 2;
        if left & 0x8000 != 0 {
            if !make {
                n += 1; // JMP 0e92
            }
            break;
        }
    }
    let v = r16(c, bus, 0x2239) << 1;
    w16(c, bus, 0x07bc, v);
    let v = r16(c, bus, 0x223b) << 1;
    c.set_x(EAX, v);
    w16(c, bus, 0x07c0, v);
    let stations = r16(c, bus, 0x2247);
    c.set_x(ECX, stations);
    n += 9;
    if stations != 0 {
        w16(c, bus, 0x07d8, stations);
        let mut si: u16 = 0x224d;
        c.set_x(ESI, si);
        n += 2;
        loop {
            n += daily_station(c, bus, si)?;
            si = c.x(ESI).wrapping_add(0x1da);
            c.set_x(ESI, si);
            let left = r16(c, bus, 0x07d8).wrapping_sub(1);
            w16(c, bus, 0x07d8, left);
            n += 5;
            if left == 0 {
                break;
            }
        }
    }
    near_ret(c, bus);
    Some(n + 1)
}

/// One station of [`daily`] (`EL2M07:0eb3` to `1005`), with `SI` at its record.
fn daily_station(c: &mut Cpu, bus: &mut dyn Bus, si: u16) -> Option<u64> {
    c.set_x(EDI, si);
    c.set_x(EBP, 0x02a9);
    c.set_x(EBX, 0x2219);
    w16(c, bus, 0x07d4, 0x1e);
    let mut n = 4;
    loop {
        let bx = c.x(EBX);
        let cl = bus.read8(lin(c.ds, bx));
        c.set_l(ECX, cl);
        c.set_x(EBX, bx.wrapping_add(1));
        let dx = ((cl & 7) as u16) << 1;
        c.set_x(EDX, dx);
        w16(c, bus, 0x07c8, dx);
        push16(c, bus, bx.wrapping_add(1));
        push16(c, bus, 0x0ed3);
        n += 8 + price_port(c, bus);
        let di = c.x(EDI);
        let (ax, dx, b) = blend(c.x(EDX), r16(c, bus, di.wrapping_add(2)));
        c.set_x(EAX, ax);
        c.set_x(EDX, dx);
        c.set_x(EBX, b);
        w16(c, bus, di.wrapping_add(2), ax);
        push16(c, bus, 0x0efc);
        n += 22 + stock_port(c, bus);
        let (ax, dx, _) = blend(c.x(EDX), r16(c, bus, di));
        c.set_x(EAX, ax);
        c.set_x(EDX, dx);
        w16(c, bus, di, ax);
        let bx = pop16(c, bus);
        c.set_x(EBX, bx);
        c.set_x(EDI, di.wrapping_add(4));
        let bp = c.x(EBP);
        c.set_x(EBP, bp.wrapping_add(8));
        let left = r16(c, bus, 0x07d4).wrapping_sub(1);
        w16(c, bus, 0x07d4, left);
        n += 21 + 5;
        if left & 0x8000 != 0 {
            break;
        }
    }
    for off in [0x07bc, 0x07c0] {
        let v = r16(c, bus, off);
        push16(c, bus, v);
    }
    c.set_x(EAX, 7);
    n += 3 + callf(c, bus, crate::rng::random_port, 0x0f3c) + 2;
    if (c.x(EAX) as i16) < (r16(c, bus, 0x2239) as i16) {
        let count = r16(c, bus, si.wrapping_add(0x19e));
        c.set_x(EAX, count);
        n += 1 + callf(c, bus, crate::rng::random_port, 0x0f4a);
        let bx = c.x(EAX) << 4;
        c.set_x(EAX, bx);
        c.set_x(EBX, bx);
        let di = bx.wrapping_add(si).wrapping_add(0x7e);
        c.set_x(EDI, di);
        let ax = r16(c, bus, di);
        c.set_x(EAX, ax);
        n += 6;
        let flag = |c: &Cpu, bus: &mut dyn Bus, mask: u8| {
            let a = lin(c.ds, si.wrapping_add(0x7d));
            let v = bus.read8(a);
            bus.write8(a, v & mask);
        };
        if ax == 0x9828 {
            n += 2 + 1;
            if r16(c, bus, di.wrapping_add(2)) & 0x8000 == 0 {
                flag(c, bus, 0xfe);
                n += 1;
            } else {
                flag(c, bus, 0xfd);
            }
        } else {
            n += 2;
            if ax == 0x9826 {
                flag(c, bus, 0xfb);
                n += 2;
            }
        }
        n += 2;
        if ax == 0x9827 {
            flag(c, bus, 0xf7);
            n += 1;
        }
        w16(c, bus, di, 0);
        let count = r16(c, bus, si.wrapping_add(0x19e));
        push16(c, bus, count);
        w16(c, bus, si.wrapping_add(0x19e), count.wrapping_sub(1));
        push16(c, bus, 0x0f8c);
        n += 4 + event(c, bus)?;
        let v = pop16(c, bus);
        w16(c, bus, si.wrapping_add(0x19e), v);
        n += 1;
    }
    let di = si.wrapping_add(0x1a0);
    c.set_x(EDI, di);
    c.set_x(EAX, 0x0e);
    n += 2 + callf(c, bus, crate::rng::random_port, 0x0f9b);
    let ax = c.x(EAX);
    let count = r16(c, bus, si.wrapping_add(0x1d8));
    let cx = count.wrapping_sub(ax);
    c.set_x(ECX, cx);
    n += 3;
    if (cx as i16) > 0 {
        let mut bx = di.wrapping_add(ax << 2);
        c.set_x(EAX, ax << 2);
        n += 3;
        for _ in 0..cx {
            let v = r32(c, bus, bx.wrapping_add(4));
            w32(c, bus, bx, v);
            c.r[EAX] = v;
            bx = bx.wrapping_add(4);
            n += 4;
        }
        c.set_x(EBX, bx);
        c.set_x(ECX, 0);
        w16(c, bus, si.wrapping_add(0x1d8), count.wrapping_sub(1));
        n += 1;
    }
    let count = r16(c, bus, si.wrapping_add(0x1d8));
    c.set_x(EAX, count << 2);
    c.set_x(EDI, di.wrapping_add(count << 2));
    n += 3;
    for (range, limit, kind, ret_ip) in [(9u16, 0x223du16, 0u16, 0x0fca), (9, 0x223f, 1, 0x0fe0), (0x1b, 0x2247, 9, 0x0ff6)] {
        c.set_x(EAX, range);
        n += 1 + callf(c, bus, crate::rng::random_port, ret_ip) + 2;
        if (c.x(EAX) as i16) < (r16(c, bus, limit) as i16) {
            w16(c, bus, 0x07d0, kind);
            push16(c, bus, ret_ip.wrapping_add(0x0f));
            n += 2 + station_list_port(c, bus);
        }
    }
    let v = pop16(c, bus);
    w16(c, bus, 0x07c0, v);
    let v = pop16(c, bus);
    w16(c, bus, 0x07bc, v);
    Some(n)
}

/// Port of `EL2M07:1551` (module 7, handler `+0c`), *observed*. `AX` = 2 (arrival): count the
/// stations ([`station_count_port`]), shift the four dwords `DS:4603`–`460f` up one (0 in
/// `4603`), and build the markets of system `DS:208b` ([`builder_port`]). `AX` = 0Fh: the daily
/// update ([`daily_port`]) unless byte `DS:2075` = 8. `AX`, `BX`, `CX` and `DX` are kept on
/// those paths. Other values do nothing. Ends in `RETF`.
pub fn handler_0c_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    handler_0c(c, bus).unwrap_or(DECLINE)
}

fn handler_0c(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let ax = c.x(EAX);
    let saved = [c.x(EAX), c.x(EBX), c.x(ECX), c.x(EDX)];
    let restore = |c: &mut Cpu| {
        c.set_x(EAX, saved[0]);
        c.set_x(EBX, saved[1]);
        c.set_x(ECX, saved[2]);
        c.set_x(EDX, saved[3]);
    };
    let mut n = 2;
    if ax == 2 {
        for v in saved {
            push16(c, bus, v);
        }
        push16(c, bus, 0x1563);
        n += 4 + 1 + station_count(c, bus)?;
        let mut eax = 0;
        for off in [0x4603u16, 0x4607, 0x460b] {
            let v = r32(c, bus, off);
            w32(c, bus, off, eax);
            eax = v;
        }
        w32(c, bus, 0x460f, eax);
        let here = r32(c, bus, 0x208b);
        c.r[EAX] = here;
        w32(c, bus, 0x07c0, here);
        push16(c, bus, 0x1584);
        n += 5 + 2 + 1 + builder(c, bus)?;
        for _ in 0..4 {
            pop16(c, bus);
        }
        restore(c);
        c.retf(bus);
        return Some(n + 5);
    }
    n += 2;
    if ax != 0x0f {
        c.retf(bus);
        return Some(n + 1);
    }
    n += 2;
    if bus.read8(lin(c.ds, 0x2075)) == 8 {
        c.retf(bus);
        return Some(n + 1);
    }
    for v in saved {
        push16(c, bus, v);
    }
    let here = r32(c, bus, 0x208b);
    c.r[EAX] = here;
    w32(c, bus, 0x07c0, here);
    push16(c, bus, 0x159f);
    n += 4 + 2 + 1 + daily(c, bus)?;
    for _ in 0..4 {
        pop16(c, bus);
    }
    restore(c);
    c.retf(bus);
    Some(n + 5)
}

/// Every ported routine of `EL2M07` (near routines: slot 0).
pub const PORTS_M07: &[(u16, u16, &str, Port)] = &[(0x0855, 0, "price", price_port), (0x08c0, 0, "stock", stock_port)];

/// The bulletin board's routines in `EL2M07`, which need modules 2 and 6 resident (near routines).
pub const BOARD_M07: &[(u16, u16, &str, Port)] = &[(0x0b03, 0, "advert", advert_port), (0x101a, 0, "station_list", station_list_port), (0x0923, 0, "event", event_port), (0x0949, 0, "builder", builder_port), (0x0818, 0, "station_count", station_count_port), (0x0e1a, 0, "daily", daily_port), (0x1551, 0, "handler_0c", handler_0c_port)];
