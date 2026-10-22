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

/// `CALLF [DS:slot]` into an `EL2MATH3` routine through its native port ([`crate::math3::PORTS`]),
/// as if the far call were executed at `ret_ip`. Returns the instructions including the `CALLF`,
/// or `None` if no port covers the slot or the port declines.
fn callf_math3(c: &mut Cpu, bus: &mut dyn Bus, slot: u16, ret_ip: u16) -> Option<u64> {
    let port = crate::math3::PORTS.iter().find(|p| p.1 == slot)?.3;
    let off = bus.read16(lin(c.ds, slot));
    let seg = bus.read16(lin(c.ds, slot + 2));
    let cs = c.cs;
    push16(c, bus, cs);
    push16(c, bus, ret_ip);
    c.cs = seg;
    c.ip = off;
    let n = port(c, bus);
    if n == DECLINE {
        return None;
    }
    Some(n + 1)
}

/// `CALL 4db8` from inside a routine: the return address is pushed (dead after the `RET`).
fn call_body_name(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> Option<u64> {
    push16(c, bus, ret_ip);
    let n = body_name(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    Some(1 + n + 1)
}

/// Copy the 20h-word template at `EL2M00:tpl` to `ES:SI` (`MOV BX,SI; MOV CX,20h` and the
/// `LOOP`), leaving `BX`, `CX`, `DI` as the original does. 162 instructions.
fn copy_template(c: &mut Cpu, bus: &mut dyn Bus, tpl: u16) -> u64 {
    let (es, si, cs) = (c.es, c.x(ESI), c.cs);
    for k in 0..0x20u16 {
        let w = bus.read16(lin(cs, tpl.wrapping_add(2 * k)));
        bus.write16(lin(es, si.wrapping_add(2 * k)), w);
        c.set_x(EAX, w);
    }
    c.set_x(EBX, si.wrapping_add(0x40));
    c.set_x(ECX, 0);
    c.set_x(EDI, tpl.wrapping_add(0x40));
    2 + 5 * 0x20
}

/// `EL2M00:4c32` without its `RET`, *observed*: add up to four small bodies after the body at
/// `ES:SI` while fewer than 18 bodies exist (`DS:204d`). Count: the high word of `BX × low16(s1)`
/// (system seed `DS:2025`), at most 3, plus one. A private copy of the seed (`DS:07c4`/`07c8`)
/// is stepped like `random(n)` (`s1 += s0; s0 = rol(rol(s0,5) + s1, 16) + s1`) once per body.
/// Each body: the 40h-byte template at `EL2M00:1d63`, `[+0]` = 0039_0000h, `[+4]` = the private
/// seed, `[+8]` = `DS:204b`, `[+1Eh]`/`[+12h]` from `EL2M00:3e99[++DS:202d & Fh]`, named with
/// kind 6, type `[+0Ch]` = `EL2M00:3e39[(CX >> 4 & 1Eh) + DX + ([+6] & 6)]` (`CX`, `DX` as
/// pushed on entry).
fn moons(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let ds = c.ds;
    let count = bus.read8(lin(ds, 0x204d));
    if count >= 0x12 {
        return Some(2);
    }
    let (dx0, cx0) = (c.x(EDX), c.x(ECX));
    push16(c, bus, dx0);
    push16(c, bus, cx0);
    let s0 = bus.read32(lin(ds, SYS_S0));
    let s1 = bus.read32(lin(ds, SYS_S1));
    bus.write32(lin(ds, 0x07c4), s0);
    bus.write32(lin(ds, 0x07c8), s1);
    c.r[EDI] = s1;
    let p = c.x(EBX) as u32 * (s1 & 0xffff);
    c.set_x(EAX, p as u16);
    let mut dx = (p >> 16) as u16;
    let mut n: u64 = 2 + 10;
    if (dx as i16) >= 4 {
        dx = 3;
        n += 1;
    }
    c.set_x(EDX, dx);
    bus.write16(lin(ds, 0x07c0), dx);
    n += 1;
    loop {
        let a = bus.read32(lin(ds, 0x07c4));
        let b = bus.read32(lin(ds, 0x07c8)).wrapping_add(a);
        let a = a.rotate_left(5).wrapping_add(b).rotate_left(16).wrapping_add(b);
        bus.write32(lin(ds, 0x07c4), a);
        bus.write32(lin(ds, 0x07c8), b);
        c.r[EAX] = a;
        c.r[EBX] = b;
        let si = c.x(ESI).wrapping_add(0x40);
        c.set_x(ESI, si);
        n += 11;
        n += copy_template(c, bus, 0x1d63);
        let es = c.es;
        bus.write32(lin(es, si), 0x0039_0000);
        let v = bus.read16(lin(ds, 0x204b));
        c.set_x(EAX, v);
        bus.write16(lin(es, si.wrapping_add(8)), v);
        c.r[EAX] = bus.read32(lin(ds, 0x07c4));
        bus.write32(lin(es, si.wrapping_add(4)), c.r[EAX]);
        let k = bus.read16(lin(ds, 0x202d)).wrapping_add(1);
        bus.write16(lin(ds, 0x202d), k);
        let bx = (k & 0x0f) << 2;
        c.set_x(EBX, bx);
        let d1 = bus.read16(lin(c.cs, 0x3e99u16.wrapping_add(bx)));
        bus.write16(lin(es, si.wrapping_add(0x1e)), d1);
        let d2 = bus.read16(lin(c.cs, 0x3e9bu16.wrapping_add(bx)));
        bus.write16(lin(es, si.wrapping_add(0x12)), d2);
        c.set_x(EDX, d2);
        c.set_x(EAX, 6);
        n += 13 + 1;
        n += call_body_name(c, bus, 0x4cda)?;
        let bp = c.x(ESP);
        c.set_x(EBP, bp);
        let pushed_cx = bus.read16(lin(c.ss, bp));
        let pushed_dx = bus.read16(lin(c.ss, bp.wrapping_add(2)));
        let mut bx = ((pushed_cx >> 4) & 0x1e).wrapping_add(pushed_dx);
        let d = bus.read16(lin(es, si.wrapping_add(6))) & 6;
        c.set_x(EDX, d);
        bx = bx.wrapping_add(d);
        c.set_x(EBX, bx);
        let t = bus.read16(lin(c.cs, 0x3e39u16.wrapping_add(bx)));
        c.set_x(EAX, t);
        bus.write16(lin(es, si.wrapping_add(0x0c)), t);
        let cnt = bus.read8(lin(ds, 0x204d)).wrapping_add(1);
        bus.write8(lin(ds, 0x204d), cnt);
        n += 13;
        if cnt >= 0x12 {
            break;
        }
        let left = bus.read16(lin(ds, 0x07c0)).wrapping_sub(1);
        bus.write16(lin(ds, 0x07c0), left);
        n += 2;
        if left & 0x8000 != 0 {
            break;
        }
    }
    // POP CX; POP DX
    let sp = c.x(ESP);
    c.set_x(ECX, bus.read16(lin(c.ss, sp)));
    c.set_x(EDX, bus.read16(lin(c.ss, sp.wrapping_add(2))));
    c.set_x(ESP, sp.wrapping_add(4));
    Some(n + 2)
}

/// Port of `EL2M00:4c32` (entered by a near `CALL`).
pub fn moons_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    match moons(c, bus) {
        Some(n) => {
            near_ret(c, bus);
            n + 1
        }
        None => DECLINE,
    }
}

/// `EL2M00:4d10` without its `RET`, *observed*: add one large body after `ES:SI` while fewer than
/// 18 bodies exist; `DS:204d` and `DS:204e` count up. Template `EL2M00:19e3` (`AX` = 0),
/// `1da3` (1) or `1c63` (other); `[+0]` = 0039_0000h, `[+4]` = the system seed `s0`, `[+8]` =
/// `DS:204b`, `[+0Eh]` = 7FFFh, `[+10h]` = 22h. Its size `[+14h]`/`[+16h]` is
/// `sqrt((7FFFh·2^22h)² / parent) × 5EDBh` in the `EL2MATH3` float format, the parent being the
/// body `DS:204b − DS:202d` records back. Named with kind 7.
fn big_body(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let ds = c.ds;
    if bus.read8(lin(ds, 0x204d)) >= 0x12 {
        return Some(2);
    }
    let v = bus.read8(lin(ds, 0x204d)).wrapping_add(1);
    bus.write8(lin(ds, 0x204d), v);
    let v = bus.read8(lin(ds, 0x204e)).wrapping_add(1);
    bus.write8(lin(ds, 0x204e), v);
    let si = c.x(ESI).wrapping_add(0x40);
    c.set_x(ESI, si);
    let ax = c.x(EAX);
    let mut n: u64 = 2 + 6;
    let tpl = if ax == 0 {
        0x19e3
    } else if ax == 1 {
        n += 3;
        c.set_x(EAX, 0);
        0x1da3
    } else {
        n += 4;
        c.set_x(EAX, ax.wrapping_sub(1));
        0x1c63
    };
    c.set_x(EDI, tpl);
    n += copy_template(c, bus, tpl);
    let es = c.es;
    bus.write32(lin(es, si), 0x0039_0000);
    c.r[EAX] = bus.read32(lin(ds, SYS_S0));
    bus.write32(lin(es, si.wrapping_add(4)), c.r[EAX]);
    let v = bus.read16(lin(ds, 0x204b));
    c.set_x(EAX, v);
    bus.write16(lin(es, si.wrapping_add(8)), v);
    let k = bus.read16(lin(ds, 0x202d)).wrapping_add(1);
    bus.write16(lin(ds, 0x202d), k);
    // MOV AX,7FFFh; MOV BX,22h; MOV [+0Eh],AX; MOV [+10h],BX; MOV DX,AX; MOV CX,BX; PUSH DX
    c.set_x(EAX, 0x7fff);
    c.set_x(EBX, 0x22);
    bus.write16(lin(es, si.wrapping_add(0x0e)), 0x7fff);
    bus.write16(lin(es, si.wrapping_add(0x10)), 0x22);
    c.set_x(EDX, 0x7fff);
    c.set_x(ECX, 0x22);
    push16(c, bus, 0x7fff);
    n += 6 + 7;
    n += callf_math3(c, bus, 0x02b4, 0x4d79)?;
    let dx = c.x(EDX);
    c.set_x(EAX, dx);
    let sp = c.x(ESP);
    c.set_x(EDX, bus.read16(lin(c.ss, sp)));
    c.set_x(ESP, sp.wrapping_add(2));
    n += 2;
    n += callf_math3(c, bus, 0x02b4, 0x4d80)?;
    let bp = bus.read16(lin(ds, 0x204b)).wrapping_sub(bus.read16(lin(ds, 0x202d))) << 6;
    c.set_x(EBP, bp);
    let bx = c.x(EBX);
    c.set_x(ECX, bx);
    let si = c.x(ESI);
    c.set_x(EBX, bus.read16(lin(es, bp.wrapping_add(si))));
    c.set_x(EAX, bus.read16(lin(es, bp.wrapping_add(si).wrapping_add(2))));
    n += 6;
    n += callf_math3(c, bus, 0x0288, 0x4d98)?;
    let cx = c.x(ECX);
    c.set_x(EDX, cx);
    n += 1;
    n += callf_math3(c, bus, 0x02c8, 0x4d9e)?;
    let dx = c.x(EDX);
    c.set_x(EBX, dx);
    c.set_x(EDX, 0x5edb);
    c.set_x(ECX, 0);
    n += 3;
    n += callf_math3(c, bus, 0x02b4, 0x4da9)?;
    let si = c.x(ESI);
    bus.write16(lin(es, si.wrapping_add(0x14)), c.x(EDX));
    bus.write16(lin(es, si.wrapping_add(0x16)), c.x(EBX));
    c.set_x(EAX, 7);
    n += 2 + 1;
    n += call_body_name(c, bus, 0x4db7)?;
    Some(n)
}

/// Port of `EL2M00:4d10` (entered by a near `CALL`).
pub fn big_body_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    match big_body(c, bus) {
        Some(n) => {
            near_ret(c, bus);
            n + 1
        }
        None => DECLINE,
    }
}

fn call_moons(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> Option<u64> {
    push16(c, bus, ret_ip);
    let n = moons(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    Some(1 + n + 1)
}

fn call_big_body(c: &mut Cpu, bus: &mut dyn Bus, ret_ip: u16) -> Option<u64> {
    push16(c, bus, ret_ip);
    let n = big_body(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    Some(1 + n + 1)
}

/// Body record fields the classifiers write: `+0Ah`, `+0Ch` (the type/model), `+20h` (temperature).
fn set_field(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u16) {
    bus.write16(lin(c.es, c.x(ESI).wrapping_add(off)), v);
}

fn field(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u16 {
    bus.read16(lin(c.es, c.x(ESI).wrapping_add(off)))
}

/// `EL2M00:4b69` without its `RET`, *observed*: a body with temperature 96h–1F4h is named with
/// kind 10 when the parent value (`DS:2049`) is above B4h, or when the temperature is at least
/// C8h and the parent value above 8; from a parent value of 2Dh it gets small bodies (`BX` = 0,
/// `DX` = 18h) and from 96h a large body (`AX` = 0).
fn classify_4b69(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let cx = bus.read16(lin(c.ds, 0x2049));
    let bx = field(c, bus, 0x20);
    c.set_x(ECX, cx);
    c.set_x(EBX, bx);
    let (cx, bx) = (cx as i16, bx as i16);
    if bx > 0x1f4 {
        return Some(4);
    }
    if bx < 0x96 {
        return Some(6);
    }
    let mut n = 8;
    if cx <= 0xb4 {
        n += 2;
        if bx < 0xc8 {
            return Some(n);
        }
        n += 2;
        if cx <= 8 {
            return Some(n);
        }
    }
    c.set_x(EAX, 0x0a);
    n += 1;
    n += call_body_name(c, bus, 0x4b94)?;
    n += 2;
    if (c.x(ECX) as i16) < 0x2d {
        return Some(n);
    }
    c.set_x(EBX, 0);
    c.set_x(EDX, 0x18);
    n += 2;
    n += call_moons(c, bus, 0x4ba1)?;
    n += 2;
    if (c.x(ECX) as i16) < 0x96 {
        return Some(n);
    }
    c.set_x(EAX, 0);
    n += 1;
    n += call_big_body(c, bus, 0x4bac)?;
    Some(n)
}

/// `EL2M00:4abb` without its `RET`, *observed*: the cool-body classifier. Temperature `+= 14h`.
/// With a parent value of at least 96h and a temperature of 111h–139h: temperature 127h, type
/// 88h, `+0Ah` = 12h, named kind 8, small bodies (`BX` = parent >> 6, `DX` = 0), a large body
/// (`AX` = 1 from a parent value of B4h). Otherwise type 7Eh, `+0Ah` = 8, and (temperature up to
/// 212h, and either a parent value ≥ C8h with temperature ≥ A0h, or temperature ≥ B4h with a
/// parent value above 6): `+2` capped at 47h, named kind 10, small bodies from a parent value of
/// 1Eh (`BX` = 0, `DX` = 18h), a large body from 64h (`AX` = 1 from AAh).
fn classify_4abb(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let cx = bus.read16(lin(c.ds, 0x2049));
    let bx = field(c, bus, 0x20).wrapping_add(0x14);
    set_field(c, bus, 0x20, bx);
    c.set_x(ECX, cx);
    c.set_x(EBX, bx);
    let (cxs, bxs) = (cx as i16, bx as i16);
    let mut n: u64 = 6;
    if cxs >= 0x96 {
        n += 2;
        if bxs >= 0x111 {
            n += 2;
            if bxs <= 0x139 {
                set_field(c, bus, 0x20, 0x127);
                set_field(c, bus, 0x0c, 0x88);
                set_field(c, bus, 0x0a, 0x12);
                c.set_x(EAX, 8);
                n += 4;
                n += call_body_name(c, bus, 0x4b51)?;
                let b = c.x(ECX) >> 6;
                c.set_x(EBX, b);
                c.set_x(EDX, 0);
                n += 3;
                n += call_moons(c, bus, 0x4b5c)?;
                let mut ax = 0u16;
                n += 3;
                if (c.x(ECX) as i16) >= 0xb4 {
                    ax = 1;
                    n += 1;
                }
                c.set_x(EAX, ax);
                n += call_big_body(c, bus, 0x4b68)?;
                return Some(n);
            }
        }
    }
    // 4ad0
    set_field(c, bus, 0x0c, 0x7e);
    set_field(c, bus, 0x0a, 0x08);
    n += 4;
    if bxs > 0x212 {
        return Some(n);
    }
    n += 2;
    let to_4af9 = if cxs >= 0xc8 {
        n += 2;
        bxs >= 0xa0
    } else {
        false
    };
    if !to_4af9 {
        n += 2;
        if bxs < 0xb4 {
            return Some(n);
        }
        n += 2;
        if cxs <= 6 {
            return Some(n);
        }
    }
    // 4af9
    n += 2;
    if (field(c, bus, 0x02) as i16) >= 0x47 {
        set_field(c, bus, 0x02, 0x47);
        n += 1;
    }
    c.set_x(EAX, 0x0a);
    n += 1;
    n += call_body_name(c, bus, 0x4b0c)?;
    n += 2;
    if (c.x(ECX) as i16) < 0x1e {
        return Some(n);
    }
    c.set_x(EBX, 0);
    c.set_x(EDX, 0x18);
    n += 2;
    n += call_moons(c, bus, 0x4b19)?;
    n += 2;
    if (c.x(ECX) as i16) < 0x64 {
        return Some(n);
    }
    let mut ax = 0u16;
    n += 3;
    if (c.x(ECX) as i16) >= 0xaa {
        ax = 1;
        n += 1;
    }
    c.set_x(EAX, ax);
    n += call_big_body(c, bus, 0x4b2c)?;
    Some(n)
}

/// `EL2M00:4990` without its `RET`, *observed*: the temperate-body classifier (habitable band
/// D8h–139h), with branches for hot (above 139h), warm (AAh–D7h), cool (42h–7Bh) and the rest
/// (which continue in [`classify_4abb`]). See docs/SYSTEM.md for the table of outcomes.
fn classify_4990(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let cx = bus.read16(lin(c.ds, 0x2049));
    let mut bx = field(c, bus, 0x20);
    c.set_x(ECX, cx);
    c.set_x(EBX, bx);
    let cxs = cx as i16;
    let mut n: u64 = 4;
    if bx > 0x139 {
        // 4a65: hot
        set_field(c, bus, 0x0c, 0x84);
        set_field(c, bus, 0x0a, 0x18);
        let ax = bus.read16(lin(c.ds, 0x07bc)) >> 1;
        c.set_x(EAX, ax);
        bx = bx.wrapping_add(ax);
        c.set_x(EBX, bx);
        set_field(c, bus, 0x20, bx);
        return Some(n + 6);
    }
    n += 2;
    if bx <= 0xd7 {
        // 4a7d
        n += 2;
        if (bx as i16) < 0xaa {
            // 4a9d
            n += 2;
            if (bx as i16) > 0x7b {
                return Some(n + classify_4abb(c, bus)?);
            }
            n += 2;
            if (bx as i16) < 0x42 {
                return Some(n + classify_4abb(c, bus)?);
            }
            bx = bx.wrapping_add(0x16);
            c.set_x(EBX, bx);
            set_field(c, bus, 0x20, bx);
            set_field(c, bus, 0x0c, 0x86);
            set_field(c, bus, 0x0a, 0x0e);
            return Some(n + 4);
        }
        bx = bx.wrapping_add(0x19);
        c.set_x(EBX, bx);
        set_field(c, bus, 0x20, bx);
        n += 4;
        if cxs >= 0xc8 {
            return self_4a45(c, bus, n);
        }
        set_field(c, bus, 0x0c, 0x84);
        set_field(c, bus, 0x0a, 0x0c);
        return Some(n + 2);
    }
    // Habitable band.
    bx = bx.wrapping_add(0x23);
    set_field(c, bus, 0x20, bx);
    n += 3; // ADD; MOV; PUSH CX
    push16(c, bus, cx);
    push16(c, bus, 0x49b3);
    n += 1 + sys_step_inline(c, bus);
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(4)); // RET; POP CX
    c.set_x(ECX, cx);
    n += 1;
    let bx = field(c, bus, 0x20).wrapping_sub(0xfa);
    c.set_x(EBX, bx);
    let al = (c.x(EAX) as u8) >> 2;
    c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
    n += 5; // MOV BX; SUB; SHR AL; CMP; JA
    if al > bx as u8 {
        // 4a33
        n += 2;
        if cxs < 0x78 {
            set_field(c, bus, 0x0c, 0x8e);
            set_field(c, bus, 0x0a, 0x10);
            return Some(n + 2);
        }
        return self_4a45(c, bus, n);
    }
    n += 2;
    if cxs < 4 {
        return Some(n);
    }
    c.set_x(EAX, 9);
    n += 1;
    n += call_body_name(c, bus, 0x49ce)?;
    n += 2;
    if cxs < 0x10 {
        return Some(n);
    }
    n += 2;
    if (field(c, bus, 0x20) as i16) <= 0x118 {
        // 4a11
        set_field(c, bus, 0x0c, 0x8c);
        c.set_x(EBX, cx >> 6);
        c.set_x(EDX, 0x30);
        n += 4;
        n += call_moons(c, bus, 0x4a22)?;
        n += 2;
        if cxs < 0x28 {
            return Some(n);
        }
        c.set_x(EAX, 0);
        n += 3;
        if cxs >= 0x50 {
            c.set_x(EAX, 1);
            n += 1;
        }
        n += call_big_body(c, bus, 0x4a32)?;
        return Some(n);
    }
    n += 2;
    if bus.read16(lin(c.ds, SYS_S0)) & 0x0f == 0 {
        set_field(c, bus, 0x0c, 0x8e);
        c.set_x(EDX, 0x48);
        n += 3;
    } else {
        c.set_x(EDX, 0);
        n += 1;
    }
    after_49f1(c, bus, n)
}

/// `EL2M00:49f1`: small bodies (`BX` = parent >> 5), then a large body sized by the parent value.
fn after_49f1(c: &mut Cpu, bus: &mut dyn Bus, mut n: u64) -> Option<u64> {
    let cx = c.x(ECX);
    c.set_x(EBX, cx >> 5);
    n += 2;
    n += call_moons(c, bus, 0x49f9)?;
    n += 2;
    let cxs = c.x(ECX) as i16;
    if cxs < 0x1e {
        return Some(n);
    }
    let mut ax = 0u16;
    n += 3;
    if cxs >= 0x32 {
        ax = 1;
        n += 3;
        if cxs >= 0xf2 {
            ax = 2;
            n += 1;
        }
    }
    c.set_x(EAX, ax);
    n += call_big_body(c, bus, 0x4a10)?;
    Some(n)
}

/// `EL2M00:4a45` (in [`classify_4990`]): temperature at least 127h, type 8Ah, `+0Ah` = 16h, named
/// kind 8, then (from `49ee`) `DX` = 0 and [`after_49f1`].
fn self_4a45(c: &mut Cpu, bus: &mut dyn Bus, mut n: u64) -> Option<u64> {
    n += 2;
    if (c.x(EBX) as i16) < 0x127 {
        set_field(c, bus, 0x20, 0x127);
        n += 1;
    }
    set_field(c, bus, 0x0c, 0x8a);
    set_field(c, bus, 0x0a, 0x16);
    c.set_x(EAX, 8);
    n += 3;
    n += call_body_name(c, bus, 0x4a63)?;
    c.set_x(EDX, 0);
    n += 2; // JMP 49ee; MOV DX,0
    after_49f1(c, bus, n)
}

macro_rules! near_port {
    ($name:ident, $inner:ident) => {
        /// Port of the `EL2M00` routine of the same name (entered by a near `CALL`).
        pub fn $name(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
            match $inner(c, bus) {
                Some(n) => {
                    near_ret(c, bus);
                    n + 1
                }
                None => DECLINE,
            }
        }
    };
}

near_port!(classify_4990_port, classify_4990);
near_port!(classify_4abb_port, classify_4abb);
near_port!(classify_4b69_port, classify_4b69);

/// Every ported routine of `EL2M00` (near routines: slot 0).
pub const PORTS_M00: &[(u16, u16, &str, Port)] = &[
    (0x4990, 0, "classify_4990", classify_4990_port),
    (0x4abb, 0, "classify_4abb", classify_4abb_port),
    (0x4b69, 0, "classify_4b69", classify_4b69_port),
    (0x4bda, 0, "body_4bda", body_4bda_port),
    (0x4c32, 0, "moons", moons_port),
    (0x4d10, 0, "big_body", big_body_port),
    (0x4db8, 0, "body_name", body_name_port),
    (0x4df5, 0, "sys_step", sys_step_port),
    (0x4e29, 0, "lookup_3c39", lookup_3c39_port),
];
