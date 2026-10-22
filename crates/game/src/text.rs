//! The game's text: the main program's expander and the modules' string handlers (docs/TEXT.md).
//!
//! Every module keeps its strings behind a table of word offsets (relative to the table) and
//! answers text ids from `8000h` up through its handler `+28`. Most handlers copy the string with
//! the main program's expander `1000:e969` (slot `DS:005c`). The ports are register- and
//! memory-exact and return the original's instruction counts; strings are read from the user's
//! files in memory.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, EDI, EDX, ESI};

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
        if target != 0xeaf3 {
            return None;
        }
        c.set_x(EBX, ax2);
        c.set_x(EAX, c.ss);
        c.es = c.ss;
        let saved = [bus.read32(lin(c.ds, 0x07c0)), bus.read32(lin(c.ds, 0x07c4)), bus.read32(lin(c.ds, 0x07c8))];
        // eaf3: LES SI,[07e4]; PUSH CS; CALL e96e; RET
        c.set_x(ESI, bus.read16(lin(c.ds, 0x07e4)));
        c.es = bus.read16(lin(c.ds, 0x07e6));
        let m = expand_loop_at(c, bus, depth)?;
        bus.write32(lin(c.ds, 0x07c0), saved[0]);
        bus.write32(lin(c.ds, 0x07c4), saved[1]);
        bus.write32(lin(c.ds, 0x07c8), saved[2]);
        n = 14 + 3 + m + 1 + 6;
    }
    c.set_x(ESI, si0);
    c.es = es0;
    Some(n)
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
