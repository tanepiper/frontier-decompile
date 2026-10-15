//! The 3D routines of `EL2MATH3`: perspective projection, vertex transformation and the lists
//! they fill (docs/MATH3.md). They work on the stack frame the frame allocators give their
//! caller (`BP`), on vertex records in the stack segment and on model data reached through `FS`.
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives its
//! address and the number added to the instruction count is that block's instruction count.

use super::*;

/// Code segment offset of the reciprocal table in `EL2MATH3` used by [`project`]: 1024 words,
/// indexed by a scaled depth.
pub const RECIPROCAL_TABLE: u32 = 0x0122;

/// `IMUL CX` of `a`: the signed 32-bit product as (high word, low word).
fn imul16(a: u16, b: u16) -> (u16, u16) {
    let p = (a as i16 as i32 * b as i16 as i32) as u32;
    ((p >> 16) as u16, p as u16)
}

/// `SAR EAX,2; SAR EBX,2; SAR ECX,2`.
fn sar2(v: &mut [u32; 3]) {
    for x in v.iter_mut() {
        *x = ((*x as i32) >> 2) as u32;
    }
}

/// The halving loop at `EL2MATH3:1a24` (and `1a78`): while a coordinate is outside
/// −1F00h < v < 1F00h, `SAR` both by one. Returns the instruction count without the `RETF`.
fn clamp(ax: &mut u16, bx: &mut u16) -> u64 {
    let mut n = 0;
    for first in [true, false] {
        n += 2; // OR r,r; JNS
        loop {
            let v = if first { *ax } else { *bx } as i16;
            n += 2; // CMP; JG/JL
            let inside = if v < 0 { v > -0x1f00 } else { v < 0x1f00 };
            if inside {
                break;
            }
            *ax = ((*ax as i16) >> 1) as u16;
            *bx = ((*bx as i16) >> 1) as u16;
            n += 3; // SAR; SAR; JMP
        }
    }
    n
}

/// Port of `EL2MATH3:19c2` (slot `DS:0314`): perspective projection of the point (`EAX`, `EBX`,
/// `ECX`) = (x, y, z), 32-bit, to screen offsets `AX` ≈ x·k/z, `BX` = `DX` ≈ y·k/z.
///
/// First x and y are brought within the depth: the unsigned comparisons against z and −z route
/// to two scaling loops (`SAR` all three by 2 while x, or y, is outside ±8000h). Then the depth
/// picks a reciprocal from the table at `CS:0122` ([`RECIPROCAL_TABLE`]) at one of four scales
/// (z < 400h, < 1000h, < 4000h, else halved below 8000h first), x and y are multiplied by it and
/// shifted, and for the two nearest scales both are halved together until each is within
/// ±1F00h. `CX` is left as the reciprocal. If z stays negative the original loops for ever.
pub fn project(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    #[derive(Clone, Copy)]
    enum At {
        L19d3,
        L19e5,
        L19e8,
        L1b5c,
    }
    let mut v = [c.r[EAX], c.r[EBX], c.r[ECX]];
    let mut n: u64 = 2; // 19c2: CMP ECX,EAX; JNC
    let mut at = if v[2] >= v[0] {
        // 1b48
        v[2] = v[2].wrapping_neg();
        n += 3;
        if v[2] < v[1] {
            At::L19e5
        } else {
            v[2] = v[2].wrapping_neg();
            n += 3;
            if v[2] >= v[1] {
                At::L19e8
            } else {
                At::L1b5c
            }
        }
    } else {
        v[2] = v[2].wrapping_neg();
        n += 3; // NEG ECX; CMP ECX,EAX; JNC
        if v[2] >= v[0] {
            // 1b01: x outside the depth
            n += 2; // OR EAX,EAX; JS
            if (v[0] as i32) < 0 {
                n += 2; // 1b27: CMP EAX,FFFF8000h; JNC
                if v[0] < 0xffff_8000 {
                    loop {
                        sar2(&mut v);
                        n += 5;
                        if v[0] >= 0xffff_8000 {
                            break;
                        }
                    }
                    n += 1;
                }
            } else {
                n += 2; // CMP EAX,8000h; JC
                if v[0] >= 0x8000 {
                    loop {
                        sar2(&mut v);
                        n += 5;
                        if v[0] < 0x8000 {
                            break;
                        }
                    }
                    n += 1;
                }
            }
        }
        At::L19d3
    };
    loop {
        at = match at {
            At::L19d3 => {
                n += 2; // CMP ECX,EBX; JC
                if v[2] < v[1] {
                    At::L19e5
                } else {
                    v[2] = v[2].wrapping_neg();
                    n += 3;
                    if v[2] < v[1] {
                        At::L1b5c
                    } else {
                        v[2] = v[2].wrapping_neg();
                        n += 1;
                        At::L19e5
                    }
                }
            }
            At::L19e5 => {
                v[2] = v[2].wrapping_neg();
                n += 1;
                At::L19e8
            }
            At::L1b5c => {
                // y outside the depth
                n += 2; // OR EBX,EBX; JS
                if (v[1] as i32) < 0 {
                    n += 2;
                    if v[1] < 0xffff_8000 {
                        loop {
                            sar2(&mut v);
                            n += 5;
                            if v[1] >= 0xffff_8000 {
                                break;
                            }
                        }
                        n += 1;
                    }
                } else {
                    n += 2;
                    if v[1] >= 0x8000 {
                        loop {
                            sar2(&mut v);
                            n += 5;
                            if v[1] < 0x8000 {
                                break;
                            }
                        }
                        n += 1;
                    }
                }
                At::L19e8
            }
            At::L19e8 => break,
        }
    }
    // 19e8: pick the scale by depth.
    let (index, shift, near) = if v[2] >= 0x4000 {
        n += 2;
        loop {
            n += 2; // 1ad8: CMP ECX,8000h; JNC
            if v[2] < 0x8000 {
                break;
            }
            if (v[2] as i32) < 0 {
                return DECLINE; // SAR keeps a negative depth at or above 8000h: never returns
            }
            for x in v.iter_mut() {
                *x = ((*x as i32) >> 1) as u32;
            }
            n += 3;
        }
        n += 12;
        ((v[2] >> 6) << 1, 5, false)
    } else if v[2] >= 0x1000 {
        n += 4 + 12;
        ((v[2] >> 4) << 1, 3, false)
    } else if v[2] >= 0x400 {
        n += 6 + 11;
        ((v[2] >> 2) << 1, 1, true)
    } else {
        n += 6 + 12;
        (v[2] << 1, 0, true)
    };
    let k = bus.read16(lin32(c.cs, index + RECIPROCAL_TABLE));
    let scale = |a: u16| {
        let (hi, lo) = imul16(a, k);
        if shift == 0 {
            // SHL AX,1; RCL DX,1
            hi << 1 | lo >> 15
        } else {
            ((hi as i16) >> shift) as u16
        }
    };
    let mut ax = scale(v[0] as u16);
    let mut bx = scale(v[1] as u16);
    let dx = bx;
    // PUSH DX … POP AX carries x through the stack; callers' frames can see that word.
    bus.write16(lin(c.ss, c.x(ESP).wrapping_sub(2)), ax);
    if near {
        n += clamp(&mut ax, &mut bx);
        n += 1; // RETF
    }
    c.r[EAX] = (v[0] & 0xFFFF_0000) | ax as u32;
    c.r[EBX] = (v[1] & 0xFFFF_0000) | bx as u32;
    c.r[ECX] = k as u32;
    c.set_x(EDX, dx);
    c.retf(bus);
    n
}

/// Port of `EL2MATH3:1ba7` (slot `DS:030c`): start a list at `SS:[DS:1f3b]`. Its first dword is
/// copied from `[BP-62h]`; then (`ES:[SI+8]` / 2 − 2) words of `8080h` follow, if positive.
pub fn init_list(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut di = bus.read16(lin(c.ds, 0x1f3b));
    let cx = bus.read16(lin(c.es, c.x(ESI).wrapping_add(8)));
    let head = bus.read32(lin(c.ss, c.x(EBP).wrapping_sub(0x62)));
    bus.write32(lin(c.ss, di), head);
    c.r[EAX] = (head & 0xFFFF_0000) | 0x8080;
    di = di.wrapping_add(4);
    let count = (cx >> 1).wrapping_sub(2);
    let mut n = 9;
    if (count as i16) <= 0 {
        c.set_x(ECX, count);
    } else {
        for _ in 0..count {
            bus.write16(lin(c.ss, di), 0x8080);
            di = di.wrapping_add(2);
        }
        n += 3 * count as u64;
        c.set_x(ECX, 0);
    }
    c.set_x(EDI, di);
    c.retf(bus);
    n + 1
}

// ---- vertex transformation ----------------------------------------------------------------

/// Main data segment words the vertex routines use.
pub mod vars {
    /// Stack segment offset of vertex record 0. Records are 20h bytes; the record at +20h is the
    /// mirror partner (x negated) and shares the definition.
    pub const VERTICES: u16 = 0x1f39;
    /// List pointer used by [`super::init_list`].
    pub const LIST: u16 = 0x1f3b;
    /// Word: the state a transformed vertex gets (FFFFh from `v0300`, 1 from `v02fc`); later
    /// reused as a dword scratch.
    pub const MARK: u16 = 0x1f63;
    /// The game's random number generator: two dwords (`[1fd9]` += `[1fdd]`, `[1fdd]` = the old
    /// `[1fd9]` rotated by 16).
    pub const RNG: u16 = 0x1fd9;
}

/// `BP`-relative fields of the caller's frame (the frame allocators' 5598h frame), as offsets.
mod frame {
    /// Far pointer to the model's vertex definitions (`LFS SI,[BP-DCh]`).
    pub const DEFINITIONS: u16 = 0xff24;
    /// Word: right shift that scales transformed coordinates (`17h` − this).
    pub const SCALE: u16 = 0xff2c;
    /// Byte: the mark a projected vertex gets in its state byte.
    pub const FRAME_MARK: u16 = 0xff66;
    /// 3×3 matrix of words at `BP-24h`…`BP-14h`.
    pub const MATRIX: u16 = 0xffdc;
    /// Translation dwords at `BP-10h`, `BP-0Ch`, `BP-8`.
    pub const TRANSLATION: u16 = 0xfff0;
}

/// Deepest vertex dependency chain a port follows before declining (the game's are short).
const MAX_DEPTH: u32 = 64;

fn sx16(v: u16) -> u32 {
    v as i16 as i32 as u32
}

/// Context of one vertex-routine call: the CPU, memory and the nesting depth.
struct V<'a> {
    c: &'a mut Cpu,
    bus: &'a mut dyn Bus,
    depth: u32,
}

impl V<'_> {
    fn ds16(&mut self, off: u16) -> u16 {
        self.bus.read16(lin(self.c.ds, off))
    }
    fn ds32(&mut self, off: u16) -> u32 {
        self.bus.read32(lin(self.c.ds, off))
    }
    fn set_ds16(&mut self, off: u16, v: u16) {
        self.bus.write16(lin(self.c.ds, off), v)
    }
    fn set_ds32(&mut self, off: u16, v: u32) {
        self.bus.write32(lin(self.c.ds, off), v)
    }
    fn ss8(&mut self, off: u16) -> u8 {
        self.bus.read8(lin(self.c.ss, off))
    }
    fn ss16(&mut self, off: u16) -> u16 {
        self.bus.read16(lin(self.c.ss, off))
    }
    fn ss32(&mut self, off: u16) -> u32 {
        self.bus.read32(lin(self.c.ss, off))
    }
    fn set_ss8(&mut self, off: u16, v: u8) {
        self.bus.write8(lin(self.c.ss, off), v)
    }
    fn set_ss16(&mut self, off: u16, v: u16) {
        self.bus.write16(lin(self.c.ss, off), v)
    }
    fn set_ss32(&mut self, off: u16, v: u32) {
        self.bus.write32(lin(self.c.ss, off), v)
    }
    /// `LFS r,SS:[off]`: (offset word, segment word). The runtime reads the segment word at
    /// linear address + 2 without wrapping the offset at 64 KB.
    fn far_ptr(&mut self, off: u16) -> (u16, u16) {
        let a = lin(self.c.ss, off);
        (self.bus.read16(a), self.bus.read16(a + 2))
    }
    /// `[BP+off]` (stack segment).
    fn bp(&self, off: u16) -> u16 {
        self.c.x(EBP).wrapping_add(off)
    }
    fn push16(&mut self, v: u16) {
        let sp = self.c.x(ESP).wrapping_sub(2);
        self.c.set_x(ESP, sp);
        self.set_ss16(sp, v);
    }
    fn pop16(&mut self) -> u16 {
        let sp = self.c.x(ESP);
        self.c.set_x(ESP, sp.wrapping_add(2));
        self.ss16(sp)
    }
    fn push32(&mut self, v: u32) {
        let sp = self.c.x(ESP).wrapping_sub(4);
        self.c.set_x(ESP, sp);
        self.set_ss32(sp, v);
    }
    fn pop32(&mut self) -> u32 {
        let sp = self.c.x(ESP);
        self.c.set_x(ESP, sp.wrapping_add(4));
        self.ss32(sp)
    }
    fn retf(&mut self) {
        self.c.retf(self.bus);
    }
    /// `CALLF [slot]` to the routine `f` (the slots hold their `EL2MATH3` entries once the
    /// overlay is loaded, *assumed* throughout): 1 + its instructions.
    fn callf(&mut self, ret: u16, f: fn(&mut V) -> u64) -> u64 {
        if self.depth >= MAX_DEPTH {
            return DECLINE;
        }
        let cs = self.c.cs;
        self.push16(cs);
        self.push16(ret);
        self.depth += 1;
        let n = f(self);
        self.depth -= 1;
        n.saturating_add(1)
    }
    /// `CALLF [0314]` ([`project`]).
    fn call_project(&mut self, ret: u16) -> u64 {
        let cs = self.c.cs;
        self.push16(cs);
        self.push16(ret);
        project(self.c, self.bus).saturating_add(1)
    }
    /// `MOV DI,AX` (after `SHL AX,5`) `; ADD DI,[1f39]`: the record of the vertex `AX` names.
    fn record(&mut self, ax: u16) -> u16 {
        let base = self.ds16(vars::VERTICES);
        ax.wrapping_add(base)
    }
}

/// Add up instruction counts, keeping [`DECLINE`].
fn add(a: u64, b: u64) -> u64 {
    a.saturating_add(b)
}

/// `EL2MATH3:0e50`: the shared tail. The vertex record at `DI` gets its world position
/// (rotated position + translation, `+4/+8/+C`); if its state word is not negative (`v02fc`'s
/// mark) it is projected too ([`project_record`]). Pops `SI`, `BP`... and returns.
fn finish(v: &mut V) -> u64 {
    let (bp, si) = (v.c.x(EBP), v.c.x(ESI));
    v.push16(bp);
    v.push16(si);
    let di = v.c.x(EDI);
    let world = world_position(v, di);
    v.c.r[EAX] = world[0];
    v.c.r[EBX] = world[1];
    v.c.r[ECX] = world[2];
    if (v.ss16(di.wrapping_add(0x12)) as i16) < 0 {
        let si = v.pop16();
        v.c.set_x(ESI, si);
        let bp = v.pop16();
        v.c.set_x(EBP, bp);
        v.retf();
        return 13 + 3;
    }
    add(13, project_record(v, 0x0e8f))
}

/// `EAX/EBX/ECX = [BP-10h/-0Ch/-8] + SS:[DI+14h/18h/1Ch]`, stored to `SS:[DI+4/8/C]`.
fn world_position(v: &mut V, di: u16) -> [u32; 3] {
    let mut w = [0u32; 3];
    for (i, x) in w.iter_mut().enumerate() {
        let k = 4 * i as u16;
        let t = v.ss32(v.bp(frame::TRANSLATION.wrapping_add(k)));
        *x = t.wrapping_add(v.ss32(di.wrapping_add(0x14 + k)));
    }
    for (i, x) in w.iter().enumerate() {
        v.set_ss32(di.wrapping_add(4 + 4 * i as u16), *x);
    }
    w
}

/// `EL2MATH3:0e83`/`0d02` onwards, with `BP` and `SI` pushed and the world position in
/// `EAX`/`EBX`/`ECX`: `XCHG DI,BP`; a point nearer than depth 40h gets the screen position
/// `8002h, 8002h`, others are projected and offset to the screen centre (`+A0h`, `54h −`).
/// Then the record's state byte gets the frame mark, `SI` and `BP` are popped and it returns.
fn project_record(v: &mut V, ret: u16) -> u64 {
    let (di, bp) = (v.c.x(EDI), v.c.x(EBP));
    v.c.set_x(EDI, bp);
    v.c.set_x(EBP, di);
    let mut n: u64 = 3; // XCHG DI,BP; CMP ECX,40h; JL
    if (v.c.r[ECX] as i32) < 0x40 {
        v.set_ss32(di, 0x8002_8002);
        n += 7;
    } else {
        n = add(n, v.call_project(ret));
        if n == DECLINE {
            return DECLINE;
        }
        let ax = v.c.x(EAX).wrapping_add(0xa0);
        let bx = v.c.x(EBX).wrapping_sub(0x54).wrapping_neg();
        v.c.set_x(EAX, ax);
        v.c.set_x(EBX, bx);
        v.set_ss16(di, ax);
        v.set_ss16(di.wrapping_add(2), bx);
        n += 11;
    }
    v.c.set_x(EDI, di);
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let bp = v.pop16();
    v.c.set_x(EBP, bp);
    let mark = v.ss8(v.bp(frame::FRAME_MARK));
    v.c.set_l(EAX, mark);
    v.set_ss8(di.wrapping_add(0x12), mark);
    v.retf();
    n
}

/// Port of `EL2MATH3:0d2b` (slot `DS:0300`): transform vertex `AX` (= index × 20h) of the
/// current model into its record, marking it FFFFh (transformed, not projected).
pub fn transform_vertex(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    vertex(&mut V { c, bus, depth: 0 })
}

/// Port of `EL2MATH3:0ccc` (slot `DS:02fc`): make sure the vertex record at `DI` is projected.
/// If its state `DX` is 0 it is transformed first (marked 1, `EL2MATH3:0d31`); otherwise the
/// world position is recomputed from the stored rotated one. Then it is projected.
pub fn project_vertex(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    project_vertex_v(&mut V { c, bus, depth: 0 })
}

/// Port of `EL2MATH3:0c35` (slot `DS:0304`): world position of vertex `AX` (index): from the
/// stored rotated position if the record is transformed, else by transforming it (`v0300`).
pub fn vertex_position(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut v = V { c, bus, depth: 0 };
    let ax = v.c.x(EAX) << 5;
    v.c.set_x(EAX, ax);
    let di = v.record(ax);
    v.c.set_x(EDI, di);
    if v.ss16(di.wrapping_add(0x12)) == 0 {
        return add(5, vertex(&mut v));
    }
    let world = world_position(&mut v, di);
    v.c.r[EAX] = world[2];
    v.retf();
    15
}

fn vertex(v: &mut V) -> u64 {
    v.set_ds16(vars::MARK, 0xffff);
    add(1, transform(v))
}

fn project_vertex_v(v: &mut V) -> u64 {
    v.set_ds16(vars::MARK, 1);
    if v.c.x(EDX) == 0 {
        return add(3, transform(v));
    }
    let (bp, si) = (v.c.x(EBP), v.c.x(ESI));
    v.push16(bp);
    v.push16(si);
    let di = v.c.x(EDI);
    let world = world_position(v, di);
    v.c.r[EAX] = world[0];
    v.c.r[EBX] = world[1];
    v.c.r[ECX] = world[2];
    add(3 + 11, project_record(v, 0x0d10))
}

/// `EL2MATH3:0d31`: transform the vertex `AX` names into the record at `DI`, by its type.
fn transform(v: &mut V) -> u64 {
    let (di0, si0) = (v.c.x(EDI), v.c.x(ESI));
    v.push16(di0);
    v.push16(si0);
    let ax = v.c.x(EAX) & 0xffdf;
    v.c.set_x(EAX, ax);
    if ax & 0x8000 != 0 {
        return add(4, copy_vertex(v));
    }
    // 0d3a: the definition: two words (type and parameters) per mirror pair.
    let (mut si, fs) = v.far_ptr(v.bp(frame::DEFINITIONS));
    let di = v.record(ax);
    v.c.set_x(EDI, di);
    let ax = ax >> 4;
    v.c.set_x(EAX, ax);
    si = si.wrapping_add(ax);
    v.c.set_x(ESI, si);
    let bx = v.bus.read16(lin(fs, si));
    let dx = v.bus.read16(lin(fs, si.wrapping_add(2)));
    v.c.set_x(EBX, bx);
    v.c.set_x(EDX, dx);
    v.c.fs = fs;
    let kind = (bx >> 8) as u8 as i8;
    let mut n: u64 = 4 + 9;
    if kind <= 2 {
        return add(n, plain(v, true));
    }
    n += 2; // 0ec2: CMP BX,4FFh; JG
    if (bx as i16) <= 0x4ff {
        return add(n, midpoint_projected(v));
    }
    n += 2; // 0fd0: CMP BH,6; JG
    if kind <= 6 {
        return add(n, negated(v));
    }
    n += 2; // 106b: CMP BH,8; JG
    if kind <= 8 {
        return add(n, perturbed(v));
    }
    n += 2; // 1151: CMP BH,0Ah; JG
    if kind <= 10 {
        return add(n, plain(v, false));
    }
    add(n, combined(v))
}

/// `EL2MATH3:0c72`: a negative vertex number: copy the record named by the frame's table at
/// `BP-44h` into the caller's record at `DI`, with its rotated position recomputed from the
/// world one. Returns with `SI` and `DI` popped.
fn copy_vertex(v: &mut V) -> u64 {
    let di = v.c.x(EDI);
    let base = v.ds16(vars::VERTICES);
    let k = ((di.wrapping_sub(base) as i16) >> 3) as u16;
    let si = v.ss16(v.bp(k).wrapping_sub(0x44));
    let mut eax = v.ss32(si);
    v.set_ss32(di, eax);
    for (j, t) in [(4u16, 0xfff0u16), (8, 0xfff4), (0xc, 0xfff8)] {
        eax = v.ss32(si.wrapping_add(j));
        v.set_ss32(di.wrapping_add(j), eax);
        let rel = eax.wrapping_sub(v.ss32(v.bp(t)));
        v.set_ss32(di.wrapping_add(j + 0x10), rel);
    }
    eax = v.ss32(si.wrapping_add(0x10));
    v.set_ss32(di.wrapping_add(0x10), eax);
    v.c.r[EAX] = eax;
    let s = v.pop16();
    v.c.set_x(ESI, s);
    let d = v.pop16();
    v.c.set_x(EDI, d);
    v.retf();
    4 + 16 + 3
}

/// Signed 32-bit `IMUL` keeping the low 32 bits.
fn imul32(a: u32, b: u32) -> u32 {
    (a as i32).wrapping_mul(b as i32) as u32
}

/// `EL2MATH3:0d58` (types ≤ 2, `mirror`) and `1158` (types 9–10): rotate the definition's
/// (x, y, z) bytes (×100h) by the frame matrix, scale by `SAR 17h−[BP-D4h]`, store the rotated
/// position, and for the mirror partner (+20h) the same with x negated (types ≤ 2) or the
/// position minus one x column (types 9–10, *as observed in the code*). Then [`finish`].
fn plain(v: &mut V, mirror: bool) -> u64 {
    let di = v.c.x(EDI);
    let mark = v.ds16(vars::MARK);
    v.set_ss16(di.wrapping_add(0x12), mark);
    v.set_ss16(di.wrapping_add(0x32), mark);
    let (bl, dh, dl) = (v.c.l(EBX), v.c.h(EDX), v.c.l(EDX));
    // MOV AH,BL; MOV BH,DH; MOV CH,DL; XOR AL,AL; XOR BL,BL; XOR CL,CL
    v.c.set_x(EAX, (bl as u16) << 8);
    v.c.set_x(EBX, (dh as u16) << 8);
    v.c.set_x(ECX, (dl as u16) << 8);
    v.set_ds16(0x1f6f, di);
    let p = [sx16(v.c.x(EAX)), sx16(v.c.x(EBX)), sx16(v.c.x(ECX))];
    let r = rotate(v, p);
    let edx = r.edx;
    let bp = v.c.x(EBP);
    v.push16(bp);
    let cl = 0x17u16.wrapping_sub(v.ss16(v.bp(frame::SCALE)));
    let sh = (cl & 31) as u32;
    let sar = |x: u32| ((x as i32) >> sh) as u32;
    let (eax, ebx, edx) = (sar(r.eax), sar(r.ebx), sar(edx));
    let (edi, esi, ebp) = (sar(r.c0), sar(r.c1), sar(r.c2));
    let rec = v.ds16(0x1f6f);
    v.set_ss32(rec.wrapping_add(0x14), eax);
    v.set_ss32(rec.wrapping_add(0x18), ebx);
    v.set_ss32(rec.wrapping_add(0x1c), edx);
    let times = if mirror { 2 } else { 1 };
    let mut m = [eax, ebx, edx];
    for _ in 0..times {
        m[0] = m[0].wrapping_sub(edi);
        m[1] = m[1].wrapping_sub(esi);
        m[2] = m[2].wrapping_sub(ebp);
    }
    v.set_ss32(rec.wrapping_add(0x34), m[0]);
    v.set_ss32(rec.wrapping_add(0x38), m[1]);
    v.set_ss32(rec.wrapping_add(0x3c), m[2]);
    v.c.r[EAX] = m[0];
    v.c.r[EBX] = m[1];
    v.c.r[EDX] = m[2];
    v.c.r[EDI] = edi;
    v.c.r[ESI] = esi;
    v.c.r[ECX] = ebp;
    v.c.r[EBP] = (ebp & 0xFFFF_0000) | rec as u32;
    let bp = v.pop16();
    v.c.set_x(EBP, bp);
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    // 13 + 27 + (24 | 21 + JMP) + 6
    let n = if mirror { 13 + 27 + 24 + 6 } else { 13 + 27 + 22 + 6 };
    add(n, finish(v))
}

/// The matrix products of `EL2MATH3:0d73`–`0ded`: (`EAX`, `EBX`, `ECX`) = rows · p, and the
/// first column's three products kept in `[1f5f]`, `[1f63]`, `[1f67]` (also returned).
struct Rotated {
    eax: u32,
    ebx: u32,
    edx: u32,
    c0: u32,
    c1: u32,
    c2: u32,
}

fn rotate(v: &mut V, p: [u32; 3]) -> Rotated {
    let m = |v: &mut V, k: u16| sx16(v.ss16(v.bp(frame::MATRIX.wrapping_add(2 * k))));
    // Matrix words m0..m8 at BP-24h..BP-14h; rows use (m0, m3, m6), (m1, m4, m7), (m2, m5, m8).
    let c0 = imul32(m(v, 0), p[0]);
    v.set_ds32(0x1f5f, c0);
    let eax = imul32(m(v, 6), p[2]).wrapping_add(imul32(m(v, 3), p[1]).wrapping_add(c0));
    let c1 = imul32(m(v, 1), p[0]);
    v.set_ds32(0x1f63, c1);
    let ebx = imul32(m(v, 7), p[2]).wrapping_add(imul32(m(v, 4), p[1]).wrapping_add(c1));
    let c2 = imul32(m(v, 2), p[0]);
    v.set_ds32(0x1f67, c2);
    let edx = imul32(m(v, 8), p[2]).wrapping_add(imul32(m(v, 5), p[1]).wrapping_add(c2));
    v.c.r[EDX] = p[0];
    v.c.r[EDI] = p[1];
    v.c.r[ESI] = p[2];
    Rotated { eax, ebx, edx, c0, c1, c2 }
}

/// `CBW; SHL AX,5; MOV DI,AX; ADD DI,[1f39]; MOV DX,SS:[DI+12h]` with `AL` = `index`: the
/// record of the vertex a definition byte names, and its state word (5 instructions).
fn reference(v: &mut V, index: u8) {
    let ax = (index as i8 as i16 as u16) << 5;
    v.c.set_x(EAX, ax);
    let di = v.record(ax);
    v.c.set_x(EDI, di);
    let dx = v.ss16(di.wrapping_add(0x12));
    v.c.set_x(EDX, dx);
}

/// `SHL AX,5; MOV DI,AX; ADD DI,[1f39]; MOV DX,SS:[DI+12h]` with `AX` = `ax` (4 instructions).
fn reference_word(v: &mut V, ax: u16) {
    let ax = ax << 5;
    v.c.set_x(EAX, ax);
    let di = v.record(ax);
    v.c.set_x(EDI, di);
    let dx = v.ss16(di.wrapping_add(0x12));
    v.c.set_x(EDX, dx);
}

/// After `CMP DL,[BP-9Ah]; JZ`: `CALLF [02fc]` unless the record (`DI`, state `DX`) already
/// has this frame's mark. Returns the call's instructions (0 if none).
fn ensure_projected(v: &mut V, ret: u16) -> u64 {
    let mark = v.ss8(v.bp(frame::FRAME_MARK));
    if v.c.l(EDX) == mark {
        return 0;
    }
    v.callf(ret, project_vertex_v)
}

/// After `OR DX,DX; JNZ`: `CALLF [0300]` unless the record (`DI`, state `DX`) is transformed.
/// Returns the call's instructions (0 if none).
fn ensure_transformed(v: &mut V, ret: u16) -> u64 {
    if v.c.x(EDX) != 0 {
        return 0;
    }
    v.callf(ret, vertex)
}

/// The averaging block of `EL2MATH3:0f12` (and `0f8d`): with the two projected records at `DI`
/// and `BX`, store the midpoint of their screen positions at `SI+out` and the smaller (signed)
/// of their depths at `SI+depth`. 15 or 16 instructions.
fn midpoint_store(v: &mut V, out: u16, depth_at: u16) -> u64 {
    let (di, bx, si) = (v.c.x(EDI), v.c.x(EBX), v.c.x(ESI));
    for k in [0u16, 2] {
        let a = sx16(v.ss16(di.wrapping_add(k)));
        let b = sx16(v.ss16(bx.wrapping_add(k)));
        v.c.r[EDX] = b;
        let m = ((a.wrapping_add(b) as i32) >> 1) as u32;
        v.c.r[EAX] = m;
        v.set_ss16(si.wrapping_add(out + k), m as u16);
    }
    let mut n = 13 + 1;
    let mut depth = v.ss32(di.wrapping_add(0xc));
    let other = v.ss32(bx.wrapping_add(0xc));
    if (depth as i32) >= (other as i32) {
        depth = other;
        n += 1;
    }
    v.c.r[EAX] = depth;
    v.set_ss32(si.wrapping_add(depth_at), depth);
    n
}

/// `EL2MATH3:0eca` (types 3–4): each partner's screen position is the midpoint of two projected
/// vertices and its depth the nearer of theirs; the definition word `DX` names the first pair's
/// two vertices (high and low byte), `DX` xor 101h the partner's. Both records get this frame's
/// mark. Returns with `SI` and `DI` popped.
fn midpoint_projected(v: &mut V) -> u64 {
    let si = v.c.x(EDI);
    v.c.set_x(ESI, si);
    let mark = v.ss8(v.bp(frame::FRAME_MARK));
    v.c.set_l(EAX, mark);
    v.set_ss8(si.wrapping_add(0x12), mark);
    v.set_ss8(si.wrapping_add(0x32), mark);
    let dx = v.c.x(EDX);
    v.set_ds16(0x1f6b, dx);
    v.push16(dx);
    let mut n: u64 = 14; // 0eca..0ef1
    for (first, ret_a, ret_b) in [(true, 0x0ef7u16, 0x0f11u16), (false, 0x0f72, 0x0f8c)] {
        if !first {
            let pair = v.ds16(0x1f6b) ^ 0x0101;
            v.c.set_x(EAX, pair);
            v.push16(pair);
            n += 3 + 8; // 0f51..0f6c
        }
        let pair = v.c.x(if first { EDX } else { EAX });
        // MOV AL,DH (first) / MOV AL,AH; CBW; ...
        let hi = if first { dx >> 8 } else { pair >> 8 } as u8;
        reference(v, hi);
        n = add(n, ensure_projected(v, ret_a));
        let ax = v.pop16();
        v.c.set_x(EAX, ax);
        let di = v.c.x(EDI);
        v.push16(di);
        reference(v, ax as u8);
        n = add(n, 9); // POP AX; PUSH DI; CBW; SHL; MOV; ADD; MOV; CMP; JZ
        n = add(n, ensure_projected(v, ret_b));
        if n == DECLINE {
            return DECLINE;
        }
        let bx = v.pop16();
        v.c.set_x(EBX, bx);
        n += 1;
        n += if first { midpoint_store(v, 0, 0xc) } else { midpoint_store(v, 0x20, 0x2c) };
    }
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    v.retf();
    add(n, 3)
}

/// `EL2MATH3:0fd7` (types 5–6): the vertex pair is the negation of the pair `DX` names: each
/// partner's rotated position is minus that of the matching partner (`DX`, then `DX` xor 1).
/// Then [`finish`].
fn negated(v: &mut V) -> u64 {
    let bx = v.c.x(EDI);
    v.c.set_x(EBX, bx);
    let mark = v.ds16(vars::MARK);
    v.c.set_x(EAX, mark);
    v.set_ss16(bx.wrapping_add(0x12), mark);
    v.set_ss16(bx.wrapping_add(0x32), mark);
    let dx = v.c.x(EDX);
    v.push16(dx);
    v.push16(bx);
    reference_word(v, dx);
    let mut n = add(13, ensure_transformed(v, 0x0ffd));
    for (out, ret) in [(0x14u16, 0x103eu16), (0x34, 0)] {
        if n == DECLINE {
            return DECLINE;
        }
        let bx = v.pop16();
        v.c.set_x(EBX, bx);
        let di = v.c.x(EDI);
        for k in [0u16, 4, 8] {
            let e = v.ss32(di.wrapping_add(0x14 + k)).wrapping_neg();
            v.c.r[EAX] = e;
            v.set_ss32(bx.wrapping_add(out + k), e);
        }
        n += 1 + 9;
        if out == 0x14 {
            // POP AX; XOR AL,1; PUSH BX; SHL AX,5; MOV DI,AX; ADD DI; MOV DX; OR; JNZ
            let ax = v.pop16() ^ 1;
            v.push16(bx);
            reference_word(v, ax);
            n = add(n, 9);
            n = add(n, ensure_transformed(v, ret));
        }
    }
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    add(n + 3, finish(v))
}

/// `EL2MATH3:1072` (types 7–8): the vertex pair is the pair `DX` names plus a random offset per
/// axis from the game RNG ([`vars::RNG`]), `SAR` by the definition's low byte. The partner gets
/// the same offsets in the order z, y, x (*as observed*: the pushes are popped in reverse).
fn perturbed(v: &mut V) -> u64 {
    let (dx, di, bx) = (v.c.x(EDX), v.c.x(EDI), v.c.x(EBX));
    v.push16(dx);
    v.push16(di);
    v.push16(bx);
    let mark = v.ds16(vars::MARK);
    v.set_ss16(di.wrapping_add(0x12), mark);
    v.set_ss16(di.wrapping_add(0x32), mark);
    let ax = dx << 5;
    v.c.set_x(EAX, ax);
    let rec = v.record(ax);
    v.c.set_x(EDI, rec);
    let st = v.ss16(rec.wrapping_add(0x12));
    v.c.set_x(EDX, st);
    let mut n = add(13, ensure_transformed(v, 0x1097));
    if n == DECLINE {
        return DECLINE;
    }
    let cx = v.pop16();
    v.c.set_x(ECX, cx);
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let pair = v.pop16();
    v.set_ds16(0x1f67, pair);
    n += 3;
    let di = v.c.x(EDI);
    let cl = (cx & 31) as u32;
    for k in [0x14u16, 0x18, 0x1c] {
        let r = random(v);
        let off = ((r as i32) >> cl) as u32;
        v.push32(off);
        let eax = off.wrapping_add(v.ss32(di.wrapping_add(k)));
        v.set_ss32(si.wrapping_add(k), eax);
        v.c.r[EAX] = eax;
        n += 9;
    }
    let ax = (v.ds16(0x1f67) ^ 1) << 5;
    v.push16(si);
    v.c.set_x(EAX, ax);
    let rec = v.record(ax);
    v.c.set_x(EDI, rec);
    let st = v.ss16(rec.wrapping_add(0x12));
    v.c.set_x(EDX, st);
    n = add(n, add(9, ensure_transformed(v, 0x1127)));
    if n == DECLINE {
        return DECLINE;
    }
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.c.x(EDI);
    for (src, dst) in [(0x14u16, 0x34u16), (0x1c, 0x3c), (0x18, 0x38)] {
        let eax = v.pop32().wrapping_add(v.ss32(di.wrapping_add(src)));
        v.set_ss32(si.wrapping_add(dst), eax);
        v.c.r[EAX] = eax;
    }
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    add(n + 13, finish(v))
}

/// One step of the game's random number generator at `DS:1fd9` (`EL2MATH3:109d`): returns the
/// old first dword rotated by 16, which also becomes the second; the first gains the old
/// second. `EDX` is left as the old second dword.
fn random(v: &mut V) -> u32 {
    let a = v.ds32(vars::RNG);
    let b = v.ds32(vars::RNG + 4);
    v.set_ds32(vars::RNG, a.wrapping_add(b));
    let r = a.rotate_right(16);
    v.set_ds32(vars::RNG + 4, r);
    v.c.r[EDX] = b;
    r
}

// ---- vertex types above 10 ----------------------------------------------------------------

/// `EL2MATH3:123b` (types above 10): vertices built from others. The definition word `DX`
/// names them by byte; if the caller asked for the mirror partner (its `DI`, still on the stack,
/// is not the computed record) the bytes are flipped (xor 101h) and the two records swapped, so
/// `DI` is the record asked for and `BX` the other.
fn combined(v: &mut V) -> u64 {
    let (bx_def, dx_def) = (v.c.x(EBX), v.c.x(EDX));
    v.c.set_x(ECX, bx_def);
    v.set_ds16(0x1f6d, bx_def);
    v.set_ds16(0x1f6b, dx_def);
    let caller_di = v.ss16(v.c.x(ESP).wrapping_add(2));
    v.c.set_x(EBX, caller_di);
    let mut n: u64 = 7;
    let di = v.c.x(EDI);
    let swapped = caller_di != di;
    if swapped {
        let w = v.ds16(0x1f6b) ^ 0x0101;
        v.set_ds16(0x1f6b, w);
        v.c.set_x(EBX, di);
        v.c.set_x(EDI, caller_di);
        n += 2;
    }
    let di = v.c.x(EDI);
    let mark = v.ds16(vars::MARK);
    v.set_ss16(di.wrapping_add(0x12), mark);
    let ax = v.ds16(0x1f6b);
    v.c.set_x(EAX, ax);
    n += 5;
    let kind = v.c.h(ECX) as i8;
    if kind <= 0x0c {
        return add(n, average(v));
    }
    n += 2;
    if kind <= 0x0e {
        return add(n, average_projected(v));
    }
    n += 2;
    if kind <= 0x10 {
        return add(n, parallelogram(v));
    }
    n += 2;
    if kind <= 0x12 {
        return add(n, sum(v));
    }
    n += 2;
    if kind <= 0x14 {
        return add(n, interpolate(v));
    }
    add(n, along(v))
}

/// `MOV SI,SP; MOV SI,SS:[SI+2]`: the caller's `DI`, pushed at `EL2MATH3:0d31` (2 instructions).
fn caller_record(v: &mut V) -> u16 {
    let si = v.ss16(v.c.x(ESP).wrapping_add(2));
    v.c.set_x(ESI, si);
    si
}

/// `PUSH AX; MOV AL,AH; …; OR DX,DX; JNZ; (CALLF [0300]); POP AX; PUSH DI; CBW; …; JNZ;
/// (CALLF [0300]); POP BX`: transform the vertices `AH` (then in `BX`) and `AL` (then in `DI`).
/// `high_first` = false takes `AL` first (`EL2MATH3:1428`). Returns the instructions.
fn two_transformed(v: &mut V, high_first: bool, rets: [u16; 2]) -> u64 {
    let ax = v.c.x(EAX);
    v.push16(ax);
    let mut n: u64 = if high_first { 9 } else { 8 };
    reference(v, if high_first { (ax >> 8) as u8 } else { ax as u8 });
    n = add(n, ensure_transformed(v, rets[0]));
    if n == DECLINE {
        return DECLINE;
    }
    let ax = v.pop16();
    v.c.set_x(EAX, ax);
    let di = v.c.x(EDI);
    v.push16(di);
    reference(v, if high_first { ax as u8 } else { (ax >> 8) as u8 });
    n = add(n, if high_first { 9 } else { 10 });
    n = add(n, ensure_transformed(v, rets[1]));
    if n == DECLINE {
        return DECLINE;
    }
    let bx = v.pop16();
    v.c.set_x(EBX, bx);
    n + 1
}

/// `POP SI; POP DI; JMP 0e50`.
fn pop_and_finish(v: &mut V, n: u64) -> u64 {
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    add(n + 3, finish(v))
}

/// `EL2MATH3:1266` (types 11–12): the rotated position is the average of two vertices'.
fn average(v: &mut V) -> u64 {
    let n = two_transformed(v, true, [0x127f, 0x1297]);
    if n == DECLINE {
        return DECLINE;
    }
    let si = caller_record(v);
    let (di, bx) = (v.c.x(EDI), v.c.x(EBX));
    for k in [0x14u16, 0x18, 0x1c] {
        let a = v.ss32(di.wrapping_add(k)).wrapping_add(v.ss32(bx.wrapping_add(k)));
        let m = ((a as i32) >> 1) as u32;
        v.c.r[EAX] = m;
        v.set_ss32(si.wrapping_add(k), m);
    }
    pop_and_finish(v, n + 2 + 12)
}

/// `EL2MATH3:12e0` (types 13–14): screen position the midpoint of two projected vertices, depth
/// the nearer, world position the first one's (`DI`); marked as projected. No transform tail.
fn average_projected(v: &mut V) -> u64 {
    let ax = v.c.x(EAX);
    v.push16(ax);
    reference(v, (ax >> 8) as u8);
    let mut n = add(9, ensure_projected(v, 0x12fb));
    if n == DECLINE {
        return DECLINE;
    }
    let ax = v.pop16();
    v.c.set_x(EAX, ax);
    let di = v.c.x(EDI);
    v.push16(di);
    reference(v, ax as u8);
    n = add(n, add(9, ensure_projected(v, 0x1315)));
    if n == DECLINE {
        return DECLINE;
    }
    let bx = v.pop16();
    v.c.set_x(EBX, bx);
    let si = caller_record(v);
    n += 3;
    let di = v.c.x(EDI);
    for k in [0u16, 2] {
        let a = sx16(v.ss16(bx.wrapping_add(k)));
        let b = sx16(v.ss16(di.wrapping_add(k)));
        v.c.r[EDX] = b;
        let m = ((a.wrapping_add(b) as i32) >> 1) as u32;
        v.c.r[EAX] = m;
        v.set_ss16(si.wrapping_add(k), m as u16);
    }
    n += 13 + 1;
    let mut depth = v.ss32(di.wrapping_add(0xc));
    let other = v.ss32(bx.wrapping_add(0xc));
    if (depth as i32) >= (other as i32) {
        depth = other;
        n += 1;
    }
    v.set_ss32(si.wrapping_add(0xc), depth);
    let mut eax = 0;
    for k in [4u16, 8] {
        eax = v.ss32(di.wrapping_add(k));
        v.set_ss32(si.wrapping_add(k), eax);
    }
    let mark = v.ss8(v.bp(frame::FRAME_MARK));
    v.c.r[EAX] = (eax & !0xFF) | mark as u32;
    v.set_ss8(si.wrapping_add(0x12), mark);
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let di = v.pop16();
    v.c.set_x(EDI, di);
    v.retf();
    n + 4 + 5
}

/// `EL2MATH3:1381` (types 15–16): rotated position = c − b + a for the three vertices the
/// definition's bytes name (`DL`, `DH`, then the low parameter byte).
fn parallelogram(v: &mut V) -> u64 {
    let mut n: u64 = 2;
    if v.c.x(EBX) != v.c.x(EDI) {
        let b = v.bus.read8(lin(v.c.ds, 0x1f6d)) ^ 1;
        v.bus.write8(lin(v.c.ds, 0x1f6d), b);
        n += 1;
    }
    let lo = v.bus.read8(lin(v.c.ds, 0x1f6c));
    let hi = v.bus.read8(lin(v.c.ds, 0x1f6d));
    let bx = (hi as u16) << 8 | lo as u16;
    v.c.set_x(EBX, bx);
    v.push16(bx);
    n += 3;
    let ax = v.c.x(EAX);
    reference(v, ax as u8);
    n = add(n, add(7, ensure_transformed(v, 0x13a9)));
    for (high, ret) in [(false, 0x13c2u16), (true, 0x13dc)] {
        if n == DECLINE {
            return DECLINE;
        }
        let ax = v.pop16();
        v.c.set_x(EAX, ax);
        let di = v.c.x(EDI);
        v.push16(di);
        if !high {
            v.push16(ax);
        }
        reference(v, if high { (ax >> 8) as u8 } else { ax as u8 });
        n = add(n, add(10, ensure_transformed(v, ret)));
    }
    if n == DECLINE {
        return DECLINE;
    }
    let si = v.pop16();
    v.c.set_x(ESI, si);
    let bx = v.pop16();
    v.c.set_x(EBX, bx);
    let bp = v.c.x(EBP);
    v.push16(bp);
    let out = v.ss16(v.c.x(ESP).wrapping_add(4));
    v.c.set_x(EBP, out);
    let di = v.c.x(EDI);
    for k in [0x14u16, 0x18, 0x1c] {
        let e = v.ss32(si.wrapping_add(k)).wrapping_sub(v.ss32(bx.wrapping_add(k))).wrapping_add(v.ss32(di.wrapping_add(k)));
        v.c.r[EAX] = e;
        v.set_ss32(out.wrapping_add(k), e);
    }
    let bp = v.pop16();
    v.c.set_x(EBP, bp);
    pop_and_finish(v, n + 2 + 3 + 12 + 1)
}

/// `EL2MATH3:1428` (types 17–18): rotated position = the sum of two vertices'.
fn sum(v: &mut V) -> u64 {
    let n = two_transformed(v, false, [0x143f, 0x1459]);
    if n == DECLINE {
        return DECLINE;
    }
    let si = caller_record(v);
    let (di, bx) = (v.c.x(EDI), v.c.x(EBX));
    for k in [0x14u16, 0x18, 0x1c] {
        let e = v.ss32(bx.wrapping_add(k)).wrapping_add(v.ss32(di.wrapping_add(k)));
        v.c.r[EAX] = e;
        v.set_ss32(si.wrapping_add(k), e);
    }
    pop_and_finish(v, n + 2 + 9)
}

/// `NOT DX; NOT AX; ADD AX,1; ADC DX,0` when `DX` is negative: |`DX:AX`| (2 or 6 with the
/// `MOV`s… counted by the caller: returns the extra 4 when negated).
fn abs32(dx: &mut u16, ax: &mut u16) -> u64 {
    if (*dx as i16) >= 0 {
        return 0;
    }
    let v = ((*dx as u32) << 16 | *ax as u32).wrapping_neg();
    *ax = v as u16;
    *dx = (v >> 16) as u16;
    4
}

/// `INC BX; SHR DX,1; RCR AX,1; JNZ …; CMP AH,3Fh; JNC …` entered at the `OR DX,DX; JNZ` /
/// `CMP AH,3Fh; JC` test: shift `DX:AX` right until it is below 3F00h, counting in `BX`.
fn shrink(dx: &mut u16, ax: &mut u16, bx: &mut u16, test_first: u64) -> u64 {
    let mut n = test_first; // the JNZ (after OR DX,DX or the flags of ADC/OR)
    let mut in_loop = *dx != 0;
    if !in_loop {
        n += 2; // CMP AH,3Fh; JC
        in_loop = (*ax >> 8) as u8 >= 0x3f;
    }
    while in_loop {
        *bx = bx.wrapping_add(1);
        let v = ((*dx as u32) << 16 | *ax as u32) >> 1;
        *dx = (v >> 16) as u16;
        *ax = v as u16;
        n += 4;
        if *dx == 0 {
            n += 2;
            in_loop = (*ax >> 8) as u8 >= 0x3f;
        }
    }
    n
}

/// `EL2MATH3:1499` (types 19–20): rotated position = b + (a − b)·t, with a, b two projected
/// vertices (`DH`, `DL`) and t from the low parameter byte: a constant (0–3Fh, or that ×400h),
/// a word of the model's table at `[BP-D8h]:72h+`, or a word of the frame at `BP-40h+`; halved.
/// The difference is shifted down until each component is below 3F00h, multiplied by t (16 ×
/// 16 bits) and shifted back by (shifts − 15).
fn interpolate(v: &mut V) -> u64 {
    let w = v.ds16(0x1f6d);
    v.push16(w);
    let ax = v.c.x(EAX);
    v.push16(ax);
    reference(v, (ax >> 8) as u8);
    let mut n = add(10, ensure_projected(v, 0x14b8));
    if n == DECLINE {
        return DECLINE;
    }
    let ax = v.pop16();
    v.c.set_x(EAX, ax);
    let di = v.c.x(EDI);
    v.push16(di);
    reference(v, ax as u8);
    n = add(n, add(9, ensure_projected(v, 0x14d2)));
    if n == DECLINE {
        return DECLINE;
    }
    let bx = v.pop16();
    v.c.set_x(EBX, bx);
    let mut si = v.pop16() << 1;
    n += 5;
    let t = if si & 0x100 == 0 {
        n += 6;
        if si & 0x80 == 0 {
            (si >> 1) & 0x3f
        } else {
            si.rotate_right(7) & 0xfc00
        }
    } else if si & 0x80 == 0 {
        n += 9;
        si = (si & 0x7e).wrapping_add(v.ss16(v.bp(0xff28)));
        let es = v.ss16(v.bp(0xff2a));
        let es_saved = v.c.es;
        v.push16(es_saved);
        let t = v.bus.read16(lin(es, si.wrapping_add(0x72)));
        v.pop16();
        t
    } else {
        n += 4;
        si &= 0x7e;
        v.ss16(v.bp(si).wrapping_sub(0x40))
    };
    v.c.set_x(ESI, si);
    let t = t >> 1;
    n += 1;
    let (bp, bx) = (v.c.x(EBP), v.c.x(EBX));
    v.push16(bp);
    v.push16(bx);
    v.push16(t);
    let di = v.c.x(EDI);
    let mut d = [0u32; 3];
    for (i, k) in [0x14u16, 0x18, 0x1c].into_iter().enumerate() {
        d[i] = v.ss32(di.wrapping_add(k)).wrapping_sub(v.ss32(bx.wrapping_add(k)));
    }
    v.set_ds32(0x1f5f, d[0]);
    v.set_ds32(0x1f63, d[1]);
    v.set_ds32(0x1f67, d[2]);
    // Only 16-bit halves of these change from here on.
    v.c.r[EAX] = d[0];
    v.c.r[ECX] = d[1];
    v.c.r[EDX] = d[2];
    n += 12;
    // How far to shift so every component's magnitude is below 3F00h.
    let mut count: u16 = 0;
    n += 1; // XOR BX,BX
    for (i, k) in [2usize, 0, 1].into_iter().enumerate() {
        let (mut ax, mut dx) = (d[k] as u16, (d[k] >> 16) as u16);
        n += 4 + abs32(&mut dx, &mut ax); // MOV AX; MOV DX; OR DX,DX; JNS (+ negate)
        if i > 0 {
            n += 2; // MOV CX,BX; JCXZ
            for _ in 0..count {
                let v = ((dx as u32) << 16 | ax as u32) >> 1;
                dx = (v >> 16) as u16;
                ax = v as u16;
            }
            n += 3 * count as u64;
            n += 1; // OR DX,DX (before the JNZ)
        }
        n += shrink(&mut dx, &mut ax, &mut count, 1);
        v.c.set_x(EAX, ax);
        v.c.set_x(EDX, dx);
    }
    // 15d5: shift the three differences by the count and multiply by t.
    let cx = count;
    let (mut x, mut y, mut z) = (d[0], d[1], d[2]);
    n += 1 + 3 + 1; // MOV CX,BX; three MOVs; JCXZ
    if cx != 0 {
        n += 4 + 7 * cx as u64 + 1;
        for _ in 0..cx {
            x = ((x as i32) >> 1) as u32;
            y = ((y as i32) >> 1) as u32;
            z = ((z as i32) >> 1) as u32;
        }
        v.push16(cx);
        v.pop16();
        v.c.set_x(EDX, (x >> 16) as u16);
        v.c.set_x(EDI, (y >> 16) as u16);
        v.c.set_x(ESI, (z >> 16) as u16);
    }
    // XCHG BP,CX; POP [1f5f]; IMUL [1f5f]; MOV SI,DX; MOV DI,AX; MOV AX,[1f5f]; IMUL BX;
    // XCHG DX,CX; MOV BX,AX; MOV AX,[1f5f]; IMUL DX
    let t = v.pop16();
    v.set_ds16(0x1f5f, t);
    let p = |a: u16| (a as i16 as i32 * t as i16 as i32) as u32;
    let (mut px, mut py, mut pz) = (p(x as u16), p(y as u16), p(z as u16));
    n += 11;
    // SUB BP,0Fh; JS; JZ: scale back by (count − 15).
    let mut e = cx.wrapping_sub(0xf) as i16;
    n += 3;
    if e > 0 {
        while e != 0 {
            px <<= 1;
            py <<= 1;
            pz <<= 1;
            e -= 1;
            n += 8;
        }
        n += 1; // JMP 1643
    } else if e < 0 {
        n -= 1; // the JZ is not reached
        while e != 0 {
            px = ((px as i32) >> 1) as u32;
            py = ((py as i32) >> 1) as u32;
            pz = ((pz as i32) >> 1) as u32;
            e += 1;
            n += 8;
        }
    }
    let b = v.pop16();
    let rel = |v: &mut V, k: u16, p: u32| p.wrapping_add(v.ss32(b.wrapping_add(k)));
    let (rx, ry, rz) = (rel(v, 0x14, px), rel(v, 0x18, py), rel(v, 0x1c, pz));
    let out = v.ss16(v.c.x(ESP).wrapping_add(4));
    for (k, r) in [(0x14u16, rx), (0x18, ry), (0x1c, rz)] {
        v.set_ss32(out.wrapping_add(k), r);
    }
    v.c.set_x(EDI, rx as u16);
    v.c.set_x(ESI, (rx >> 16) as u16);
    v.c.set_x(EBX, ry as u16);
    v.c.set_x(ECX, (ry >> 16) as u16);
    v.c.set_x(EAX, rz as u16);
    v.c.set_x(EDX, (rz >> 16) as u16);
    v.c.set_x(EBP, out);
    let bp = v.pop16();
    v.c.set_x(EBP, bp);
    n += 1 + 6 + 2 + 6 + 1;
    pop_and_finish(v, n)
}

/// `EL2MATH3:1673` (types above 20): rotate the definition bytes like a plain vertex, then
/// [`normalise`] the result to the length `[FS:DI+16h]`·7F00h (shifted like the coordinates,
/// with `FS:DI` from `[BP-4]`); the partner is its negation.
fn along(v: &mut V) -> u64 {
    let mut n: u64 = 2;
    if v.c.x(EDX) != v.ds16(0x1f6b) {
        let (bx, di) = (v.c.x(EBX), v.c.x(EDI));
        v.c.set_x(EBX, di);
        v.c.set_x(EDI, bx);
        n += 1;
    }
    let mark = v.ds16(vars::MARK);
    v.c.set_x(EBX, mark);
    let di = v.c.x(EDI);
    v.set_ss16(di.wrapping_add(0x32), mark);
    let (cl, dh, dl) = (v.c.l(ECX), v.c.h(EDX), v.c.l(EDX));
    v.c.set_x(EAX, (cl as u16) << 8);
    v.c.set_x(EBX, (dh as u16) << 8);
    v.c.set_x(ECX, (dl as u16) << 8);
    let bp = v.c.x(EBP);
    v.push16(bp);
    v.push16(di);
    n += 2 + 6 + 2;
    let p = [sx16(v.c.x(EAX)), sx16(v.c.x(EBX)), sx16(v.c.x(ECX))];
    let r = rotate(v, p);
    n += 30;
    let cl = 0x17u16.wrapping_sub(v.ss16(v.bp(frame::SCALE)));
    v.c.r[ECX] = (r.edx & 0xFFFF_0000) | cl as u32;
    let sh = (cl & 31) as u32;
    let sar = |x: u32| ((x as i32) >> sh) as u32;
    let (eax, ebx, edx) = (sar(r.eax), sar(r.ebx), sar(r.edx));
    n += 6;
    let (pdi, fs) = v.far_ptr(v.bp(0xfffc));
    v.c.fs = fs;
    let len = v.bus.read16(lin(fs, pdi.wrapping_add(0x16))) as u32;
    let edi = len.wrapping_mul(0x7f00) >> sh;
    n += 5;
    v.c.r[EAX] = eax;
    v.c.r[EBX] = ebx;
    v.c.r[ECX] = edx;
    v.c.r[EDX] = edx;
    v.c.r[EDI] = edi;
    // CALL 2263 (near)
    v.push16(0x173b);
    let m = normalise(v);
    if m == DECLINE {
        return DECLINE;
    }
    v.pop16();
    n += 1 + m;
    let rec = v.pop16();
    v.c.set_x(EBP, rec);
    let mut w = [v.c.r[EAX], v.c.r[EBX], v.c.r[ECX]];
    for (k, x) in [0x14u16, 0x18, 0x1c].into_iter().zip(w) {
        v.set_ss32(rec.wrapping_add(k), x);
    }
    for x in w.iter_mut() {
        *x = x.wrapping_neg();
    }
    for (k, x) in [0x34u16, 0x38, 0x3c].into_iter().zip(w) {
        v.set_ss32(rec.wrapping_add(k), x);
    }
    v.c.r[EAX] = w[0];
    v.c.r[EBX] = w[1];
    v.c.r[ECX] = w[2];
    let bp = v.pop16();
    v.c.set_x(EBP, bp);
    n += 1 + 9 + 1;
    pop_and_finish(v, n)
}

/// `SHL v,1; SUB v,d; JNC; ADD v,d; RCL q,1; JNC` from `q` = 1 until the marker bit leaves `q`
/// (32 rounds), then `NOT q; SHR q,1`: a restoring division of `v`·2^31 by `d`. Returns
/// (quotient, the instructions without the set-up).
fn divide31(mut x: u32, d: u32) -> (u32, u64) {
    let mut q: u32 = 1;
    let mut n = 0;
    loop {
        x <<= 1;
        let (r, borrow) = x.overflowing_sub(d);
        x = r;
        n += 3;
        if borrow {
            x = x.wrapping_add(d);
            n += 1;
        }
        let out = q >> 31;
        q = q << 1 | borrow as u32;
        n += 2;
        if out == 1 {
            break;
        }
    }
    (!q >> 1, n + 2)
}

/// `(hi·lo·2) >> 16` + hi² of |v| (`EL2MATH3:2269`…): v²/2^32, roughly. 11 or 13 instructions.
fn square_hi(v: u32) -> (u32, u64) {
    let (a, n) = if (v as i32) < 0 { (v.wrapping_neg(), 13) } else { (v, 11) };
    let hi = a >> 16;
    let lo = a & 0xffff;
    let cross = (lo.wrapping_mul(hi) << 1) >> 16;
    (hi.wrapping_mul(hi).wrapping_add(cross), n)
}

/// `(|v| · EDI) >> 15` by 16-bit halves (`EL2MATH3:23eb`…), with v's sign. 23 or 25.
fn scale_by(v: &mut V, x: u32) -> (u32, u64) {
    let edi = v.c.r[EDI];
    let (a, mut n) = if (x as i32) < 0 { (x.wrapping_neg(), 24) } else { (x, 23) };
    let hi = a >> 16;
    let ebp = hi.wrapping_mul(edi & 0xffff);
    let eh = edi >> 16;
    let mut r = (a & 0xffff).wrapping_mul(eh).wrapping_add(ebp) >> 16;
    let edx = hi.wrapping_mul(eh);
    r = r.wrapping_add(edx) << 1;
    v.c.r[EDX] = edx;
    v.c.r[EBP] = ebp;
    v.c.r[ESI] = x;
    if (x as i32) < 0 {
        r = r.wrapping_neg();
        n += 1;
    }
    (r, n)
}

/// Near routine `EL2MATH3:2263`: scale (`EAX`, `EBX`, `ECX`) to length `EDI`. The length is a
/// square root by Newton's method (`EBP` from s/2 + 40000000h, halving towards s/`EBP` until
/// they agree within 8000h), each component is divided by it bit by bit and multiplied by
/// `EDI`. `EBP` and `EDI` are preserved. Returns the instructions including the `RET`.
fn normalise(v: &mut V) -> u64 {
    let (eax, ebx, ecx) = (v.c.r[EAX], v.c.r[EBX], v.c.r[ECX]);
    let mut n: u64 = 3;
    let (sa, na) = square_hi(eax);
    let (sb, nb) = square_hi(ebx);
    let (sc, nc) = square_hi(ecx);
    n += na + nb + nc;
    let s = sa.wrapping_add(sb).wrapping_add(sc);
    n += 3 + 3 + 1 + 1 + 1 + 3;
    let s2 = s << 1;
    let ebp_saved = v.c.r[EBP];
    let mut ebp = (s2 >> 1).wrapping_add(0x4000_0000);
    let mut rounds = 0;
    let mut esi;
    loop {
        let (q, m) = divide31(s2, ebp);
        n += 2 + m;
        esi = q;
        ebp = ebp.wrapping_add(esi) >> 1;
        esi = esi.wrapping_sub(ebp);
        n += 2 + 3;
        if (esi as i32) >= -0x8000 {
            break;
        }
        rounds += 1;
        if rounds > 1000 {
            return DECLINE;
        }
    }
    n += 1; // POP EAX
    let mut out = [0u32; 3];
    for (i, x) in [eax, ebx, ecx].into_iter().enumerate() {
        let a = if (x as i32) < 0 { x.wrapping_neg() } else { x };
        let (q, m) = divide31(a, ebp);
        n += 3 + ((x as i32) < 0) as u64 + 1 + m + 2 + 1;
        let q = if (x as i32) < 0 {
            n += 1;
            q.wrapping_neg()
        } else {
            q
        };
        out[i] = q;
        if i == 2 {
            v.c.r[ESI] = x;
        }
    }
    v.c.r[EBP] = ebp;
    let mut res = [0u32; 3];
    for i in 0..3 {
        let (r, m) = scale_by(v, out[i]);
        res[i] = r;
        n += m;
    }
    v.c.r[EAX] = res[0];
    v.c.r[EBX] = res[1];
    v.c.r[ECX] = res[2];
    v.c.r[EBP] = ebp_saved;
    // What the pushes leave below the stack pointer: ECX from the first three, then EBP, and
    // the PUSH EAX slot reused by the PUSH EDI pairs.
    let e = v.c.x(ESP);
    v.set_ss32(e.wrapping_sub(12), ecx);
    v.set_ss32(e.wrapping_sub(8), v.c.r[EDI]);
    v.set_ss32(e.wrapping_sub(4), ebp_saved);
    n + 2
}
