//! `EL2MCGA3`: the MCGA/VGA mode 13h video driver (docs/VIDEO.md).
//!
//! The driver exports 40 far routines into the main vector table slots `DS:01d0`–`026f`
//! (docs/OVERLAYS.md §2): flat-shaded spans, triangles, quadrilaterals, lines, discs and the
//! scanline lists the 3D renderer builds, text, bitmaps from `EL2G.GRF` (with a cache in EMS),
//! texture-mapped triangles and quadrilaterals, rectangle fills, screen clears and copies, the
//! page flip, and the palette (DAC uploads and a colour allocator).
//!
//! Each port is a transliteration of the original instructions, as for `EL2MATH3`
//! ([`crate::math3`]): it takes and leaves every register and memory byte the routine writes,
//! counts the instructions the original executes for its inputs, and performs the same VGA port
//! accesses in the same order ([`Io`]). The drawing primitives are split into an entry that reads
//! the original's register interface and a rasteriser taking geometric inputs (vertices, colour,
//! clipping) — the boundary a draw-command layer can use later (docs/VIDEO.md §6).
//!
//! Parity against the original running on the reference runtime: `crates/hooks/tests/mcga3_parity.rs`.

mod raster;
pub use raster::*;

pub use crate::math3::{lin, lin32, Bus, Cpu, DIVIDE_HANDLER_STEPS, DIVIDE_RESUME, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};

/// VGA ports as the driver sees them. `steps` is the number of instructions the call has
/// executed before the `IN` or `OUT`, so a host can place the access in time.
pub trait Io {
    fn inb(&mut self, steps: u64, port: u16) -> u8;
    fn outb(&mut self, steps: u64, port: u16, v: u8);
}

/// Why a port left a call to the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decline(pub &'static str);

/// A call in progress: the 386 state, memory, VGA ports and the instructions executed so far.
pub struct Ctx<'a> {
    pub c: Cpu,
    pub bus: &'a mut dyn Bus,
    pub io: &'a mut dyn Io,
    pub steps: u64,
}

/// A port of a whole routine, entry to `RETF`: leaves [`Ctx::c`] and memory as the original's
/// return would, with [`Ctx::steps`] the original's instruction count, or declines (nothing it
/// did counts; the hook runs the original). Memory below the final stack pointer is dead and
/// not reproduced.
pub type Port = fn(&mut Ctx) -> Result<(), Decline>;

/// Direction flag.
pub const DF: u32 = 0x0400;
/// Interrupt flag.
pub const IF: u32 = 0x0200;

// ---- main data segment variables the driver uses (names inferred from use) -------------

/// Segment every primitive draws into: `A000` or the back buffer (`DS:09e6`), chosen by the
/// exports at `DS:0268` and `DS:026c`. The primitives load `ES` from it.
pub const TARGET: u16 = 0x09e2;
/// Segment of the back buffer the driver allocated at start-up.
pub const BACK_BUFFER: u16 = 0x09e6;
/// Colour byte of the flat-shaded primitives.
pub const COLOUR: u16 = 0x57fd;
/// Near address of the span routine the triangle and quadrilateral fills call
/// (`EL2MCGA3:1a99` unclipped, `1a86` clipped).
pub const SPAN_ROUTINE: u16 = 0x57cf;
/// Scanline counter of the fills.
pub const ROWS: u16 = 0x57bd;
/// Slope of the first edge (dword, 16.16).
pub const SLOPE_A: u16 = 0x57bf;
/// Slope of the second edge (dword, 16.16).
pub const SLOPE_B: u16 = 0x57c3;
/// Start x of the first edge (dword, 16.16).
pub const START_A: u16 = 0x57c7;
/// Start x of the second edge (dword, 16.16).
pub const START_B: u16 = 0x57cb;
/// Fourth vertex of a quadrilateral (x, y).
pub const QUAD_X3: u16 = 0x57e9;
pub const QUAD_Y3: u16 = 0x57ed;
/// Byte: 0 when the fills swapped their edges into left/right order, FFh when not.
pub const EDGES_SWAPPED: u16 = 0x891d;

/// Width of the screen and height of the 3D view the fills clip against.
pub const WIDTH: u16 = 0x140;
pub const VIEW_HEIGHT: u16 = 0xa8;

impl Ctx<'_> {
    pub fn ds_rb(&mut self, off: u16) -> u8 {
        self.bus.read8(lin(self.c.ds, off))
    }
    pub fn ds_wb(&mut self, off: u16, v: u8) {
        self.bus.write8(lin(self.c.ds, off), v)
    }
    pub fn ds_rw(&mut self, off: u16) -> u16 {
        self.bus.read16(lin(self.c.ds, off))
    }
    pub fn ds_ww(&mut self, off: u16, v: u16) {
        self.bus.write16(lin(self.c.ds, off), v)
    }
    pub fn ds_rd(&mut self, off: u16) -> u32 {
        self.ds_rw(off) as u32 | (self.ds_rw(off.wrapping_add(2)) as u32) << 16
    }
    pub fn ds_wd(&mut self, off: u16, v: u32) {
        self.ds_ww(off, v as u16);
        self.ds_ww(off.wrapping_add(2), (v >> 16) as u16);
    }

    pub fn ax(&self) -> u16 {
        self.c.x(EAX)
    }
    pub fn bx(&self) -> u16 {
        self.c.x(EBX)
    }
    pub fn cx(&self) -> u16 {
        self.c.x(ECX)
    }
    pub fn dx(&self) -> u16 {
        self.c.x(EDX)
    }
    pub fn si(&self) -> u16 {
        self.c.x(ESI)
    }
    pub fn di(&self) -> u16 {
        self.c.x(EDI)
    }
    pub fn bp(&self) -> u16 {
        self.c.x(EBP)
    }
    pub fn sp(&self) -> u16 {
        self.c.x(ESP)
    }
    pub fn set(&mut self, r: usize, v: u16) {
        self.c.set_x(r, v)
    }

    /// `PUSH r16`.
    pub fn push(&mut self, v: u16) {
        let sp = self.sp().wrapping_sub(2);
        self.set(ESP, sp);
        self.bus.write16(lin(self.c.ss, sp), v);
    }

    /// `POP r16`.
    pub fn pop(&mut self) -> u16 {
        let sp = self.sp();
        self.set(ESP, sp.wrapping_add(2));
        self.bus.read16(lin(self.c.ss, sp))
    }

    /// `RETF` (+1 instruction).
    pub fn retf(&mut self) {
        self.c.retf(self.bus);
        self.steps += 1;
    }

    /// Decline if the direction flag is set: the string instructions would run backwards.
    pub fn forward(&self) -> Result<(), Decline> {
        if self.c.flags & DF != 0 {
            Err(Decline("direction flag set"))
        } else {
            Ok(())
        }
    }
}

/// Every export of `EL2MCGA3`: (offset, vector slot, name, port). Exports not listed here run
/// the original.
pub const EXPORTS: &[(u16, u16, &str, Port)] = &[
    (0x1b62, 0x0264, "triangle", triangle),
    (0x1b3f, 0x0260, "triangle_clipped", triangle_clipped),
];
