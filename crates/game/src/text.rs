//! The game's text: the main program's expander and the modules' string handlers (docs/TEXT.md).
//!
//! Every module keeps its strings behind a table of word offsets (relative to the table) and
//! answers text ids from `8000h` up through its handler `+28`. Most handlers copy the string with
//! the main program's expander `1000:e969` (slot `DS:005c`). The ports are register- and
//! memory-exact and return the original's instruction counts; strings are read from the user's
//! files in memory.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBX, EDI, EDX, ESI};

/// The expander's loop (`1000:e96e`), from `ES:SI` to `SS:DI`, through its `RETF`. Returns the
/// instructions it executes, or `None` for an `FFh` escape naming a string other than `3016h`
/// (the nested-string formatter `1000:eaf3`, which copies the string at `[DS:07e4]`), which is not
/// ported. See [`crate::names`] for the byte codes.
pub fn expand_loop(c: &mut Cpu, bus: &mut dyn Bus) -> Option<u64> {
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
            let w = bus.read16(lin(c.es, si));
            let id = w.swap_bytes();
            if id != 0x3016 {
                return None;
            }
            c.set_x(ESI, si.wrapping_add(2));
            let (si, es) = (c.x(ESI), c.es);
            c.set_x(EBX, id << 1);
            c.set_x(EAX, c.ss);
            c.set_x(ESI, bus.read16(lin(c.ds, 0x07e4)));
            c.es = bus.read16(lin(c.ds, 0x07e6));
            n += 14 + 14 + 3;
            n += expand_loop(c, bus)?;
            c.set_x(ESI, si);
            c.es = es;
            let di = c.x(EDI).wrapping_sub(1);
            c.set_x(EDI, di);
            n += 1 + 6 + 2;
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
    let ax = c.x(EAX);
    c.set_x(EBX, ax);
    let si = c.x(ESI);
    let rel = bus.read16(lin(c.es, ax.wrapping_add(si)));
    c.set_x(ESI, si.wrapping_add(rel));
    Some(2 + expand_loop(c, bus)?)
}

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
