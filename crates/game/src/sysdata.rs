//! Per-system data (`EL2M06`, docs/ECONOMY.md): what a star system is like, from its 32-bit id.
//!
//! Hand-made systems have their own 30h-byte record in `EL2M06`'s table (from `EL2M06:01f3`, ended
//! by an id of 0); every other system takes one of seven template records by its squared distance
//! from Sol's sector, so the inhabited core thins out towards the frontier. The port is register-
//! and memory-exact and returns the original's instruction count; the records are read from the
//! user's `EL2M06.OVL` in memory.

use crate::math3::{lin, Bus, Cpu, Port, EAX, EBX, ECX, EDI, EDX, ESI};

/// Template records by squared distance from Sol: (at least this, record) in the order tested.
pub const TEMPLATES: [(u16, u16); 6] = [(0x1fa4, 0x00a3), (0x00e1, 0x00d3), (0x0064, 0x0103), (0x0031, 0x0133), (0x0019, 0x0163), (0x0009, 0x0193)];
/// The record for the systems nearest Sol.
pub const CORE: u16 = 0x01c3;

/// The template a procedural system takes (`EL2M06:3f8f`), *observed*: `x` = id bits 6–18, `y` =
/// id >> 19, a jitter `j` = id & 7; `d = (|x − 1718h| + j)² + (|y − 1524h| + j)²` in 16 bits.
/// Returns (record, `d`, and the high word of the second square, left in `DX`).
pub fn template(id: u32) -> (u16, u16, u16) {
    let j = (id & 7) as u16;
    let x = ((id >> 6) as u16) & 0x1fff;
    let y = (id >> 19) as u16;
    let dx = (x.wrapping_sub(0x1718) as i16).unsigned_abs().wrapping_add(j);
    let dy = (y.wrapping_sub(0x1524) as i16).unsigned_abs().wrapping_add(j);
    let sx = (dx as i16 as i32) * (dx as i16 as i32);
    let sy = (dy as i16 as i32) * (dy as i16 as i32);
    let d = (sx as u16).wrapping_add(sy as u16);
    let rec = TEMPLATES.iter().find(|t| d >= t.0).map(|t| t.1).unwrap_or(CORE);
    (rec, d, (sy >> 16) as u16)
}

/// Port of `EL2M06:3eb9` (module 6, handler `+20`), *observed*: look up system `DS:07c0` (dword)
/// in the hand-made records, else its distance template ([`template`]), then by `AL`:
/// - 0: `DS:07c0`/`07c4` = the record's `+2Ch`/`+2Eh` words, `DS:07bc`, `07c8`, `07cc`, `07d0`,
///   `07d4`, `07d8` = its bytes `+5`…`+0Ah`;
/// - 1: `AX` = byte `+4`, `BX` = byte `+0Bh`, `DS:07c4` = byte `+0Ah`, `ES:DI` = `ES:SI` = the record
///   `+0Ch`;
/// - other: `DS:07bc` = low and `DS:07c0` = high nibble of byte `+2Bh`, `DS:07c4` = the low
///   nibble, `DS:07c8` = byte `+0Ah` >> 6, `DS:07d8`/`07d4` = `DS:20f1`/`20f3`, swapped unless the
///   low nibble is above the high one (or equal with `DS:07c8` = 2), and `DS:07c4` = the high
///   nibble when it is the larger.
pub fn system_data_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let cs = c.cs;
    let ds = c.ds;
    let ax0 = c.x(EAX);
    c.set_x(EBX, cs);
    c.es = cs;
    let id = bus.read32(lin(ds, 0x07c0));
    let mut si: u16 = 0x01c3;
    let mut n: u64 = 3;
    let mut found = false;
    loop {
        si = si.wrapping_add(0x30);
        let e = bus.read32(lin(cs, si));
        c.r[ECX] = e;
        if e == 0 {
            n += 4;
            break;
        }
        n += 6;
        if e == id {
            found = true;
            break;
        }
    }
    if !found {
        let (rec, d, dxh) = template(id);
        let j = (id & 7) as u16;
        let x = ((id >> 6) as u16) & 0x1fff;
        let y = (id >> 19) as u16;
        let negx = (x.wrapping_sub(0x1718) as i16) < 0;
        let negy = (y.wrapping_sub(0x1524) as i16) < 0;
        c.set_x(ECX, d);
        c.set_x(EDX, dxh);
        c.set_x(ESI, j);
        n += 33 + negx as u64 + negy as u64;
        let k = TEMPLATES.iter().position(|t| d >= t.0);
        n += 3 * (k.map(|k| k + 1).unwrap_or(TEMPLATES.len())) as u64;
        if k.is_none() {
            n += 2; // MOV SI,1C3h; JMP 3ed5
        }
        si = rec;
        c.set_x(EAX, ax0); // POP AX
    }
    c.set_x(ESI, si);
    let es = c.es;
    let b = |bus: &mut dyn Bus, o: u16| bus.read8(lin(es, si.wrapping_add(o))) as u16;
    let w16 = |bus: &mut dyn Bus, off: u16, v: u16| bus.write16(lin(ds, off), v);
    let al = ax0 as u8;
    n += 2; // OR AL,AL; JNZ
    if al == 0 {
        let v = bus.read16(lin(es, si.wrapping_add(0x2c)));
        w16(bus, 0x07c0, v);
        let v = bus.read16(lin(es, si.wrapping_add(0x2e)));
        w16(bus, 0x07c4, v);
        let mut last = 0;
        for (k, off) in [0x07bcu16, 0x07c8, 0x07cc, 0x07d0, 0x07d4, 0x07d8].iter().enumerate() {
            last = b(bus, 5 + k as u16);
            w16(bus, *off, last);
        }
        c.set_x(EAX, last);
        return ret(c, bus, n + 18);
    }
    n += 2; // CMP AL,1; JNZ
    if al == 1 {
        c.set_x(EAX, b(bus, 4));
        c.set_x(EBX, b(bus, 0x0b));
        let dl = b(bus, 0x0a);
        c.set_x(EDX, dl);
        w16(bus, 0x07c4, dl);
        let si2 = si.wrapping_add(0x0c);
        c.set_x(ESI, si2);
        c.set_x(EDI, si2);
        return ret(c, bus, n + 10);
    }
    c.set_x(EDI, si);
    let byte = b(bus, 0x2b);
    let (lo, hi) = (byte & 0x0f, byte >> 4);
    c.set_x(EBX, hi);
    w16(bus, 0x07bc, lo);
    w16(bus, 0x07c0, hi);
    w16(bus, 0x07c4, lo);
    let cls = b(bus, 0x0a) >> 6;
    w16(bus, 0x07c8, cls);
    let a = bus.read16(lin(ds, 0x20f1));
    w16(bus, 0x07d8, a);
    let d4 = bus.read16(lin(ds, 0x20f3));
    w16(bus, 0x07d4, d4);
    c.set_x(EAX, hi);
    n += 18; // MOV DI … CMP [7bc],AX; JZ
    let swap = |bus: &mut dyn Bus| {
        let d8 = bus.read16(lin(ds, 0x07d8));
        let d4 = bus.read16(lin(ds, 0x07d4));
        bus.write16(lin(ds, 0x07d8), d4);
        bus.write16(lin(ds, 0x07d4), d8);
        d8
    };
    if lo == hi {
        n += 2; // CMP [7c8],2; JZ
        if cls == 2 {
            return ret(c, bus, n + 1);
        }
        let v = swap(bus);
        c.set_x(EAX, v);
        return ret(c, bus, n + 4);
    }
    n += 1; // JA
    if lo > hi {
        return ret(c, bus, n + 1);
    }
    w16(bus, 0x07c4, hi);
    let v = swap(bus);
    c.set_x(EAX, v);
    ret(c, bus, n + 5)
}

/// `RETF`, counted.
fn ret(c: &mut Cpu, bus: &mut dyn Bus, n: u64) -> u64 {
    c.retf(bus);
    n
}

/// Every ported routine of `EL2M06`.
pub const PORTS_M06: &[(u16, u16, &str, Port)] = &[(0x3eb9, 0, "system_data", system_data_port)];
