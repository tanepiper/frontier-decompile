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

/// Every ported routine of `EL2M07` (near routines: slot 0).
pub const PORTS_M07: &[(u16, u16, &str, Port)] = &[(0x0855, 0, "price", price_port), (0x08c0, 0, "stock", stock_port)];

/// The bulletin board's routines in `EL2M07`, which need modules 2 and 6 resident (near routines).
pub const BOARD_M07: &[(u16, u16, &str, Port)] = &[(0x0b03, 0, "advert", advert_port), (0x101a, 0, "station_list", station_list_port)];
