//! Star-system generation in `EL2M00` (docs/SYSTEM.md): the system seed, body records and their
//! classification, names and the moons and starports added to them.
//!
//! Body records are 40h bytes in `ES` (`ES:SI`); `DS:204d` counts the bodies made (at most 18) and
//! `DS:2049` is the current body's parent value the classifiers branch on. The ports are register-
//! and memory-exact and return the original's instruction counts; tables are read from `EL2M00`
//! in memory.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use crate::names::name_port;

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

/// `EL2M00:4bad` without its `RET`, *observed*: a body at least DCh hot under a parent value
/// above 32h is named with kind 10; from a parent value of 50h it gets small bodies (`BX` = 0,
/// `DX` = 18h).
fn classify_4bad(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let cx = bus.read16(lin(c.ds, 0x2049));
    let bx = field(c, bus, 0x20);
    c.set_x(ECX, cx);
    c.set_x(EBX, bx);
    if (bx as i16) < 0xdc {
        return Some(4);
    }
    if (cx as i16) <= 0x32 {
        return Some(6);
    }
    c.set_x(EAX, 0x0a);
    let mut n = 7 + call_body_name(c, bus, 0x4bca)?;
    n += 2;
    if (c.x(ECX) as i16) < 0x50 {
        return Some(n);
    }
    c.set_x(EBX, 0);
    c.set_x(EDX, 0x18);
    n += 2;
    n += call_moons(c, bus, 0x4bd9)?;
    Some(n)
}

/// `EL2M00:4bff` without its `RET`, *observed*: from the third body on (`DS:202d` > 2) the body
/// becomes type AEh, class 32h, `+18h` = 3E2h, temperature 2C09h, and `+02h` is decremented
/// (a white dwarf, *inferred*).
fn classify_4bff(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let k = bus.read16(lin(c.ds, 0x202d));
    c.set_x(EAX, k);
    if (k as i16) <= 2 {
        return 3;
    }
    set_field(c, bus, 0x0c, 0xae);
    set_field(c, bus, 0x0a, 0x32);
    let (es, si) = (c.es, c.x(ESI));
    bus.write32(lin(es, si.wrapping_add(0x18)), 0x3e2);
    set_field(c, bus, 0x20, 0x2c09);
    let v = field(c, bus, 0x02).wrapping_sub(1);
    set_field(c, bus, 0x02, v);
    8
}

/// `CALL 4df5` inside a routine: the step, and whether the new `s0` is negative (the `SF` the
/// caller's `JS`/`JNS` tests).
fn call_sys_step(c: &mut Cpu, bus: &mut dyn Bus) -> (u64, bool) {
    let n = sys_step_inline(c, bus);
    (1 + n, c.r[EAX] & 0x8000_0000 != 0)
}

/// `EL2M00:4c2c` without its `RET`, *observed*: one system-seed step; a negative `s0` continues in
/// [`classify_4bff`].
fn classify_4c2c(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (n, neg) = call_sys_step(c, bus);
    if neg {
        return n + 1 + classify_4bff(c, bus);
    }
    n + 1
}

/// `EL2M00:4c27` without its `RET`, *observed*: one step; a non-negative `s0` ends; otherwise
/// [`classify_4c2c`].
fn classify_4c27(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (n, neg) = call_sys_step(c, bus);
    if !neg {
        return n + 1;
    }
    n + 1 + classify_4c2c(c, bus)
}

/// `OR AX,AX` flags: ZF, SF, PF from `AX`, CF and OF clear.
fn or_flags(c: &mut Cpu) {
    let ax = c.x(EAX);
    let mut f = c.flags & !0x08D5;
    if ax == 0 {
        f |= 0x40;
    }
    if ax & 0x8000 != 0 {
        f |= 0x80;
    }
    if (ax as u8).count_ones() % 2 == 0 {
        f |= 0x04;
    }
    c.flags = f;
}

/// `CALLF [DS:02c4]` (`from_u32`) etc. with the return offset `ret`.
fn m3(c: &mut Cpu, bus: &mut dyn Bus, slot: u16, ret: u16) -> Option<u64> {
    callf_math3(c, bus, slot, ret)
}

/// `EL2M00:47d8` without its `RET`, *observed*: finish the body at `ES:SI`.
///
/// Mass `+0Eh`/`+10h` = `DS:07c0` × `DS:07c8` (16 × 32 bits) as a float (`from_u32`, exponent
/// `+ 19h`); the radius `+14h`/`+16h` from the mass and the parent body `[DS:07e0]` (Roche-style:
/// `sqrt(mass² / parent) × 5EDBh`, one exponent higher for a close moon of a body with `+0Ah` = 0),
/// or 0 for a primary (`DS:2037` = 0); `+12h` = `s0` low word; seed `+04h` = `s1`; parent index
/// `+08h` = `DS:2037`; orbit `+00h`/`+02h` = `DS:07bc` as a float (exponent `+ 3Fh`); `+1Ch`, `+1Eh`,
/// `+3Ah`, `+3Bh`, `+3Eh` from the seed. Unless the temperature `+20h` is above 3E8h, the heat of
/// every body up the parent chain (records at `[DS:2041]:[DS:203f]`, their `+18h`) is summed in
/// float and added to `+20h`. `+3Ch` = 0, `DS:204b` = `DS:202d`, then the classifier at
/// `[DS:07e8]` is called (`CALL AX`) if set. Returns `AX` = FFFFh with ZF clear.
fn finish_body(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let ds = c.ds;
    let es = c.es;
    let si = c.x(ESI);
    let r16 = |bus: &mut dyn Bus, off: u16| bus.read16(lin(ds, off));
    let mut n: u64 = 0;
    // MOV AX,[7c0]; PUSH AX; MUL [7c8]; POP AX; PUSH CX; PUSH DX; MUL [7ca]; POP CX; ADD; ADC; POP CX
    let a = r16(bus, 0x07c0) as u32;
    let lo = a * r16(bus, 0x07c8) as u32;
    let hi = a * r16(bus, 0x07ca) as u32;
    let (sum, carry) = hi.overflowing_add(lo >> 16);
    let _ = carry;
    let mut ax = sum as u16;
    let mut dx = ((hi >> 16) as u16).wrapping_add(((hi & 0xffff) + (lo >> 16) > 0xffff) as u16);
    c.set_x(EAX, ax);
    c.set_x(EDX, dx);
    bus.write16(lin(es, si.wrapping_add(0x0e)), ax);
    bus.write16(lin(es, si.wrapping_add(0x10)), dx);
    let cx0 = c.x(ECX);
    c.set_x(ECX, ax | dx);
    n += 16;
    let mut di = c.x(EDI);
    let _ = cx0;
    if ax | dx != 0 {
        n += m3(c, bus, 0x02c4, 0x4800)?;
        let bx = c.x(EBX).wrapping_add(0x19);
        c.set_x(EBX, bx);
        bus.write16(lin(es, si.wrapping_add(0x0e)), c.x(EDX));
        bus.write16(lin(es, si.wrapping_add(0x10)), bx);
        let parent = r16(bus, 0x2037);
        c.set_x(EAX, parent);
        c.set_x(ECX, parent);
        n += 7;
        if parent != 0 {
            di = r16(bus, 0x07e0);
            c.set_x(EDI, di);
            n += 3;
            if bus.read16(lin(es, di.wrapping_add(0x0a))) == 0 {
                let d = si.wrapping_sub(di);
                c.set_x(EAX, d);
                n += 4;
                if (d as i16) <= 0x81 {
                    c.set_x(EBX, c.x(EBX).wrapping_add(1));
                    n += 1;
                }
            }
            // 4829
            let bx = c.x(EBX);
            c.set_x(ECX, bx);
            let dx = c.x(EDX);
            c.set_x(EAX, dx);
            push16(c, bus, dx);
            n += 3;
            n += m3(c, bus, 0x02b4, 0x4832)?;
            let d = c.x(EDX);
            c.set_x(EAX, d);
            let sp = c.x(ESP);
            c.set_x(EDX, bus.read16(lin(c.ss, sp)));
            c.set_x(ESP, sp.wrapping_add(2));
            n += 2;
            n += m3(c, bus, 0x02b4, 0x4839)?;
            let bx = c.x(EBX);
            c.set_x(ECX, bx);
            c.set_x(EBX, bus.read16(lin(es, di)));
            c.set_x(EAX, bus.read16(lin(es, di.wrapping_add(2))));
            n += 3;
            n += m3(c, bus, 0x0288, 0x4846)?;
            let cx = c.x(ECX);
            c.set_x(EDX, cx);
            n += 1;
            n += m3(c, bus, 0x02c8, 0x484c)?;
            let d = c.x(EDX);
            c.set_x(EBX, d);
            c.set_x(EDX, 0x5edb);
            c.set_x(ECX, 0);
            n += 3;
            n += m3(c, bus, 0x02b4, 0x4857)?;
            let d = c.x(EDX);
            c.set_x(ECX, d);
            let b = c.x(EBX);
            c.set_x(EAX, b);
            n += 2;
        }
        // 485b
        bus.write16(lin(es, si.wrapping_add(0x14)), c.x(ECX));
        bus.write16(lin(es, si.wrapping_add(0x16)), c.x(EAX));
        let s0lo = r16(bus, SYS_S0);
        c.set_x(EAX, s0lo);
        bus.write16(lin(es, si.wrapping_add(0x12)), s0lo);
        n += 4;
    } else {
        // JZ 486a: CX = AX | DX = 0
    }
    // 486a
    let s1 = bus.read32(lin(ds, SYS_S1));
    c.r[EAX] = s1;
    bus.write32(lin(es, si.wrapping_add(4)), s1);
    let k = r16(bus, 0x202d).wrapping_add(1);
    bus.write16(lin(ds, 0x202d), k);
    let parent = r16(bus, 0x2037);
    c.set_x(EAX, parent);
    bus.write16(lin(es, si.wrapping_add(8)), parent);
    c.set_x(EAX, r16(bus, 0x07bc));
    c.set_x(EDX, r16(bus, 0x07be));
    n += 7;
    n += m3(c, bus, 0x02c4, 0x4889)?;
    let bx = c.x(EBX).wrapping_add(0x3f);
    c.set_x(EBX, bx);
    bus.write16(lin(es, si), c.x(EDX));
    bus.write16(lin(es, si.wrapping_add(2)), bx);
    let w = r16(bus, 0x2023);
    let p = w as u32 * w as u32;
    let d = ((p >> 24) & 0xff) as u16;
    bus.write16(lin(es, si.wrapping_add(0x1c)), d);
    let w3 = w.rotate_right(3);
    let p = w3 as u32 * w3 as u32;
    let d2 = ((p >> 16) as u16 >> 4).wrapping_add(bus.read16(lin(es, di.wrapping_add(0x3e))));
    bus.write16(lin(es, si.wrapping_add(0x1e)), d2);
    let w6 = bus.read16(lin(es, si.wrapping_add(6)));
    let p = w6 as u32 * w6 as u32;
    let d3 = (p >> 16) as u16 >> 1;
    bus.write16(lin(es, si.wrapping_add(0x3e)), d3);
    bus.write8(lin(es, si.wrapping_add(0x3a)), (d3 as u8) & 3);
    let f = bus.read8(lin(ds, 0x2034));
    bus.write8(lin(es, si.wrapping_add(0x3b)), f);
    let temp = bus.read16(lin(es, si.wrapping_add(0x20)));
    c.set_x(EAX, temp);
    // DX after AND DL,3: DH from d3, DL & 3.
    c.set_x(EDX, (d3 & 0xff00) | (d3 & 3));
    n += 25;
    if (temp as i16) <= 0x3e8 {
        // Sum the heat up the parent chain.
        let (pb, pc) = (r16(bus, 0x07bc), r16(bus, 0x07c0));
        let mut sp = c.x(ESP);
        for v in [pb, pc, si, es] {
            sp = sp.wrapping_sub(2);
            bus.write16(lin(c.ss, sp), v);
        }
        c.set_x(ESP, sp);
        bus.write16(lin(ds, 0x07bc), 0);
        bus.write16(lin(ds, 0x07c0), 0);
        n += 6;
        let (mut cur_es, mut cur_si) = (es, si);
        let mut links = 0;
        loop {
            // A parent chain that loops never ends in the original either; the host must not
            // hang, so a walk this long is left to the original.
            links += 1;
            if links > 256 {
                return None;
            }
            let bx = bus.read16(lin(cur_es, cur_si.wrapping_add(8)));
            c.set_x(EBX, bx);
            n += 3;
            if bx == 0 {
                break;
            }
            let rec = (bx << 6).wrapping_add(r16(bus, 0x203f)).wrapping_sub(0x40);
            let pseg = r16(bus, 0x2041);
            let hx = bus.read16(lin(pseg, rec.wrapping_add(0x18)));
            let hy = bus.read16(lin(pseg, rec.wrapping_add(0x1a)));
            c.set_x(EBX, rec);
            c.set_x(EAX, hx);
            c.set_x(EDX, hy);
            c.set_x(ECX, hx | hy);
            n += 11;
            if hx | hy != 0 {
                push16(c, bus, rec);
                n += 1;
                n += m3(c, bus, 0x02c4, 0x4917)?;
                let b = c.x(EBX).wrapping_add(0x32);
                c.set_x(ECX, b);
                n += 4;
                // ES:SI is the walking pointer here.
                c.set_x(EBX, bus.read16(lin(cur_es, cur_si.wrapping_add(0x0e))));
                c.set_x(EAX, bus.read16(lin(cur_es, cur_si.wrapping_add(0x10))));
                n += m3(c, bus, 0x0288, 0x4928)?;
                let cx = c.x(ECX);
                c.set_x(EBX, cx);
                let ax = c.x(EAX);
                c.set_x(EDX, ax);
                n += 2;
                n += m3(c, bus, 0x02b4, 0x4930)?;
                let d = c.x(EDX);
                c.set_x(EAX, d);
                let b = c.x(EBX);
                c.set_x(EDX, b);
                c.set_x(EBX, r16(bus, 0x07bc));
                c.set_x(ECX, r16(bus, 0x07c0));
                n += 4;
                n += m3(c, bus, 0x027c, 0x4940)?;
                bus.write16(lin(ds, 0x07bc), c.x(EAX));
                bus.write16(lin(ds, 0x07c0), c.x(EDX));
                let sp = c.x(ESP);
                c.set_x(EBX, bus.read16(lin(c.ss, sp)));
                c.set_x(ESP, sp.wrapping_add(2));
                n += 3;
            }
            // 4948: MOV SI,BX; MOV ES,[2041]; JMP 48ee
            cur_si = c.x(EBX);
            cur_es = pseg;
            c.set_x(ESI, cur_si);
            c.es = cur_es;
            n += 3;
        }
        c.es = cur_es;
        c.set_x(ESI, cur_si);
        // 4950
        c.set_x(EAX, r16(bus, 0x07bc));
        c.set_x(EDX, r16(bus, 0x07c0));
        n += 2;
        n += m3(c, bus, 0x02c8, 0x495b)?;
        n += m3(c, bus, 0x02c8, 0x495f)?;
        let d = c.x(EDX);
        c.set_x(ECX, d);
        let a = c.x(EAX);
        c.set_x(EDX, a);
        n += 2;
        n += m3(c, bus, 0x028c, 0x4967)?;
        // POP ES; POP SI; POP [7c0]; POP [7bc]
        let sp = c.x(ESP);
        c.es = bus.read16(lin(c.ss, sp));
        c.set_x(ESI, bus.read16(lin(c.ss, sp.wrapping_add(2))));
        let v = bus.read16(lin(c.ss, sp.wrapping_add(4)));
        bus.write16(lin(ds, 0x07c0), v);
        let v = bus.read16(lin(c.ss, sp.wrapping_add(6)));
        bus.write16(lin(ds, 0x07bc), v);
        c.set_x(ESP, sp.wrapping_add(8));
        let t = bus.read16(lin(es, si.wrapping_add(0x20))).wrapping_add(c.x(EAX));
        bus.write16(lin(es, si.wrapping_add(0x20)), t);
        n += 5;
    }
    let _ = (ax, dx, &mut ax, &mut dx);
    // 4975
    bus.write16(lin(es, si.wrapping_add(0x3c)), 0);
    let k = r16(bus, 0x202d);
    c.set_x(EAX, k);
    bus.write16(lin(ds, 0x204b), k);
    let f = r16(bus, 0x07e8);
    c.set_x(EAX, f);
    n += 6;
    if f != 0 {
        push16(c, bus, 0x498a);
        n += 1;
        let m = match f {
            0x4990 => classify_4990(c, bus)?,
            0x4abb => classify_4abb(c, bus)?,
            0x4b69 => classify_4b69(c, bus)?,
            0x4bad => classify_4bad(c, bus)?,
            0x4bff => classify_4bff(c, bus),
            0x4c27 => classify_4c27(c, bus),
            0x4c2c => classify_4c2c(c, bus),
            0x4bda => {
                // 4bda's inner part: reuse the port, which ends with its own RET.
                let r = body_4bda_port(c, bus);
                if r == DECLINE {
                    return None;
                }
                r - 1
            }
            _ => return None,
        };
        let sp = c.x(ESP);
        c.set_x(ESP, sp.wrapping_add(2));
        n += m + 1;
        if f == 0x4bda {
            // body_4bda_port already popped the return address.
            c.set_x(ESP, sp);
        }
    }
    c.set_x(EAX, 0xffff);
    or_flags(c);
    n += 2;
    Some(n)
}

near_port!(finish_body_port, finish_body);

/// `EL2M00:468a` without its `RET`, *observed*: set up a new body at `ES:SI` from its mass
/// `DS:07bc` (a dword) and finish it.
///
/// `DS:07bc`, `07c0`, `07c8` and `07e0` are saved around the work. The class index is the first
/// entry of the dword thresholds at `EL2M00:00ff` above the mass; it selects `+18h` (`EL2M00:01b7`),
/// `+0Ah` (`00a3`), the temperature `+20h` (`0189`), `DS:07e4` (`015b`), the classifier
/// `DS:07e8` (`0213`) and the type `+0Ch` (`00d1`). Type 0 ends there (`AX` = 0, ZF set).
/// Otherwise `DS:202f` counts the body and it is named: from a mass of `0103_6640h` up it is a
/// star, named after the system (`[DS:2029]`) with a space and, for a companion (`DS:2037` ≠ 0),
/// the letter `'@' + ++DS:2030`. Below that, a body whose parent `[DS:07e0]` has the "named" bit
/// (`+3Bh` bit 7) gets its own kind-10 name from the seed `s1`; otherwise it takes the parent's
/// name and a suffix by
/// `DS:2034`: a number from `++DS:2031` (0) as one or two digits, a letter `'``' + ++DS:2032`
/// (1), or a digit from `++DS:2033` (other). Then [`finish_body`] with the saved values restored.
fn setup_body(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let ds = c.ds;
    let es = c.es;
    let si = c.x(ESI);
    let cs = c.cs;
    let saved = [
        bus.read32(lin(ds, 0x07bc)),
        bus.read16(lin(ds, 0x07c0)) as u32,
        bus.read32(lin(ds, 0x07c8)),
        bus.read32(lin(ds, 0x07e0)),
    ];
    // PUSH dword [7bc]; PUSH [7c0]; PUSH dword [7c8]; PUSH dword [7e0]
    let mut sp = c.x(ESP);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[0]);
    sp = sp.wrapping_sub(2);
    bus.write16(lin(c.ss, sp), saved[1] as u16);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[2]);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[3]);
    c.set_x(ESP, sp);
    let mass = saved[0];
    c.r[EAX] = mass;
    let mut di: u16 = 0;
    let mut n: u64 = 6;
    loop {
        let t = bus.read32(lin(cs, 0x00ffu16.wrapping_add(di)));
        n += 2;
        if mass < t {
            break;
        }
        di = di.wrapping_add(4);
        n += 2;
    }
    let v = bus.read32(lin(cs, 0x01b7u16.wrapping_add(di)));
    c.r[EAX] = v;
    bus.write32(lin(es, si.wrapping_add(0x18)), v);
    di >>= 1;
    let t = |bus: &mut dyn Bus, base: u16| bus.read16(lin(cs, base.wrapping_add(di)));
    let a = t(bus, 0x00a3);
    bus.write16(lin(es, si.wrapping_add(0x0a)), a);
    let a = t(bus, 0x0189);
    bus.write16(lin(es, si.wrapping_add(0x20)), a);
    let a = t(bus, 0x015b);
    bus.write16(lin(ds, 0x07e4), a);
    let a = t(bus, 0x0213);
    bus.write16(lin(ds, 0x07e8), a);
    let kind = t(bus, 0x00d1);
    bus.write16(lin(es, si.wrapping_add(0x0c)), kind);
    c.set_x(EAX, kind);
    c.set_x(EDI, di);
    n += 15;
    let restore = |c: &mut Cpu, bus: &mut dyn Bus| {
        // POP dword [7e0]; POP dword [7c8]; POP [7c0]; POP dword [7bc]
        let sp = c.x(ESP);
        let v = bus.read32(lin(c.ss, sp));
        bus.write32(lin(ds, 0x07e0), v);
        let v = bus.read32(lin(c.ss, sp.wrapping_add(4)));
        bus.write32(lin(ds, 0x07c8), v);
        let v = bus.read16(lin(c.ss, sp.wrapping_add(8)));
        bus.write16(lin(ds, 0x07c0), v);
        let v = bus.read32(lin(c.ss, sp.wrapping_add(10)));
        bus.write32(lin(ds, 0x07bc), v);
        c.set_x(ESP, sp.wrapping_add(14));
    };
    if kind == 0 {
        or_flags(c);
        restore(c, bus);
        return Some(n + 4);
    }
    let k = bus.read8(lin(ds, 0x202f)).wrapping_add(1);
    bus.write8(lin(ds, 0x202f), k);
    let mut di = si.wrapping_add(0x22);
    let (ax, dx) = (mass as u16, (mass >> 16) as u16);
    c.set_x(EAX, ax);
    c.set_x(EDX, dx);
    n += 6;
    let star = dx > 0x103 || (dx == 0x103 && ax >= 0x6640);
    if dx >= 0x103 {
        n += 1; // JNZ
        if dx == 0x103 {
            n += 2;
        }
    }
    let stos = |bus: &mut dyn Bus, di: &mut u16, v: u8| {
        bus.write8(lin(es, *di), v);
        *di = di.wrapping_add(1);
    };
    let mut named_by_4db8 = false;
    if star {
        // PUSH DS; LDS BX,[2029]; copy; POP DS
        let (noff, nseg) = (bus.read16(lin(ds, 0x2029)), bus.read16(lin(ds, 0x202b)));
        let mut bx = noff;
        n += 2;
        loop {
            let al = bus.read8(lin(nseg, bx));
            stos(bus, &mut di, al);
            bx = bx.wrapping_add(1);
            n += 5;
            c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
            if al == 0 {
                break;
            }
        }
        c.set_x(EBX, bx);
        bus.write8(lin(es, di.wrapping_sub(1)), 0x20);
        let k = bus.read8(lin(ds, 0x2030)).wrapping_add(1);
        bus.write8(lin(ds, 0x2030), k);
        n += 1 + 1 + 1 + 2;
        if bus.read16(lin(ds, 0x2037)) != 0 {
            let al = k.wrapping_add(0x40);
            c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
            stos(bus, &mut di, al);
            n += 4;
        }
        bus.write8(lin(es, di), 0);
        n += 1;
    } else {
        let parent = bus.read16(lin(ds, 0x07e0));
        c.set_x(EBX, parent);
        n += 3;
        if bus.read8(lin(es, parent.wrapping_add(0x3b))) & 0x80 != 0 {
            let s1 = bus.read32(lin(ds, SYS_S1));
            c.r[EAX] = s1;
            bus.write32(lin(es, si.wrapping_add(4)), s1);
            c.set_x(EAX, 0x0a);
            c.set_x(EDI, di);
            n += 3;
            n += call_body_name(c, bus, 0x4749)?;
            n += 1; // JMP 479b
            named_by_4db8 = true;
        } else {
            let mut bx = parent.wrapping_add(0x22);
            n += 2;
            loop {
                let al = bus.read8(lin(es, bx));
                stos(bus, &mut di, al);
                bx = bx.wrapping_add(1);
                n += 5;
                c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
                if al == 0 {
                    break;
                }
            }
            c.set_x(EBX, bx);
            let dl = bus.read8(lin(ds, 0x2034));
            c.set_x(EDX, (c.x(EDX) & 0xff00) | dl as u16);
            n += 3;
            let al;
            if dl == 0 {
                let k = bus.read8(lin(ds, 0x2031)).wrapping_add(1);
                bus.write8(lin(ds, 0x2031), k);
                let mut v = k;
                n += 4;
                if (k as i8) >= 0x0a {
                    bus.write8(lin(es, di.wrapping_sub(1)), 0x31);
                    di = di.wrapping_add(1);
                    v = k.wrapping_sub(0x0a);
                    n += 3;
                }
                al = v.wrapping_add(0x30);
                n += 2;
            } else if dl == 1 {
                let k = bus.read8(lin(ds, 0x2032)).wrapping_add(1);
                bus.write8(lin(ds, 0x2032), k);
                al = k.wrapping_add(0x60);
                n += 2 + 4;
            } else {
                let k = bus.read8(lin(ds, 0x2033)).wrapping_add(1);
                bus.write8(lin(ds, 0x2033), k);
                al = k.wrapping_add(0x30);
                n += 2 + 3;
            }
            c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
            bus.write8(lin(es, di.wrapping_sub(1)), al);
            bus.write8(lin(es, di), 0);
            n += 2;
        }
    }
    if !named_by_4db8 {
        c.set_x(EDI, di);
    }
    // 479b: POP ×4; PUSH ×4 (the same values); CALL 47d8
    restore(c, bus);
    let mut sp = c.x(ESP);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[0]);
    sp = sp.wrapping_sub(2);
    bus.write16(lin(c.ss, sp), saved[1] as u16);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[2]);
    sp = sp.wrapping_sub(4);
    bus.write32(lin(c.ss, sp), saved[3]);
    c.set_x(ESP, sp);
    n += 8;
    push16(c, bus, 0x47c4);
    n += 1;
    let m = finish_body(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    n += m + 1;
    restore(c, bus);
    n += 4;
    Some(n)
}

near_port!(setup_body_port, setup_body);

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
fn pop16(c: &mut Cpu, bus: &mut dyn Bus) -> u16 {
    let sp = c.x(ESP);
    let v = bus.read16(lin(c.ss, sp));
    c.set_x(ESP, sp.wrapping_add(2));
    v
}
fn push32(c: &mut Cpu, bus: &mut dyn Bus, v: u32) {
    let sp = c.x(ESP).wrapping_sub(4);
    bus.write32(lin(c.ss, sp), v);
    c.set_x(ESP, sp);
}
fn pop32(c: &mut Cpu, bus: &mut dyn Bus) -> u32 {
    let sp = c.x(ESP);
    let v = bus.read32(lin(c.ss, sp));
    c.set_x(ESP, sp.wrapping_add(4));
    v
}

/// `CALL 4df5` inside a routine: the step (`AX` = low word of the new `s0`).
fn step(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    1 + sys_step_inline(c, bus)
}

/// `CALL 4e29` inside a routine.
fn call_lookup(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    push16(c, bus, 0);
    let n = lookup_3c39_port(c, bus);
    1 + n
}

/// `CALL 468a` inside a routine; returns (instructions, ZF after it).
fn call_setup(c: &mut Cpu, bus: &mut dyn Bus, ret: u16) -> Option<(u64, bool)> {
    push16(c, bus, ret);
    let n = setup_body(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    Some((1 + n + 1, c.flags & 0x40 != 0))
}

/// `EL2M00:4e3f` without its `RET`, *observed*: build the bodies orbiting the current one, from
/// the inside out, recursing into each body's own satellites. A near routine.
///
/// On entry `DS:07c8` (the parent's mass scale, doubled and saturated here), `DS:07d8` (the orbit
/// position), `DS:07cc`, `DS:07bc`, `DS:07e4` and the body pointer `[DS:07dc]` describe the parent.
/// Each pass steps the system seed and moves the orbit outwards by a seeded amount from the table
/// at `EL2M00:3c39` ([`lookup_3c39_port`]), derives the next body's mass (stars above
/// `0103_6640h` may split into a binary through `EL2M00:50d6` while `DS:203a` allows), sets it up
/// ([`setup_body`]) and recurses into it with the whole working set (`DS:07c0`–`07e2`, the seeds,
/// `DS:2031`) saved on the stack. It stops when the orbit passes `FFE0h`, the orbit position
/// overflows, or 60 (3Ch) bodies exist.
fn orbits(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let mut n: u64 = 5;
    let mut ax = r16(c, bus, 0x07c8);
    let mut dx = r16(c, bus, 0x07ca);
    let v = ((dx as u32) << 16 | ax as u32) as u64 * 2;
    if v > 0xffff_ffff {
        ax = 0xffff;
        dx = 0xffff;
        n += 2;
    } else {
        ax = v as u16;
        dx = (v >> 16) as u16;
    }
    c.set_x(EAX, ax);
    c.set_x(EDX, dx);
    w16(c, bus, 0x07c8, ax);
    w16(c, bus, 0x07ca, dx);
    let v = r16(c, bus, 0x07bc);
    push16(c, bus, v);
    let bx = r16(c, bus, 0x07d8);
    c.set_x(EBX, bx);
    n += 4;
    n += call_lookup(c, bus);
    let a = c.x(EAX);
    w16(c, bus, 0x07d4, a);
    n += 1;
    // 4e66: the loop over the orbits.
    loop {
        n += step(c, bus);
        let mut ax = c.x(EAX) >> 3;
        let pos = r16(c, bus, 0x07d8);
        let mut dx = pos >> 2;
        let cx = dx >> 1;
        dx = dx.wrapping_add(cx);
        let mut bp = pos;
        c.set_x(ECX, cx);
        let top = bus.read16(lin(c.ss, c.x(ESP)));
        c.set_x(EBX, c.x(ESP));
        n += 10; // SHR; MOV; SHR; MOV; SHR; ADD; MOV BP; MOV BX,SP; ADD AX,SS:[BX]; JC
        let (sum, carry) = ax.overflowing_add(top);
        ax = sum;
        if !carry {
            let p = ax as u32 * bp as u32;
            bp = (p >> 16) as u16;
            n += 6;
        }
        c.set_x(EBP, bp);
        let (d, carry) = dx.overflowing_add(bp);
        dx = d;
        c.set_x(EAX, ax);
        c.set_x(EDX, dx);
        n += 2;
        if carry {
            // 50ca: ADD SP,2
            let sp = c.x(ESP);
            c.set_x(ESP, sp.wrapping_add(2));
            return Some(n + 1);
        }
        w16(c, bus, 0x07d0, dx);
        c.set_x(EBX, dx.wrapping_add(r16(c, bus, 0x07d8)));
        n += 3;
        n += call_lookup(c, bus);
        let a = c.x(EAX).wrapping_sub(r16(c, bus, 0x07d4));
        c.set_x(EAX, a);
        w16(c, bus, 0x07c4, a);
        n += 2;
        n += step(c, bus);
        let mut ax = c.x(EAX);
        let p = ax as u32 * ax as u32;
        ax = (p >> 16) as u16;
        let p = ax as u32 * ax as u32;
        ax = (p >> 16) as u16;
        let p = ax as u32 * r16(c, bus, 0x07c4) as u32;
        ax = (p >> 16) as u16;
        let v = r16(c, bus, 0x07d4).wrapping_add(ax);
        w16(c, bus, 0x07d4, v);
        // PUSH AX; MUL [7cc]; POP AX; PUSH CX; PUSH DX; MUL [7ce]; POP CX; ADD; ADC; POP CX
        let lo = ax as u32 * r16(c, bus, 0x07cc) as u32;
        let hi = ax as u32 * r16(c, bus, 0x07ce) as u32;
        let sum = hi.wrapping_add(lo >> 16);
        let mut ax2 = sum as u16;
        let mut dx2 = (sum >> 16) as u16;
        let bxv = r16(c, bus, 0x07d8);
        let mut cxv = r16(c, bus, 0x07d0);
        c.set_x(EBX, bxv);
        n += 6 + 11 + 2; // MULs; ADD [7d4] … POP CX; MOV BX,[7d8]; MOV CX,[7d0]
        let (np, carry) = bxv.overflowing_add(cxv);
        w16(c, bus, 0x07d8, np);
        n += 2;
        c.set_x(EAX, ax2);
        c.set_x(EDX, dx2);
        c.set_x(ECX, cxv);
        if carry {
            let sp = c.x(ESP);
            c.set_x(ESP, sp.wrapping_add(2));
            return Some(n + 1);
        }
        cxv >>= 1;
        c.set_x(ECX, cxv);
        w16(c, bus, 0x07d0, cxv);
        let bxv = bxv.wrapping_add(cxv);
        c.set_x(EBX, bxv);
        w16(c, bus, 0x07c0, bxv);
        n += 4;
        // CMP DX,0; JC (never); JA 4efa; CMP AX,20h; JC 50c1
        n += 3;
        let mut tiny = false;
        if dx2 == 0 {
            n += 2;
            if ax2 < 0x20 {
                tiny = true;
            }
        }
        if !tiny {
            // 4efa: CMP DX,0; JA 4f12; JC (never); CMP AX,3A98h; JNC 4f12
            n += 2;
            if dx2 == 0 {
                n += 3;
                if ax2 < 0x3a98 {
                    for _ in 0..3 {
                        let c0 = dx2 & 1;
                        dx2 >>= 1;
                        ax2 = (ax2 >> 1) | (c0 << 15);
                    }
                    n += 6;
                }
            }
            c.set_x(EAX, ax2);
            c.set_x(EDX, dx2);
            // 4f12: PUSH [2037]
            let v = r16(c, bus, 0x2037);
            push16(c, bus, v);
            n += 1 + 2;
            let big = dx2 > 0x103 || (dx2 == 0x103 && ax2 >= 0x6640);
            if dx2 >= 0x103 {
                n += 1;
                if dx2 == 0x103 {
                    n += 2;
                }
            }
            let mut made_binary = false;
            if big {
                n += 2;
                if bus.read8(lin(c.ds, 0x203a)) != 0 {
                    let mut bx = r16(c, bus, 0x203b);
                    let mut cx = r16(c, bus, 0x203d);
                    let c0 = cx & 1;
                    cx >>= 1;
                    bx = (bx >> 1) | (c0 << 15);
                    c.set_x(EBX, bx);
                    c.set_x(ECX, cx);
                    n += 6;
                    let below = dx2 < cx || (dx2 == cx && ax2 < bx);
                    if dx2 >= cx {
                        n += 1;
                        if dx2 == cx {
                            n += 2;
                        }
                    }
                    if !below {
                        dx2 = cx;
                        ax2 = bx;
                        n += 2;
                    }
                    let k = bus.read8(lin(c.ds, 0x203a)).wrapping_sub(1);
                    bus.write8(lin(c.ds, 0x203a), k);
                    n += 2;
                    c.set_x(EAX, ax2);
                    c.set_x(EDX, dx2);
                    if k != 0 {
                        // A binary pair through 50d6.
                        push16(c, bus, ax2);
                        push16(c, bus, dx2);
                        n += 2;
                        n += step(c, bus);
                        let bx = c.x(EBX);
                        let bx = (bx >> 8) >> 3;
                        c.set_x(EBX, bx);
                        w16(c, bus, 0x07c4, bx);
                        let dxp = pop16(c, bus);
                        let axp = pop16(c, bus);
                        c.set_x(EDX, dxp);
                        c.set_x(EAX, axp);
                        push16(c, bus, axp);
                        push16(c, bus, dxp);
                        let v = r16(c, bus, 0x07c0);
                        push16(c, bus, v);
                        let v = r16(c, bus, 0x202d);
                        push16(c, bus, v);
                        n += 3 + 2 + 2 + 2 + 1;
                        push16(c, bus, 0x4f69);
                        let m = binary(c, bus)?;
                        let sp = c.x(ESP);
                        c.set_x(ESP, sp.wrapping_add(2));
                        n += 1 + m + 1; // CALL, the routine, its RET
                        w16(c, bus, 0x07e4, 0x3000);
                        let a = pop16(c, bus).wrapping_add(1);
                        c.set_x(EAX, a);
                        w16(c, bus, 0x2037, a);
                        let v = pop16(c, bus);
                        w16(c, bus, 0x07c0, v);
                        dx2 = pop16(c, bus);
                        ax2 = pop16(c, bus);
                        for _ in 0..2 {
                            let c0 = dx2 & 1;
                            dx2 >>= 1;
                            ax2 = (ax2 >> 1) | (c0 << 15);
                        }
                        c.set_x(EAX, ax2);
                        c.set_x(EDX, dx2);
                        let k = bus.read8(lin(c.ds, 0x203a)).wrapping_sub(1);
                        bus.write8(lin(c.ds, 0x203a), k);
                        n += 13; // MOV [7e4]; POP AX; INC; MOV [2037]; POP [7c0]; POP DX; POP AX; 2× SHR/RCR; DEC [203a]; JMP
                        made_binary = true;
                    }
                } else {
                    // 4f88: CALL 4df5; MOV DX,103h; MUL DX
                    n += step(c, bus);
                    let p = c.x(EAX) as u32 * 0x103;
                    ax2 = p as u16;
                    dx2 = (p >> 16) as u16;
                    c.set_x(EAX, ax2);
                    c.set_x(EDX, dx2);
                    n += 2;
                }
            }
            if !made_binary {
                // 4f90
                let cnt = r16(c, bus, 0x202d);
                c.set_x(ECX, cnt);
                w16(c, bus, 0x2035, cnt);
                n += 4;
                if (cnt as i16) >= 0x3c {
                    // 50ce: POP [2037]; ADD SP,2
                    let v = pop16(c, bus);
                    w16(c, bus, 0x2037, v);
                    let sp = c.x(ESP);
                    c.set_x(ESP, sp.wrapping_add(2));
                    return Some(n + 2);
                }
                w16(c, bus, 0x07bc, ax2);
                w16(c, bus, 0x07be, dx2);
                let (off, seg) = (r16(c, bus, 0x07dc), r16(c, bus, 0x07de));
                c.set_x(ESI, off);
                c.es = seg;
                n += 3;
                let (m, zf) = call_setup(c, bus, 0x4fad)?;
                n += m;
                let si = c.x(ESI);
                w16(c, bus, 0x07dc, si);
                n += 2;
                if zf {
                    // 50bd: POP [2037]
                    let v = pop16(c, bus);
                    w16(c, bus, 0x2037, v);
                    n += 1;
                    n += 2;
                    if r16(c, bus, 0x07d4) < 0xffe0 {
                        continue;
                    }
                    let sp = c.x(ESP);
                    c.set_x(ESP, sp.wrapping_add(2));
                    return Some(n + 1);
                }
                ax2 = r16(c, bus, 0x07bc);
                dx2 = r16(c, bus, 0x07be);
                let b = r16(c, bus, 0x2035).wrapping_add(1);
                c.set_x(EBX, b);
                w16(c, bus, 0x2037, b);
                let v = r16(c, bus, 0x07dc).wrapping_add(0x40);
                w16(c, bus, 0x07dc, v);
                c.set_x(EAX, ax2);
                c.set_x(EDX, dx2);
                n += 6;
            }
            // 4fca: save the working set and recurse.
            push16(c, bus, ax2);
            push16(c, bus, dx2);
            for off in [0x07c0u16, 0x07c4] {
                let v = r16(c, bus, off);
                push16(c, bus, v);
            }
            for off in [0x07c8u16, 0x07cc] {
                let v = r32(c, bus, off);
                push32(c, bus, v);
            }
            for off in [0x07d0u16, 0x07d4, 0x07d8] {
                let v = r16(c, bus, off);
                push16(c, bus, v);
            }
            let v = r32(c, bus, 0x07e0);
            push32(c, bus, v);
            n += 10;
            // CMP DX,0; JA 5007; JC (never); CMP AX,3A98h; JNC 5007; 4ffb: (DX:AX) <<= 3
            n += 2;
            if dx2 == 0 {
                n += 3;
                if ax2 < 0x3a98 {
                    let v = ((dx2 as u32) << 16 | ax2 as u32) << 3;
                    ax2 = v as u16;
                    dx2 = (v >> 16) as u16;
                    n += 6;
                }
            }
            w16(c, bus, 0x07cc, ax2);
            w16(c, bus, 0x07ce, dx2);
            let bx = r16(c, bus, 0x2037)
                .wrapping_sub(r16(c, bus, 0x202d))
                .wrapping_shl(6)
                .wrapping_add(r16(c, bus, 0x07dc))
                .wrapping_sub(0x40);
            w16(c, bus, 0x07e0, bx);
            let v = r16(c, bus, 0x07de);
            w16(c, bus, 0x07e2, v);
            let v = r16(c, bus, 0x07e4);
            w16(c, bus, 0x07d8, v);
            c.set_x(EBX, v);
            for off in [SYS_S0, SYS_S1, 0x2031] {
                let v = r32(c, bus, off);
                push32(c, bus, v);
            }
            let k = bus.read8(lin(c.ds, 0x2034)).wrapping_add(1);
            bus.write8(lin(c.ds, 0x2034), k);
            n += 2 + 10 + 3 + 1 + 2; // MOV [7cc]/[7ce]; the parent pointer (10); 3 PUSH dword; INC [2034]; CMP DX; JC
            let big = dx2 > 0x103 || (dx2 == 0x103 && ax2 >= 0x6640);
            if dx2 >= 0x103 {
                n += 1;
                if dx2 == 0x103 {
                    n += 2;
                }
            }
            if big {
                w32(c, bus, 0x2031, 0);
                n += 1;
            }
            // 505d: EAX = [ESP+1Ah] (the pushed DS:07c8) >> 6, at least DS:07cc
            if c.r[ESP] >> 16 != 0 {
                return None;
            }
            let parent = bus.read32(lin(c.ss, c.x(ESP).wrapping_add(0x1a))) >> 6;
            let cc = r32(c, bus, 0x07cc);
            let m = if cc > parent { parent } else { cc };
            c.r[EAX] = m;
            w32(c, bus, 0x07c8, m);
            n += 5;
            if cc <= parent {
                n += 2;
            }
            w32(c, bus, 0x07cc, cc >> 7);
            n += 1;
            n += step(c, bus);
            let a = c.x(EAX);
            w16(c, bus, 0x07bc, a);
            n += 1;
            push16(c, bus, 0x5089);
            let m = orbits(c, bus)?;
            let sp = c.x(ESP);
            c.set_x(ESP, sp.wrapping_add(2));
            n += 1 + m + 1;
            for off in [0x2031u16, SYS_S1, SYS_S0, 0x07e0] {
                let v = pop32(c, bus);
                w32(c, bus, off, v);
            }
            for off in [0x07d8u16, 0x07d4, 0x07d0] {
                let v = pop16(c, bus);
                w16(c, bus, off, v);
            }
            for off in [0x07ccu16, 0x07c8] {
                let v = pop32(c, bus);
                w32(c, bus, off, v);
            }
            for off in [0x07c4u16, 0x07c0] {
                let v = pop16(c, bus);
                w16(c, bus, off, v);
            }
            let d = pop16(c, bus);
            let a = pop16(c, bus);
            c.set_x(EDX, d);
            c.set_x(EAX, a);
            // 50bd: POP [2037]
            let v = pop16(c, bus);
            w16(c, bus, 0x2037, v);
            n += 13 + 1;
        }
        // 50c1: CMP [7d4],FFE0h; JC 4e66
        n += 2;
        if r16(c, bus, 0x07d4) < 0xffe0 {
            continue;
        }
        let sp = c.x(ESP);
        c.set_x(ESP, sp.wrapping_add(2));
        return Some(n + 1);
    }
}

near_port!(orbits_port, orbits);

/// `EL2M00:50d6` without its `RET`, *observed*: a star of mass `DX:AX` that may be a binary.
/// Masses below `FFFF_FFEFh` but outside `0A21_FE80h`–`0A6E_49BFh` make one star at `[DS:07dc]`
/// and a companion of a quarter of the mass, both through [`setup_body`]. Inside that band (and
/// above it) the pair becomes a system: a barycentre body (type 20h) named after the system with
/// `", A"`/`"B"` letters from `DS:2030`, whose two stars share the mass in a seeded ratio. `DS:2037`,
/// `[DS:07e0]`, `DS:07c4` and `DS:07c0` are restored on the way out.
fn binary(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    let v = r16(c, bus, 0x2037);
    push16(c, bus, v);
    for off in [0x07e0u16, 0x07c4, 0x07c0] {
        let v = r32(c, bus, off);
        push32(c, bus, v);
    }
    let (ax, dx) = (c.x(EAX), c.x(EDX));
    let m = (dx as u32) << 16 | ax as u32;
    let mut n: u64 = 4;
    // CMP DX,-1; JA 5259; JNZ 50f9; CMP AX,-11h; JNC 5259
    n += 2;
    let mut single = false;
    if dx == 0xffff {
        n += 3;
        if ax >= 0xffef {
            single = true;
        }
    } else {
        n += 1;
    }
    if !single {
        // CMP DX,0A21h; JC 5117; JNZ 5106; CMP AX,FE80h; JC 5117
        n += 2;
        let below = m < 0x0a21_fe80;
        if dx >= 0x0a21 {
            n += 1;
            if dx == 0x0a21 {
                n += 2;
            }
        }
        if !below {
            // 5106: CMP DX,0A6Eh; JC 5259; JNZ 5117; CMP AX,49C0h; JC 5259
            n += 2;
            if dx < 0x0a6e {
                single = true;
            } else {
                n += 1;
                if dx == 0x0a6e {
                    n += 2;
                    if ax < 0x49c0 {
                        single = true;
                    }
                }
            }
        }
    }
    let restore = |c: &mut Cpu, bus: &mut dyn Bus| {
        for off in [0x07c0u16, 0x07c4, 0x07e0] {
            let v = pop32(c, bus);
            w32(c, bus, off, v);
        }
        let v = pop16(c, bus);
        w16(c, bus, 0x2037, v);
    };
    if single {
        // 5259: two bodies, the second with a quarter of the mass.
        push16(c, bus, dx);
        push16(c, bus, ax);
        w16(c, bus, 0x07bc, ax);
        w16(c, bus, 0x07be, dx);
        let (off, seg) = (r16(c, bus, 0x07dc), r16(c, bus, 0x07de));
        c.set_x(ESI, off);
        c.es = seg;
        n += 5;
        n += call_setup(c, bus, 0x5269)?.0;
        let si = c.x(ESI).wrapping_add(0x40);
        c.set_x(ESI, si);
        w16(c, bus, 0x07dc, si);
        let k = r16(c, bus, 0x202d);
        w16(c, bus, 0x2037, k);
        let a = pop16(c, bus);
        let d = pop16(c, bus);
        let q = ((d as u32) << 16 | a as u32) >> 2;
        w16(c, bus, 0x07bc, q as u16);
        w16(c, bus, 0x07be, (q >> 16) as u16);
        if c.r[ESP] >> 16 != 0 {
            return None;
        }
        let v = bus.read16(lin(c.ss, c.x(ESP).wrapping_add(4)));
        w16(c, bus, 0x07c0, v);
        c.set_x(EAX, v);
        c.set_x(EDX, (q >> 16) as u16);
        n += 4 + 2 + 4 + 2 + 2;
        n += call_setup(c, bus, 0x5292)?.0;
        let si = c.x(ESI).wrapping_add(0x40);
        c.set_x(ESI, si);
        w16(c, bus, 0x07dc, si);
        // POP dword [7c0], [7c4], [7e0]; POP [2037]
        restore(c, bus);
        n += 2 + 4;
        return Some(n);
    }
    // 5117: a barycentre and two stars.
    let (off, seg) = (r16(c, bus, 0x07dc), r16(c, bus, 0x07de));
    c.set_x(ESI, off);
    c.es = seg;
    bus.write16(lin(seg, off.wrapping_add(0x0a)), 0);
    bus.write16(lin(seg, off.wrapping_add(0x0c)), 0x20);
    w16(c, bus, 0x07e8, 0);
    w16(c, bus, 0x07bc, ax);
    w16(c, bus, 0x07be, dx);
    let saved = [r32(c, bus, 0x07bc), r16(c, bus, 0x07c0) as u32, r32(c, bus, 0x07c8), r32(c, bus, 0x07e0)];
    push32(c, bus, saved[0]);
    push16(c, bus, saved[1] as u16);
    push32(c, bus, saved[2]);
    push32(c, bus, saved[3]);
    n += 10;
    push16(c, bus, 0x514a);
    let m = finish_body(c, bus)?;
    let sp = c.x(ESP);
    c.set_x(ESP, sp.wrapping_add(2));
    n += 1 + m + 1;
    let si = c.x(ESI);
    w16(c, bus, 0x07dc, si);
    let v = pop32(c, bus);
    w32(c, bus, 0x07e0, v);
    let v = pop32(c, bus);
    w32(c, bus, 0x07c8, v);
    let v = pop16(c, bus);
    w16(c, bus, 0x07c0, v);
    let v = pop32(c, bus);
    w32(c, bus, 0x07bc, v);
    let (off, seg) = (r16(c, bus, 0x07dc), r16(c, bus, 0x07de));
    c.set_x(ESI, off);
    c.es = seg;
    // Name: the system name (at most 12 bytes) then "." or a space, then ", A"/"B" letters.
    let mut bx = off.wrapping_add(0x22);
    let (noff, nseg) = (r16(c, bus, 0x2029), r16(c, bus, 0x202b));
    let mut di = noff;
    let mut cx: u16 = 0x0c;
    n += 1 + 4 + 1 + 1 + 1 + 1 + 1;
    let mut al;
    loop {
        al = bus.read8(lin(nseg, di));
        bus.write8(lin(seg, bx), al);
        di = di.wrapping_add(1);
        bx = bx.wrapping_add(1);
        cx = cx.wrapping_sub(1);
        n += 6;
        if !(cx != 0 && al != 0) {
            break;
        }
    }
    c.set_x(EDI, di);
    c.set_x(ECX, cx);
    n += 2; // POP DS; JZ
    if al == 0 {
        bus.write8(lin(seg, bx.wrapping_sub(1)), 0x20);
        n += 1;
    } else {
        bus.write8(lin(seg, bx), 0x2e);
        bx = bx.wrapping_add(1);
        n += 3;
    }
    let l = bus.read8(lin(c.ds, 0x2030)).wrapping_add(0x41);
    bus.write16(lin(seg, bx), 0x2c00 | l as u16);
    bus.write16(lin(seg, bx.wrapping_add(2)), l.wrapping_add(1) as u16);
    c.set_x(EBX, bx);
    let v = r16(c, bus, 0x07dc).wrapping_add(0x40);
    w16(c, bus, 0x07dc, v);
    let k = r16(c, bus, 0x202d);
    w16(c, bus, 0x2037, k);
    c.set_x(EAX, k);
    n += 10; // MOV AL; ADD; MOV AH; MOV [BX]; INC AL; XOR AH; MOV [BX+2]; ADD [7dc]; MOV AX,[202d]; MOV [2037]
    // The mass split: PUSH dword [7bc]; a seeded fraction of the mass, at least 0103_6640h.
    let v = r32(c, bus, 0x07bc);
    push32(c, bus, v);
    n += 1;
    n += step(c, bus);
    let mut a = c.x(EAX) | 0x8000;
    a = ((a as i16) >> 2) as u16;
    let p = a as u32 * r16(c, bus, 0x07be) as u32;
    let (mut lo, mut hi) = (p as u16, (p >> 16) as u16);
    n += 7; // OR AH; SAR; MUL; MOV CX; MOV BX; CMP DX,CX; JA
    let below = hi < 0x103 || (hi == 0x103 && lo < 0x6640);
    if hi <= 0x103 {
        n += 1; // JC
        if hi == 0x103 {
            n += 2; // CMP AX,BX; JNC
        }
    }
    if below {
        hi = 0x103;
        lo = 0x6640;
        n += 2;
    }
    c.set_x(ECX, 0x103);
    c.set_x(EBX, 0x6640);
    push16(c, bus, hi);
    push16(c, bus, lo);
    let total = ((hi as u32) << 16 | lo as u32) as u64 + r32(c, bus, 0x07bc) as u64;
    let (mut ta, mut td) = (total as u16, (total >> 16) as u16);
    n += 5; // PUSH DX; PUSH AX; ADD; ADC; JNC
    if total > 0xffff_ffff {
        ta = 0xffff;
        td = 0xffff;
        n += 2;
    }
    c.set_x(EAX, ta);
    c.set_x(EDX, td);
    n += m3(c, bus, 0x02c4, 0x51e2)?;
    let b = c.x(EBX).wrapping_add(0x3f);
    c.set_x(EBX, b);
    let (off, seg) = (r16(c, bus, 0x07dc), r16(c, bus, 0x07de));
    c.set_x(ESI, off);
    c.es = seg;
    bus.write16(lin(seg, off.wrapping_sub(0x40)), c.x(EDX));
    bus.write16(lin(seg, off.wrapping_sub(0x3e)), b);
    if c.r[ESP] >> 16 != 0 {
        return None;
    }
    let sp = c.x(ESP);
    let mass = bus.read32(lin(c.ss, sp.wrapping_add(4)));
    w32(c, bus, 0x07bc, mass);
    let v = bus.read16(lin(c.ss, sp.wrapping_add(0x0c)));
    w16(c, bus, 0x07c0, v);
    c.r[EAX] = (mass & 0xffff_0000) | v as u32;
    n += 8; // ADD BX; LES; MOV [SI-40h]; MOV [SI-3Eh]; MOV EAX,[ESP+4]; MOV [7bc]; MOV AX,[ESP+0Ch]; MOV [7c0]
    let (m, _) = call_setup(c, bus, 0x5206)?;
    n += m;
    let si = c.x(ESI);
    let es = c.es;
    bus.write16(lin(es, si.wrapping_add(0x12)), 0);
    let a = bus.read16(lin(es, si.wrapping_add(0x18)));
    let d = bus.read16(lin(es, si.wrapping_add(0x1a)));
    bus.write16(lin(es, si.wrapping_sub(0x28)), a);
    bus.write16(lin(es, si.wrapping_sub(0x26)), d);
    let si = si.wrapping_add(0x40);
    c.set_x(ESI, si);
    w16(c, bus, 0x07dc, si);
    let v = bus.read16(lin(c.ss, c.x(ESP).wrapping_add(0x0c)));
    w16(c, bus, 0x07c0, v);
    c.set_x(EAX, v);
    c.set_x(EDX, d);
    let v = pop32(c, bus);
    w32(c, bus, 0x07bc, v);
    n += 10;
    let (m, _) = call_setup(c, bus, 0x5233)?;
    n += m;
    let si = c.x(ESI);
    bus.write16(lin(c.es, si.wrapping_add(0x12)), 0x7fff);
    let si = si.wrapping_add(0x40);
    c.set_x(ESI, si);
    w16(c, bus, 0x07dc, si);
    let v = pop32(c, bus);
    w32(c, bus, 0x07bc, v);
    restore(c, bus);
    n += 3 + 1 + 4;
    Some(n)
}

near_port!(binary_port, binary);


near_port!(classify_4990_port, classify_4990);
near_port!(classify_4abb_port, classify_4abb);
near_port!(classify_4b69_port, classify_4b69);
near_port!(classify_4bad_port, classify_4bad);

/// Ports of `EL2M00:4bff`, `4c27` and `4c2c` (entered by a near `CALL`).
pub fn classify_4bff_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let n = classify_4bff(c, bus);
    near_ret(c, bus);
    n + 1
}

pub fn classify_4c27_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let n = classify_4c27(c, bus);
    near_ret(c, bus);
    n + 1
}

pub fn classify_4c2c_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let n = classify_4c2c(c, bus);
    near_ret(c, bus);
    n + 1
}

/// Every ported routine of `EL2M00` (near routines: slot 0).
pub const PORTS_M00: &[(u16, u16, &str, Port)] = &[
    (0x4990, 0, "classify_4990", classify_4990_port),
    (0x4e3f, 0, "orbits", orbits_port),
    (0x50d6, 0, "binary", binary_port),
    (0x4abb, 0, "classify_4abb", classify_4abb_port),
    (0x4b69, 0, "classify_4b69", classify_4b69_port),
    (0x4bad, 0, "classify_4bad", classify_4bad_port),
    (0x4bff, 0, "classify_4bff", classify_4bff_port),
    (0x4c27, 0, "classify_4c27", classify_4c27_port),
    (0x4c2c, 0, "classify_4c2c", classify_4c2c_port),
    (0x468a, 0, "setup_body", setup_body_port),
    (0x47d8, 0, "finish_body", finish_body_port),
    (0x4bda, 0, "body_4bda", body_4bda_port),
    (0x4c32, 0, "moons", moons_port),
    (0x4d10, 0, "big_body", big_body_port),
    (0x4db8, 0, "body_name", body_name_port),
    (0x4df5, 0, "sys_step", sys_step_port),
    (0x4e29, 0, "lookup_3c39", lookup_3c39_port),
];
