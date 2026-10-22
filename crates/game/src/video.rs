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
mod screen;
mod driver;
mod palette;
mod shapes;
mod sprite;
mod texture;
mod texture_cases;
pub use raster::*;
pub use screen::*;
pub use driver::*;
pub use palette::*;
pub use shapes::*;
pub use sprite::*;
pub use texture::*;

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
/// Trap flag.
pub const TF: u32 = 0x0100;
/// The arithmetic flags.
pub const CF: u32 = 0x0001;
pub const PF: u32 = 0x0004;
pub const AF: u32 = 0x0010;
pub const ZF: u32 = 0x0040;
pub const SF: u32 = 0x0080;
pub const OF: u32 = 0x0800;

/// Parity flag of the low byte of `r`: set when it has an even number of bits set.
pub fn parity(r: u32) -> bool {
    (r as u8).count_ones() % 2 == 0
}

/// The zero, sign and parity flags of a 16-bit result.
pub fn szp16(r: u16) -> u32 {
    (if r == 0 { ZF } else { 0 }) | (if r & 0x8000 != 0 { SF } else { 0 }) | (if parity(r as u32) { PF } else { 0 })
}

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

    /// `PUSH r32`.
    pub fn push32(&mut self, v: u32) {
        self.push((v >> 16) as u16);
        self.push(v as u16);
    }

    /// `POP r32`.
    pub fn pop32(&mut self) -> u32 {
        let lo = self.pop() as u32;
        lo | (self.pop() as u32) << 16
    }

    /// A far call to `seg:off` returning to `ret` in this segment (`CALLF`, or `PUSH CS; CALL`
    /// within it): +1 instruction. A port that ends here leaves the routine called to run as
    /// itself; the code after the call is ported as its own entry at `ret`.
    pub fn far_call(&mut self, seg: u16, off: u16, ret: u16) {
        let cs = self.c.cs;
        self.push(cs);
        self.push(ret);
        self.c.cs = seg;
        self.c.ip = off;
        self.steps += 1;
    }

    /// `CALLF [slot]` through the main vector table (`DS:slot`), returning to `ret`.
    pub fn call_slot(&mut self, slot: u16, ret: u16) {
        let (off, seg) = (self.ds_rw(slot), self.ds_rw(slot.wrapping_add(2)));
        self.far_call(seg, off, ret);
    }

    /// `INT n` returning to `ret` with the flags `flags` pushed (the arithmetic flags as the
    /// instructions before left them): +1 instruction. Ends a port; the service runs as itself.
    pub fn int(&mut self, n: u8, ret: u16, flags: u32) {
        let cs = self.c.cs;
        self.push(flags as u16);
        self.push(cs);
        self.push(ret);
        self.c.flags = flags & !(IF | TF);
        let v = self.bus.read32(n as u32 * 4);
        self.c.ip = v as u16;
        self.c.cs = (v >> 16) as u16;
        self.steps += 1;
    }

    /// Decline if the direction flag is set: the string instructions would run backwards.
    pub fn forward(&self) -> Result<(), Decline> {
        if self.c.flags & DF != 0 {
            // Not ported: the string instructions with DF set (never seen); the original runs.
            Err(Decline("direction flag set"))
        } else {
            Ok(())
        }
    }
}

/// Every export of `EL2MCGA3`: (offset, vector slot, name, port). Entries with slot 0 are the
/// driver's own code after a call out of a port (`int 67h`, a far call): the routine called
/// runs as itself and returns there. Exports not listed here run the original.
pub const EXPORTS: &[(u16, u16, &str, Port)] = &[
    (0x1b62, 0x0264, "triangle", triangle),
    (0x1b3f, 0x0260, "triangle_clipped", triangle_clipped),
    (0x1e74, 0x022c, "quad", quad),
    (0x1e41, 0x0228, "quad_clipped", quad_clipped),
    (0x28f9, 0x0210, "line", line),
    (0x2f4e, 0x01d4, "disc_clipped", disc_clipped),
    (0x2f5a, 0x01d8, "disc", disc),
    (0x2dd1, 0x0214, "blob", blob),
    (0x3108, 0x01fc, "fill_rows", fill_rows),
    (0x3399, 0x01f4, "fill_masks", fill_masks),
    (0x3343, 0x01f8, "fill_masks_args", fill_masks_args),
    (0x1a1b, 0x0218, "glyph", glyph),
    (0x012b, 0x0258, "text_width", text_width),
    (0x015b, 0x01d0, "char_width", char_width),
    (0x06ba, 0x01ec, "fill_rect", fill_rect),
    (0x072a, 0x01f0, "fill_rect_masked", fill_rect_masked),
    (0x076e, 0x0224, "pixel", pixel),
    (0x0788, 0x0268, "target_back", target_back),
    (0x078f, 0x026c, "target_screen", target_screen),
    (0x3489, 0x01dc, "clear_view_7f", clear_view_7f),
    (0x349c, 0x01e0, "clear_view_0", clear_view_0),
    (0x02d3, 0x0220, "blit", blit),
    (0x02ce, 0x021c, "blit_pointer", blit_pointer),
    (0x02dc, 0, "blit@02dc", blit_after_lookup),
    (0x0173, 0, "cache", cache),
    (0x01b4, 0, "cache@01b4", cache_after_save),
    (0x01be, 0, "cache@01be", cache_after_map),
    (0x0244, 0, "cache@0244", cache_after_restore),
    (0x04d8, 0x0240, "save_under", save_under),
    (0x0583, 0x0230, "restore_under", restore_under),
    (0x0681, 0x0254, "set_pointer", set_pointer),
    (0x068c, 0, "set_pointer@068c", pointer_after_hide),
    (0x0698, 0, "set_pointer@0698", pointer_after_lookup),
    (0x06b9, 0, "set_pointer@06b9", retf_after_call),
    (0x34ac, 0x01e8, "copy_screen", copy_screen),
    (0x34b0, 0, "copy_screen@34b0", copy_after_hide),
    (0x34c8, 0, "copy_screen@34c8", retf_after_call),
    (0x34c9, 0x01e4, "copy_panel", copy_panel),
    (0x34cd, 0, "copy_panel@34cd", panel_after_hide),
    (0x34e6, 0, "copy_panel@34e6", retf_after_call),
    (0x59ce, 0x0238, "colour", colour),
    (0x5a20, 0x0200, "colour_entry", colour_entry),
    (0x5a29, 0x023c, "colours_tick", colours_tick),
    (0x5b50, 0x0234, "upload_colours", upload_colours),
    (0x5d4a, 0x0244, "nothing", nothing),
    (0x0811, 0x024c, "set_palette", set_palette),
    (0x081d, 0, "set_palette@081d", palette_after_reset),
    (0x142a, 0x0248, "load_high_colours", load_high_colours),
    (0x1432, 0, "load_high_colours@1432", high_after_reset),
    (0x34e7, 0x0250, "load_colours", load_colours),
    (0x34f0, 0, "load_colours@34f0", colours_after_reset),
    (0x3635, 0x0208, "textured_triangle", textured_triangle),
    (0x36c0, 0, "textured_triangle@36c0", textured_triangle_after_lookup),
    (0x44d1, 0x0204, "textured_quad", textured_quad),
    (0x45b3, 0, "textured_quad@45b3", textured_quad_after_lookup),
    (0x0796, 0x025c, "flip", flip),
    (0x079a, 0, "flip@079a", flip_after_shape),
    (0x07ab, 0, "flip@07ab", flip_after_position),
    (0x07cb, 0, "flip@07cb", flip_after_save),
    (0x07d4, 0, "flip@07d4", flip_after_pointer),
    (0x07d8, 0, "flip@07d8", flip_after_hook),
    (0x0807, 0, "flip@0807", flip_after_restore),
    (0x05b8, 0x020c, "init", init),
    (0x05d1, 0, "init@05d1", init_after_alloc),
    (0x0619, 0, "init@0619", init_after_mode),
    (0x0623, 0, "init@0623", init_after_palette),
    (0x062a, 0, "init@062a", init_after_target),
    (0x0636, 0, "init@0636", init_after_open),
    (0x063f, 0, "init@063f", init_after_read),
    (0x0648, 0, "init@0648", init_after_pointer),
    (0x064c, 0, "init@064c", init_after_show),
    (0x065b, 0, "init@065b", init_after_clear),
    (0x065f, 0, "init@065f", init_after_flip),
    (0x066e, 0, "init@066e", retf_after_call),
];
