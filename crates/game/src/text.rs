//! The game's text: the main program's expander and the modules' string handlers (docs/TEXT.md).
//!
//! Every module keeps its strings behind a table of word offsets (relative to the table) and
//! answers text ids from `8000h` up through its handler `+28`. Most handlers copy the string with
//! the main program's expander `1000:e969` (slot `DS:005c`). The ports are register- and
//! memory-exact and return the original's instruction counts; strings are read from the user's
//! files in memory.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDI, EDX, ESI};

/// The expander's loop (`1000:e96e`), from `ES:SI` to `SS:DI`, through its `RETF`. An `FFh` escape
/// expands its text id in place through [`dispatch`]. Returns the instructions executed, or `None`
/// where the dispatcher declines. See docs/TEXT.md for the byte codes.
pub fn expand_loop(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    expand_loop_at(c, bus, 0)
}

/// Nesting deeper than this (strings expanding strings) is left to the original: real text nests
/// a level or two, and garbage tables would otherwise recurse without end.
const MAX_DEPTH: u32 = 16;

fn expand_loop_at(c: &mut Cpu, bus: &mut dyn Bus, depth: u32) -> Option<u64> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut n = 0u64;
    loop {
        let si = c.x(ESI);
        let al = bus.read8(lin(c.es, si));
        c.set_x(ESI, si.wrapping_add(1));
        c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
        let di = c.x(EDI);
        bus.write8(lin(c.ss, di), al);
        c.set_x(EDI, di.wrapping_add(1));
        if (al as i8) > 0 {
            n += 5;
            continue;
        }
        if al == 0 {
            return Some(n + 7);
        }
        // DEC DI (undone below for ordinary bytes)
        if al < 0xfd {
            n += 11;
            continue;
        }
        let di = c.x(EDI).wrapping_sub(1);
        c.set_x(EDI, di);
        let si = c.x(ESI);
        if al == 0xff {
            // e9a7: LODSW; XCHG AH,AL; CALLF [0014]; then DEC DI; JMP e96e
            let w = bus.read16(lin(c.es, si));
            c.set_x(ESI, si.wrapping_add(2));
            c.set_x(EAX, w.swap_bytes());
            n += 14;
            n += dispatch_at(c, bus, depth + 1)?;
            let di = c.x(EDI).wrapping_sub(1);
            c.set_x(EDI, di);
            n += 2;
            continue;
        }
        if al == 0xfe {
            bus.write8(lin(c.ss, di), 0x1f);
            let w = bus.read16(lin(c.es, si));
            c.set_x(ESI, si.wrapping_add(2));
            c.set_x(EAX, w);
            bus.write16(lin(c.ss, di.wrapping_add(1)), w);
            c.set_x(EDI, di.wrapping_add(3));
        } else {
            bus.write8(lin(c.ss, di), 0x1e);
            let b = bus.read8(lin(c.es, si));
            c.set_x(ESI, si.wrapping_add(1));
            c.set_x(EAX, (c.x(EAX) & 0xff00) | b as u16);
            bus.write8(lin(c.ss, di.wrapping_add(1)), b);
            c.set_x(EDI, di.wrapping_add(2));
        }
        n += 17;
    }
}

/// `1000:e969` (slot `DS:005c`), *observed*: copy string `AX/2` of the table at `ES:SI` (word
/// offsets relative to the table) to `SS:DI`. `BX` = `AX`. Through its `RETF`.
pub fn expand(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    expand_at(c, bus, 0)
}

fn expand_at(c: &mut Cpu, bus: &mut dyn Bus, depth: u32) -> Option<u64> {
    let ax = c.x(EAX);
    c.set_x(EBX, ax);
    let si = c.x(ESI);
    let rel = bus.read16(lin(c.es, ax.wrapping_add(si)));
    c.set_x(ESI, si.wrapping_add(rel));
    Some(2 + expand_loop_at(c, bus, depth)?)
}

/// The main program's code segment, from the main data segment (`DS` = load segment + 1615h,
/// docs/OVERLAYS.md §2).
fn main_cs(c: &Cpu) -> u16 {
    c.ds.wrapping_sub(0x1615)
}

/// `1000:e89d` (slot `DS:0014`) as entered by a far call, through its `RETF`, *observed*: expand
/// text id `AX` to `SS:DI`. Saves `SI` and `ES`. Below `3000h`: the main table at `1000:e4dd`
/// (`DX` = the main code segment). `3000h`–`3FFFh`: a formatter through `CS:[8915h + 2·id]`,
/// with `DS:07c0`–`07cb` saved around it; only `3016h` (`1000:eaf3`, the string at `[DS:07e4]`)
/// is ported. `4000h`–`7FFFh`: the table at `[DS:5703]`. `8000h` up: handler `+28` of module
/// slot `id·2 >> 10`, with the slot's lock byte held; ported when the module is resident and its
/// handler is in [`MODULE_TEXT`]. Returns `None` where it is not ported.
pub fn dispatch(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    dispatch_at(c, bus, 0)
}

/// Dispatcher calls in progress, across module handlers (which restart their own depth count).
static ACTIVE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn dispatch_at(c: &mut Cpu, bus: &mut dyn Bus, depth: u32) -> Option<u64> {
    use std::sync::atomic::Ordering;
    if ACTIVE.fetch_add(1, Ordering::Relaxed) >= MAX_DEPTH {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    let r = dispatch_inner(c, bus, depth);
    ACTIVE.fetch_sub(1, Ordering::Relaxed);
    r
}

fn dispatch_inner(c: &mut Cpu, bus: &mut dyn Bus, depth: u32) -> Option<u64> {
    let (si0, es0) = (c.x(ESI), c.es);
    let id = c.x(EAX);
    let ax2 = id << 1;
    c.set_x(EAX, ax2);
    let mcs = main_cs(c);
    let n;
    if id & 0x8000 != 0 {
        // Module strings.
        let slot = ax2 >> 10;
        let lock = lin(c.ds, 0x0596u16.wrapping_add(slot));
        let v = bus.read8(lock);
        bus.write8(lock, v);
        let hdr_off = bus.read16(lin(c.ds, 0x0396u16.wrapping_add(slot << 3)));
        let hdr_seg = bus.read16(lin(c.ds, 0x0398u16.wrapping_add(slot << 3)));
        let off = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x28)));
        let seg = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x2a)));
        let port = MODULE_TEXT.iter().find(|(_, t)| t[0].0 == off).map(|(_, t)| t[0].3)?;
        c.set_x(ESI, slot << 3);
        c.set_x(EBP, hdr_off);
        c.es = hdr_seg;
        // CALLF ES:[BP+28h]: run the handler with its own CS.
        let (ret_cs, ret_ip) = (c.cs, c.ip);
        let sp = c.x(crate::math3::ESP);
        bus.write16(lin(c.ss, sp.wrapping_sub(2)), ret_cs);
        bus.write16(lin(c.ss, sp.wrapping_sub(4)), 0xe8e1);
        c.set_x(crate::math3::ESP, sp.wrapping_sub(4));
        c.cs = seg;
        c.ip = off;
        if depth > MAX_DEPTH {
            return None;
        }
        let m = port(c, bus);
        if m == DECLINE {
            return None;
        }
        c.cs = ret_cs;
        c.ip = ret_ip;
        c.set_x(ESI, slot);
        n = 13 + m + 5;
    } else if id & 0x4000 != 0 {
        // The table at [DS:5703].
        c.set_x(EAX, ax2 & 0x7fff);
        c.set_x(ESI, bus.read16(lin(c.ds, 0x5703)));
        c.es = bus.read16(lin(c.ds, 0x5705));
        n = 8 + expand_at(c, bus, depth)? + 3;
    } else if ax2 >> 8 < 0x60 {
        // The main table.
        c.set_x(ESI, 0xe4dd);
        c.set_x(EDX, mcs);
        c.es = mcs;
        n = 11 + expand_at(c, bus, depth)? + 3;
    } else {
        // A formatter.
        let target = bus.read16(lin(mcs, 0x8915u16.wrapping_add(ax2)));
        c.set_x(EBX, ax2);
        c.set_x(EAX, c.ss);
        c.es = c.ss;
        let saved = [bus.read32(lin(c.ds, 0x07c0)), bus.read32(lin(c.ds, 0x07c4)), bus.read32(lin(c.ds, 0x07c8))];
        let m = formatter(c, bus, target, depth)?;
        bus.write32(lin(c.ds, 0x07c0), saved[0]);
        bus.write32(lin(c.ds, 0x07c4), saved[1]);
        bus.write32(lin(c.ds, 0x07c8), saved[2]);
        n = 14 + m + 6;
    }
    c.set_x(ESI, si0);
    c.es = es0;
    Some(n)
}

/// `1000:b609` (`BL` = minimum digits code) or `1000:b607` (`BL` = 0) as entered by a far call,
/// through its `RETF`, *observed*: print the signed 32-bit `DX:AX` in decimal to `ES:DI`, followed
/// by a 0. A `-` first for negatives; then digits from the largest power of ten (10⁹ down) that
/// fits, each found by repeated subtraction; `BL` = 3, 2, 1 forces at least 4, 3, 2 digits (the
/// callers place a decimal point). `DI` ends past the 0; `AX` = the units digit; `CX` = the units
/// digit code; `DX` = what is left of the high word.
pub fn print_decimal(c: &mut Cpu, bus: &mut dyn Bus, from_b607: bool) -> u64 {
    let mut n: u64 = 0;
    if from_b607 {
        c.set_x(EBX, c.x(EBX) & 0xff00);
        n += 1;
    }
    let bl = c.x(EBX) as u8;
    let es = c.es;
    let mut di = c.x(EDI);
    let mut cx = c.x(EAX);
    let mut dx = c.x(EDX);
    n += 4; // MOV CX,AX; XOR AL,AL; OR DX,DX; JNS
    if dx & 0x8000 != 0 {
        let v = ((dx as u32) << 16 | cx as u32).wrapping_neg();
        cx = v as u16;
        dx = (v >> 16) as u16;
        bus.write8(lin(es, di), b'-');
        di = di.wrapping_add(1);
        n += 6;
    }
    let v = (dx as u32) << 16 | cx as u32;
    // The cascade: 32-bit thresholds compare DX first (CMP; JA; JC; CMP; JNC).
    const BIG: [u32; 5] = [1_000_000_000, 100_000_000, 10_000_000, 1_000_000, 100_000];
    let mut start = None;
    for (k, &p) in BIG.iter().enumerate() {
        let h = (p >> 16) as u16;
        n += if dx > h { 2 } else if dx < h { 3 } else { 5 };
        if v >= p {
            start = Some(k);
            break;
        }
    }
    if start.is_none() {
        // OR DX,DX; JNZ; CMP CX,2710h; JNC
        n += if dx != 0 { 2 } else { 4 };
        if v >= 10_000 {
            start = Some(5);
        }
    }
    if start.is_none() {
        // CMP CX,1000; JNC; CMP BL,3; JZ — then 100 with BL 2, 10 with BL 1, else JMP b773.
        let small = [(1000u32, 3u8), (100, 2), (10, 1)];
        for (k, &(p, b)) in small.iter().enumerate() {
            n += 2;
            if v >= p {
                start = Some(6 + k);
                break;
            }
            n += 2;
            if bl == b {
                start = Some(6 + k);
                break;
            }
        }
        if start.is_none() {
            n += 1; // JMP b773
            start = Some(9);
        }
    }
    const POW: [u32; 9] = [1_000_000_000, 100_000_000, 10_000_000, 1_000_000, 100_000, 10_000, 1000, 100, 10];
    let mut rem = v;
    let mut al: u8 = 0;
    for (k, &p) in POW.iter().enumerate().skip(start.unwrap()) {
        let d = rem / p;
        rem %= p;
        // INC AL; SUB; (SBB); JNC per try, d + 1 tries; then the restore, ADD AL,2Fh, STOSB, XOR AL,AL.
        let (per, tail) = if k < 6 { (4, 5) } else { (3, 4) };
        n += per * (d as u64 + 1) + tail;
        al = b'0' + d as u8;
        bus.write8(lin(es, di), al);
        di = di.wrapping_add(1);
    }
    let _ = al;
    // b773: ADD CL,30h; MOV AL,CL; XOR AH,AH; STOSW; RETF
    let units = b'0' + rem as u8;
    bus.write8(lin(es, di), units);
    bus.write8(lin(es, di.wrapping_add(1)), 0);
    di = di.wrapping_add(2);
    c.set_x(EAX, units as u16);
    c.set_x(ECX, units as u16);
    c.set_x(EDX, (rem >> 16) as u16);
    c.set_x(EDI, di);
    n + 5
}

/// `1000:b5a3` as entered by a near call, through its `RET`, *observed*: `DX:AX` as 8 digits of
/// base 16 to `ES:DI` and a 0 (`'0'`–`'9'`, then `3Ah + 51h` = 8Bh for ten, and so on: the font's
/// codes for A–F). `BL` = the last digit code, `CX` = 0.
fn print_hex(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut v = (c.x(EDX) as u32) << 16 | c.x(EAX) as u32;
    let es = c.es;
    let mut di = c.x(EDI);
    let mut n: u64 = 1;
    let mut bl = 0u8;
    for _ in 0..8 {
        let nib = (v >> 28) as u8;
        v <<= 4;
        bl = nib + 0x30;
        n += 1 + 12 + 3;
        if bl > 0x39 {
            bl = bl.wrapping_add(0x51);
            n += 1;
        }
        bus.write8(lin(es, di), bl);
        di = di.wrapping_add(1);
        n += 3;
    }
    bus.write8(lin(es, di), 0);
    di = di.wrapping_add(1);
    c.set_x(EAX, v as u16);
    c.set_x(EDX, (v >> 16) as u16);
    c.set_x(EBX, (c.x(EBX) & 0xff00) | bl as u16);
    c.set_x(ECX, 0);
    c.set_x(EDI, di);
    n + 3
}

/// The argument slots a formatter reads (`DS:07c0`, `07c4`, `07c8`, `07e4`, `07e8`), by the
/// formatter's entry stub; the stub's own instructions before the shared body.
fn slot_value(c: &Cpu, bus: &mut dyn Bus, slot: u16) -> (u16, u16) {
    (bus.read16(lin(c.ds, slot)), bus.read16(lin(c.ds, slot + 2)))
}

/// A formatter (`3000h`–`3FFFh`) from its `CALL` target through its `RET`, *observed*; `None` for
/// formatters that are not ported (dates, times, `3022h`–`3025h`).
fn formatter(c: &mut Cpu, bus: &mut dyn Bus, target: u16, depth: u32) -> Option<u64> {
    // (entry, argument slot, instructions of the stub before the shared body)
    let pick = |targets: [(u16, u16); 3]| targets.iter().position(|t| t.0 == target).map(|k| (targets[k].1, if k == 2 { 2 } else { 3 }));
    match target {
        0xe9b2 => Some(1), // 3000h: RET
        0xeaf3 | 0xeafc => {
            let slot = if target == 0xeaf3 { 0x07e4 } else { 0x07e8 };
            c.set_x(ESI, bus.read16(lin(c.ds, slot)));
            c.es = bus.read16(lin(c.ds, slot + 2));
            Some(3 + expand_loop_at(c, bus, depth)? + 1)
        }
        // Plain numbers through b607: 3001h–3003h, 301Dh ([07e4]), 301Eh ([07e8]).
        0xeb81 | 0xeb78 | 0xeb6f | 0xeb5d | 0xeb66 => {
            let (slot, stub) = match target {
                0xeb81 => (0x07c0, 2),
                0xeb78 => (0x07c4, 3),
                0xeb6f => (0x07c8, 3),
                0xeb5d => (0x07e4, 3),
                _ => (0x07e8, 3),
            };
            let (a, d) = slot_value(c, bus, slot);
            c.set_x(EAX, a);
            c.set_x(EDX, d);
            c.es = c.ss;
            Some(stub + 3 + print_decimal(c, bus, true) + 1)
        }
        // One, two or three decimals: 3004h–3006h, 3007h–3009h, 300Ah–300Ch.
        0xeba2 | 0xeb99 | 0xeb90 | 0xebd6 | 0xebcd | 0xebc4 | 0xec0e | 0xec05 | 0xebfc => {
            let (places, slots) = match target {
                // [jumping stub, jumping stub, the stub that falls into the body]
                0xeba2 | 0xeb99 | 0xeb90 => (1u8, [(0xeb90, 0x07c8), (0xeb99, 0x07c4), (0xeba2, 0x07c0)]),
                0xebd6 | 0xebcd | 0xebc4 => (2, [(0xebc4, 0x07c8), (0xebcd, 0x07c4), (0xebd6, 0x07c0)]),
                _ => (3, [(0xebfc, 0x07c8), (0xec05, 0x07c4), (0xec0e, 0x07c0)]),
            };
            let (slot, stub) = pick(slots)?;
            let (a, d) = slot_value(c, bus, slot);
            c.set_x(EAX, a);
            c.set_x(EDX, d);
            Some(stub + decimals(c, bus, places))
        }
        // Hexadecimal: 3013h–3015h.
        0xeb17 | 0xeb0e | 0xeb05 => {
            let (slot, stub) = pick([(0xeb05, 0x07c8), (0xeb0e, 0x07c4), (0xeb17, 0x07c0)])?;
            let (a, d) = slot_value(c, bus, slot);
            c.set_x(EAX, a);
            c.set_x(EDX, d);
            c.set_x(ECX, c.ss);
            c.es = c.ss;
            Some(stub + 3 + print_hex(c, bus) + 1)
        }
        // Tonnes: 301Fh–3021h ("-" for zero, otherwise the number and "t").
        0xeb38 | 0xeb2f | 0xeb26 => {
            let (slot, stub) = pick([(0xeb26, 0x07c8), (0xeb2f, 0x07c4), (0xeb38, 0x07c0)])?;
            let (a, d) = slot_value(c, bus, slot);
            c.set_x(EAX, a);
            c.set_x(EDX, d);
            c.set_x(EBX, a | d);
            if a | d == 0 {
                let di = c.x(EDI);
                bus.write16(lin(c.ss, di), 0x002d);
                c.set_x(EDI, di.wrapping_add(2));
                return Some(stub + 3 + 3);
            }
            c.es = c.ss;
            let m = print_decimal(c, bus, true);
            let di = c.x(EDI);
            bus.write16(lin(c.es, di.wrapping_sub(1)), 0x0074);
            c.set_x(EDI, di.wrapping_add(1));
            Some(stub + 3 + 3 + m + 3)
        }
        // Dates: 300Dh–300Fh and 3029h with the year; 3026h–3028h without.
        0xea07 | 0xe9f8 | 0xe9e9 | 0xe9da | 0xe9b4 | 0xe9c3 | 0xe9d2 => {
            let (slot, flag, stub): (Option<u16>, u16, u64) = match target {
                0xea07 => (Some(0x07c0), 1, 3),
                0xe9f8 => (Some(0x07c4), 1, 4),
                0xe9e9 => (Some(0x07c8), 1, 4),
                0xe9da => (Some(0x1fc5), 1, 4),
                0xe9b4 => (Some(0x07c8), 0, 4),
                0xe9c3 => (Some(0x07c4), 0, 4),
                _ => (None, 0, 2),
            };
            if let Some(slot) = slot {
                let (a, d) = slot_value(c, bus, slot);
                c.set_x(EAX, a);
                c.set_x(EDX, d);
            }
            bus.write16(lin(c.ds, 0x07c8), flag);
            Some(stub + date(c, bus))
        }
        // Times: 3010h–3012h.
        0xeab2 | 0xeaa9 | 0xeaa0 => {
            let (slot, stub) = pick([(0xeaa0, 0x07c8), (0xeaa9, 0x07c4), (0xeab2, 0x07c0)])?;
            let (a, d) = slot_value(c, bus, slot);
            c.set_x(EAX, a);
            c.set_x(EDX, d);
            Some(stub + time(c, bus)?)
        }
        // A text id held in a slot: 3018h–301Ch ([07c0], [07c4], [07c8], [07e4], [07e8]).
        0xec58 | 0xec50 | 0xec48 | 0xec40 | 0xec38 => {
            let slot = match target {
                0xec58 => 0x07c0,
                0xec50 => 0x07c4,
                0xec48 => 0x07c8,
                0xec40 => 0x07e4,
                _ => 0x07e8,
            };
            c.set_x(EAX, bus.read16(lin(c.ds, slot)));
            Some(2 + dispatch_at(c, bus, depth + 1)? + 1)
        }
        _ => None,
    }
}

/// `CALLF [DS:slot]` into a native `EL2MATH3` port, returning the instructions including the
/// `CALLF`; `None` if no port covers the slot or it declines.
fn callf_math3(c: &mut Cpu, bus: &mut dyn Bus, slot: u16, ret_ip: u16) -> Option<u64> {
    let port = crate::math3::PORTS.iter().find(|p| p.1 == slot)?.3;
    let off = bus.read16(lin(c.ds, slot));
    let seg = bus.read16(lin(c.ds, slot + 2));
    let sp = c.x(crate::math3::ESP);
    bus.write16(lin(c.ss, sp.wrapping_sub(2)), c.cs);
    bus.write16(lin(c.ss, sp.wrapping_sub(4)), ret_ip);
    c.set_x(crate::math3::ESP, sp.wrapping_sub(4));
    let (cs, ip) = (c.cs, c.ip);
    c.cs = seg;
    c.ip = off;
    let n = port(c, bus);
    if n == DECLINE {
        return None;
    }
    c.cs = cs;
    c.ip = ip;
    Some(n + 1)
}

/// `CWD`.
fn cwd(c: &mut Cpu) {
    c.set_x(EDX, if c.x(EAX) & 0x8000 != 0 { 0xffff } else { 0 });
}

/// The date formatters' body (`1000:ea14`), *observed*: `DX:AX` days, `DS:07c8` = 1 to print the
/// year. 1461-day cycles of four years (the first a leap year); in the other years a day is skipped
/// after 28 February so one day-of-year table (`1000:e529`: day in bits 0–4, month in bits 5–8)
/// serves all; then day, `-`, the month's 3 letters (`1000:e4f9`) and, with the flag, `-` and the
/// year. A divide overflow resumes at `1000:e9b3` (`RET`) through the `int 0` handler.
fn date(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mcs = main_cs(c);
    let es = c.es;
    let v = (c.x(EDX) as u32) << 16 | c.x(EAX) as u32;
    c.set_x(EBX, 0x05b5);
    bus.write16(lin(c.ds, crate::math3::DIVIDE_RESUME), 0xe9b3);
    let q = v / 1461;
    if q > 0xffff {
        // #DE: the int 0 handler (9 instructions) resumes at e9b3: RET.
        return 3 + 9 + 1;
    }
    let mut d = (v % 1461) as u16;
    let mut year = (q as u32) << 2;
    let mut n: u64 = 12;
    if d >= 0x16e {
        d -= 0x16e;
        n += 1;
        loop {
            year += 1;
            n += 8;
            if d < 0x16d {
                break;
            }
            d -= 0x16d;
        }
        n += 3; // ADD DX,16Dh; CMP DX,3Bh; JL
        if (d as i16) >= 0x3b {
            d += 1;
            n += 1;
        }
    }
    let bx = d << 1;
    let packed = bus.read16(lin(mcs, 0xe529u16.wrapping_add(bx)));
    c.set_x(EAX, packed & 0x1f);
    c.set_x(EDX, 0);
    let sp = c.x(crate::math3::ESP);
    // The year stays pushed (CX, AX of the 4-year count) until the end.
    bus.write16(lin(c.ss, sp.wrapping_sub(2)), year as u16);
    bus.write16(lin(c.ss, sp.wrapping_sub(4)), (year >> 16) as u16);
    n += 7;
    n += print_decimal(c, bus, true);
    let mb = (packed >> 3) & 0x3c;
    c.set_x(EBX, mb);
    let di = c.x(EDI);
    bus.write8(lin(es, di.wrapping_sub(1)), b'-');
    let w = bus.read16(lin(mcs, 0xe4f9u16.wrapping_add(mb)));
    bus.write16(lin(es, di), w);
    let b = bus.read8(lin(mcs, 0xe4fbu16.wrapping_add(mb)));
    bus.write8(lin(es, di.wrapping_add(2)), b);
    c.set_x(EAX, (w & 0xff00) | b as u16);
    let di = di.wrapping_add(3);
    c.set_x(EDI, di);
    n += 11;
    if bus.read16(lin(c.ds, 0x07c8)) == 0 {
        bus.write8(lin(es, di), 0);
        c.set_x(EDI, di.wrapping_add(1));
        return n + 4;
    }
    bus.write8(lin(es, di), b'-');
    c.set_x(EDI, di.wrapping_add(1));
    c.set_x(EDX, (year >> 16) as u16);
    c.set_x(EAX, year as u16);
    n += 5;
    n += print_decimal(c, bus, true);
    n + 1
}

/// The time formatters' body (`1000:eab9`), *observed*: `DX:AX` scaled by `divide32` (slot
/// `DS:02f4`) with `CX:BX` = 0:C22Eh, then hours (`DIV 3600`), `:`, minutes, `:`, seconds, the last
/// two with at least two digits. A quotient too large for 16 bits is not ported (declines).
fn time(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
    c.set_x(ECX, 0);
    c.set_x(EBX, 0xc22e);
    let mut n = 2 + callf_math3(c, bus, 0x02f4, 0xeac3)?;
    let v = (c.x(EDX) as u32) << 16 | c.x(EAX) as u32;
    c.set_x(EBP, 0x0e10);
    let h = v / 3600;
    if h > 0xffff {
        return None;
    }
    let rest = (v % 3600) as u16;
    c.set_x(EAX, h as u16);
    cwd(c);
    n += 5;
    n += print_decimal(c, bus, true);
    let di = c.x(EDI);
    bus.write8(lin(c.es, di.wrapping_sub(1)), b':');
    c.set_x(EAX, rest);
    cwd(c);
    c.set_x(EBX, 0x3c);
    let (m, sec) = (rest / 60, rest % 60);
    c.set_x(EAX, m);
    cwd(c);
    c.set_x(EBX, 0x0001);
    n += 9;
    n += print_decimal(c, bus, false);
    let di = c.x(EDI);
    bus.write8(lin(c.es, di.wrapping_sub(1)), b':');
    c.set_x(EAX, sec);
    cwd(c);
    c.set_x(EBX, (c.x(EBX) & 0xff00) | 1);
    n += 5;
    n += print_decimal(c, bus, false);
    Some(n + 1)
}

/// The decimal-point formatters' body (`1000:eba9`, `ebdd`, `ec15`): `BL` = places, `ES` = `SS`,
/// `CALLF b609`, then a `.` is moved in before the last `places` digits and the 0 rewritten.
fn decimals(c: &mut Cpu, bus: &mut dyn Bus, places: u8) -> u64 {
    c.set_x(EBX, (c.x(EBX) & 0xff00) | places as u16);
    c.set_x(ECX, c.ss);
    c.es = c.ss;
    let m = print_decimal(c, bus, false);
    let es = c.es;
    let di = c.x(EDI);
    // MOV AL,2Eh; XCHG [DI-places-1]…[DI-2]; MOV [DI-1],AL; MOV [DI],0; INC DI; RET
    let mut al = b'.';
    for k in (2..=places as u16 + 1).rev() {
        let a = lin(es, di.wrapping_sub(k));
        let old = bus.read8(a);
        bus.write8(a, al);
        al = old;
    }
    bus.write8(lin(es, di.wrapping_sub(1)), al);
    bus.write8(lin(es, di), 0);
    c.set_x(EAX, (c.x(EAX) & 0xff00) | al as u16);
    c.set_x(EDI, di.wrapping_add(1));
    4 + m + 1 + places as u64 + 4
}

/// Port of `1000:e89d` (slot `DS:0014`), the text dispatcher ([`dispatch`]).
pub fn dispatch_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    match dispatch(c, bus) {
        Some(n) => {
            c.retf(bus);
            n
        }
        None => DECLINE,
    }
}

/// The dispatcher's entry in `FRONTIER.EXE`.
pub const MAIN_TEXT: Table = &[(0xe89d, 0x0014, "dispatch", dispatch_port)];

/// How a module's text handler points `ES` at itself before calling the expander.
#[derive(Clone, Copy)]
enum Load {
    /// `MOV SI,seg; MOV ES,SI; MOV SI,table` (`DX` untouched).
    ViaSi,
    /// `MOV SI,table; MOV DX,CS; MOV ES,DX` or `MOV DX,seg; MOV ES,DX` (`DX` = the module's segment).
    ViaDx,
}

/// A module text handler of the common shape: `AND AX,3FEh`, point `ES:SI` at the module's table,
/// `CALLF [DS:005c]`, `RETF`.
fn module_text(c: &mut Cpu, bus: &mut dyn Bus, table: u16, load: Load) -> u64 {
    let ax = c.x(EAX) & 0x3fe;
    c.set_x(EAX, ax);
    c.es = c.cs;
    c.set_x(ESI, table);
    if let Load::ViaDx = load {
        c.set_x(EDX, c.cs);
    }
    // CALLF [005c] pushes a return frame below the stack (dead after the return).
    match expand(c, bus) {
        Some(n) => {
            c.retf(bus);
            5 + n + 1
        }
        None => DECLINE,
    }
}

macro_rules! text_port {
    ($name:ident, $table:expr, $load:expr, $addr:literal) => {
        #[doc = concat!("Port of `", $addr, "` (handler `+28`), *observed*: the module's string `AX/2` (table at `", stringify!($table), "`) to `SS:DI`.")]
        pub fn $name(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
            module_text(c, bus, $table, $load)
        }
    };
}

text_port!(m00_text, 0x0241, Load::ViaSi, "EL2M00:4e19");
text_port!(m03_text, 0x0255, Load::ViaDx, "EL2M03:3dd6");
text_port!(m05_text, 0x00a3, Load::ViaDx, "EL2M05:0170");
text_port!(m10_text, 0x00a3, Load::ViaDx, "EL2M10:0e17");
text_port!(m14_text, 0x030b, Load::ViaSi, "EL2M14:2d84");
text_port!(m15_text, 0x00a3, Load::ViaDx, "EL2M15:0d1b");
text_port!(m17_text, 0x014e, Load::ViaDx, "EL2M17:0b60");
text_port!(m18_text, 0x00a3, Load::ViaSi, "EL2M18:4cfa");

/// Port of `EL2M11:093c` (handler `+28`), *observed*: a lone `RETF` (the module has no strings).
pub fn m11_text(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    c.retf(bus);
    1
}

/// Port of `EL2M07:06ec` (handler `+28`), *observed*: as the common handler with the table at
/// `00a3`, except string 1Fh (`AX` = 3Eh after the mask), which formats a value and is not ported
/// (the port declines it).
pub fn m07_text(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let ax = c.x(EAX) & 0x3fe;
    if ax == 0x3e {
        return DECLINE;
    }
    c.set_x(EAX, ax);
    c.es = c.cs;
    c.set_x(ESI, 0x00a3);
    match expand(c, bus) {
        Some(n) => {
            c.retf(bus);
            7 + n + 1
        }
        None => DECLINE,
    }
}

/// One port table per module (offset of handler `+28`, slot 0, name, port), in the shape
/// `frontier_hooks::install_ports` takes.
pub type Table = &'static [(u16, u16, &'static str, Port)];

/// The ported text handlers: (module file, its table).
pub const MODULE_TEXT: &[(&str, Table)] = &[
    ("EL2M00.OVL", &[(0x4e19, 0, "m00_text", m00_text)]),
    ("EL2M03.OVL", &[(0x3dd6, 0, "m03_text", m03_text)]),
    ("EL2M05.OVL", &[(0x0170, 0, "m05_text", m05_text)]),
    ("EL2M07.OVL", &[(0x06ec, 0, "m07_text", m07_text)]),
    ("EL2M10.OVL", &[(0x0e17, 0, "m10_text", m10_text)]),
    ("EL2M11.OVL", &[(0x093c, 0, "m11_text", m11_text)]),
    ("EL2M14.OVL", &[(0x2d84, 0, "m14_text", m14_text)]),
    ("EL2M15.OVL", &[(0x0d1b, 0, "m15_text", m15_text)]),
    ("EL2M17.OVL", &[(0x0b60, 0, "m17_text", m17_text)]),
    ("EL2M18.OVL", &[(0x4cfa, 0, "m18_text", m18_text)]),
];
