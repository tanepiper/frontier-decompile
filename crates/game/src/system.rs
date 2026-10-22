//! Star-system generation in `EL2M00` (docs/SYSTEM.md): the system seed, body records and their
//! classification, names and the moons and starports added to them.
//!
//! Body records are 40h bytes in `ES` (`ES:SI`); `DS:204d` counts the bodies made (at most 18) and
//! `DS:2049` is the current body's parent value the classifiers branch on. The ports are register-
//! and memory-exact and return the original's instruction counts; tables are read from `EL2M00`
//! in memory.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use crate::names::name_port;

/// The system seed in the main data segment: `s0` = `DS:2021`, `s1` = `DS:2025`.
pub const SYS_S0: u16 = 0x2021;
pub const SYS_S1: u16 = 0x2025;

/// One step of the system seed (`EL2M00:4df5`): `s1 += s0; s0 = rol(s0, 3) + s1;
/// s1 = rol(s1, 5); s0 += s1`. Returns the new (`s0`, `s1`), which are also `EAX` and `EBX`.
pub fn sys_step(s0: u32, s1: u32) -> (u32, u32) {
    let b = s1.wrapping_add(s0);
    let a = s0.rotate_left(3).wrapping_add(b);
    let b = b.rotate_left(5);
    (a.wrapping_add(b), b)
}

fn near_ret(c: &mut Cpu, bus: &mut dyn Bus) {
    let sp = c.x(ESP);
    c.ip = bus.read16(lin(c.ss, sp));
    c.set_x(ESP, sp.wrapping_add(2));
}

fn push16(c: &mut Cpu, bus: &mut dyn Bus, v: u16) {
    let sp = c.x(ESP).wrapping_sub(2);
    bus.write16(lin(c.ss, sp), v);
    c.set_x(ESP, sp);
}

/// `EL2M00:4df5`, *observed*: [`sys_step`] on `DS:2021`/`2025`; `EAX` = new `s0`, `EBX` = new
/// `s1`. A near routine of 10 instructions.
fn sys_step_inline(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (p0, p1) = (lin(c.ds, SYS_S0), lin(c.ds, SYS_S1));
    let (a, b) = sys_step(bus.read32(p0), bus.read32(p1));
    bus.write32(p1, b);
    bus.write32(p0, a);
    c.r[EAX] = a;
    c.r[EBX] = b;
    10
}

/// Port of `EL2M00:4df5` (as entered by a near `CALL`).
pub fn sys_step_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let n = sys_step_inline(c, bus);
    near_ret(c, bus);
    n
}

/// Port of `EL2M00:4e29`, *observed*: `BH` ≥ 40h gives `AX` = FFFFh; otherwise `AX` =
/// `EL2M00:3c39[(BX >> 5) & 1FEh]` and `BX` is left as that index. 4 or 6 instructions.
pub fn lookup_3c39_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let bx = c.x(EBX);
    let n = if bx >> 8 >= 0x40 {
        c.set_x(EAX, 0xffff);
        4
    } else {
        let i = (bx >> 5) & 0x1fe;
        c.set_x(EBX, i);
        let v = bus.read16(lin(c.cs, 0x3c39u16.wrapping_add(i)));
        c.set_x(EAX, v);
        6
    };
    near_ret(c, bus);
    n
}

/// `EL2M00:4db8` without its `RET`, *observed*: name the body at `ES:SI`. Module 9's handler
/// `+20` (the name generator, [`name_port`]) makes a name of kind `AX` from the body's seed
/// (`ES:[SI+4]`/`[SI+6]` as `BX`/`CX`) at `SS:FC08h`; bit 7 of `ES:[SI+3Bh]` is set; the name is
/// copied to `ES:[SI+22h]` with `STOSB`. `BX`, `CX`, `DX`, `SI` and `ES` are preserved. Declines
/// when `EL2M09` is not resident (the original would load it) or the direction flag is set.
fn body_name(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    if c.flags & 0x0400 != 0 {
        return None;
    }
    let (es, si, ds, ss) = (c.es, c.x(ESI), c.ds, c.ss);
    // Module slot 9: DS:0396 + 48h, its handler +20.
    let hdr_off = bus.read16(lin(ds, 0x0396 + 0x48));
    let hdr_seg = bus.read16(lin(ds, 0x0396 + 0x4a));
    let off = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x20)));
    let seg = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x22)));
    if off != 0x0f92 {
        return None;
    }
    let lock = bus.read8(lin(ds, 0x059f));
    bus.write8(lin(ds, 0x059f), lock);
    let flag = bus.read8(lin(es, si.wrapping_add(0x3b)));
    bus.write8(lin(es, si.wrapping_add(0x3b)), flag | 0x80);
    // PUSH BX, CX, DX, SI, ES; then the call with BX/CX = the body's seed, DI = FC08h, SI = 48h,
    // ES:BP = the module header.
    let mut sub = *c;
    for v in [c.x(EBX), c.x(ECX), c.x(EDX), si, es] {
        push16(&mut sub, bus, v);
    }
    sub.set_x(EBX, bus.read16(lin(es, si.wrapping_add(4))));
    sub.set_x(ECX, bus.read16(lin(es, si.wrapping_add(6))));
    sub.set_x(EDI, 0xfc08);
    sub.set_x(ESI, 0x48);
    sub.set_x(EBP, hdr_off);
    sub.es = hdr_seg;
    let (ret_cs, ret_ip) = (c.cs, 0x4ddc);
    push16(&mut sub, bus, ret_cs);
    push16(&mut sub, bus, ret_ip);
    sub.cs = seg;
    sub.ip = off;
    let name_steps = name_port(&mut sub, bus);
    if name_steps == DECLINE {
        return None;
    }
    // DEC [59f]; POP ES, SI, DX, CX, BX: back to the caller's values.
    c.set_x(EAX, sub.x(EAX));
    let mut bp: u16 = 0xfc08;
    let mut di = si.wrapping_add(0x22);
    let mut k = 0u64;
    loop {
        let al = bus.read8(lin(ss, bp));
        bus.write8(lin(es, di), al);
        di = di.wrapping_add(1);
        bp = bp.wrapping_add(1);
        k += 1;
        if al == 0 {
            c.set_x(EAX, (sub.x(EAX) & 0xff00) | al as u16);
            break;
        }
    }
    c.set_x(EDI, di);
    c.set_x(EBP, bp);
    Some(20 + 1 + name_steps + 5 * k)
}

/// Port of `EL2M00:4db8` (entered by a near `CALL`).
pub fn body_name_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    match body_name(c, bus) {
        Some(n) => {
            near_ret(c, bus);
            n + 1
        }
        None => DECLINE,
    }
}

/// Port of `EL2M00:4bda`, *observed*: reset the body at `ES:SI` (`[SI]` = 0, `[SI+2]` = 38h);
/// `CX` = `DS:2049`, `BX` = its temperature `ES:[SI+20h]`. When `BX` ≥ DCh and `CX` > 32h
/// (signed), it is named with kind 10 ([`body_name`]).
pub fn body_4bda_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (es, si) = (c.es, c.x(ESI));
    bus.write16(lin(es, si), 0);
    bus.write16(lin(es, si.wrapping_add(2)), 0x38);
    let cx = bus.read16(lin(c.ds, 0x2049));
    let bx = bus.read16(lin(es, si.wrapping_add(0x20)));
    c.set_x(ECX, cx);
    c.set_x(EBX, bx);
    let n = if (bx as i16) < 0xdc {
        7
    } else if (cx as i16) <= 0x32 {
        9
    } else {
        c.set_x(EAX, 0x0a);
        match body_name(c, bus) {
            Some(m) => 10 + m + 2, // + 4db8's RET and this RET
            None => return DECLINE,
        }
    };
    near_ret(c, bus);
    n
}

/// Every ported routine of `EL2M00` (near routines: slot 0).
pub const PORTS_M00: &[(u16, u16, &str, Port)] = &[
    (0x4bda, 0, "body_4bda", body_4bda_port),
    (0x4db8, 0, "body_name", body_name_port),
    (0x4df5, 0, "sys_step", sys_step_port),
    (0x4e29, 0, "lookup_3c39", lookup_3c39_port),
];
