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
    // ADD lo,[BP+k]; ADC hi,[BP+k+2]: two word accesses, each offset wrapping on its own.
    let words = |v: &mut V, at: u16| v.ss16(at) as u32 | (v.ss16(at.wrapping_add(2)) as u32) << 16;
    let rel = |v: &mut V, k: u16, p: u32| p.wrapping_add(words(v, b.wrapping_add(k)));
    let (rx, ry, rz) = (rel(v, 0x14, px), rel(v, 0x18, py), rel(v, 0x1c, pz));
    let out = v.ss16(v.c.x(ESP).wrapping_add(4));
    for (k, r) in [(0x14u16, rx), (0x18, ry), (0x1c, rz)] {
        v.set_ss16(out.wrapping_add(k), r as u16);
        v.set_ss16(out.wrapping_add(k + 2), (r >> 16) as u16);
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

// ---- projection on register pairs and face shading ------------------------------------------

/// `CMP hi,hi; JA t; JC f; CMP lo,lo; JNC t`: (a ≥ b unsigned, instructions).
fn ge32_a(a: u32, b: u32) -> (bool, u64) {
    let (ah, bh) = (a >> 16, b >> 16);
    if ah > bh {
        (true, 2)
    } else if ah < bh {
        (false, 3)
    } else {
        ((a & 0xffff) >= (b & 0xffff), 5)
    }
}

/// `CMP hi,hi; JC f; JA t; CMP lo,lo; JC f`: (a < b unsigned, instructions).
fn lt32_b(a: u32, b: u32) -> (bool, u64) {
    let (ah, bh) = (a >> 16, b >> 16);
    if ah < bh {
        (true, 2)
    } else if ah > bh {
        (false, 3)
    } else {
        ((a & 0xffff) < (b & 0xffff), 5)
    }
}

/// The scaling loops of `EL2MATH3:18c4` (x, `DX:AX`) and `195a` (y, `CX:BX`): while the
/// coordinate `k` is outside −8000h…7FFFh, shift all three right by 2. Returns instructions up
/// to and including the jump back.
fn scale_pairs(v: &mut [u32; 3], k: usize) -> u64 {
    let hi = |v: &[u32; 3]| (v[k] >> 16) as u16;
    let lo = |v: &[u32; 3]| v[k] as u16;
    let mut n: u64 = 2; // OR hi,hi; JS
    if (hi(v) as i16) >= 0 {
        // JNZ loop; OR lo,lo; JNS done
        if hi(v) == 0 {
            n += 3;
            if lo(v) & 0x8000 == 0 {
                return n;
            }
        } else {
            n += 1;
        }
        loop {
            sar2(v);
            n += 12 + 2;
            if hi(v) != 0 {
                continue;
            }
            n += 2;
            if lo(v) & 0x8000 != 0 {
                continue;
            }
            return n + 1;
        }
    }
    // CMP hi,-1; JA; JC loop; CMP lo,8000h; JNC done
    if hi(v) != 0xffff {
        n += 3;
    } else {
        n += 5;
        if lo(v) >= 0x8000 {
            return n;
        }
    }
    loop {
        sar2(v);
        n += 12 + 2;
        if hi(v) != 0xffff {
            continue;
        }
        n += 3;
        if lo(v) < 0x8000 {
            continue;
        }
        return n + 1;
    }
}

/// Port of `EL2MATH3:1763` (slot `DS:0310`): [`project`] for a point held in 16-bit register
/// pairs, x = `DX:AX`, y = `CX:BX`, z = `SI:DI`, with the same routing, scales, table and
/// clamping; the reciprocal is left in `DI`.
pub fn project_pairs(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    #[derive(Clone, Copy)]
    enum At {
        L1771,
        L1789,
        L17b5,
        L17bf,
        L18c4,
        L192a,
        L195a,
    }
    let pair = |h: usize, l: usize| (c.x(h) as u32) << 16 | c.x(l) as u32;
    let mut v = [pair(EDX, EAX), pair(ECX, EBX), pair(ESI, EDI)];
    let neg = |v: &mut [u32; 3]| v[2] = v[2].wrapping_neg();
    let mut n: u64 = 0;
    let (t, k) = ge32_a(v[2], v[0]);
    n += k;
    let mut at = if t { At::L192a } else { At::L1771 };
    loop {
        at = match at {
            At::L1771 => {
                neg(&mut v);
                n += 4;
                let (t, k) = ge32_a(v[2], v[0]);
                n += k;
                if t {
                    At::L18c4
                } else {
                    At::L1789
                }
            }
            At::L18c4 => {
                n += scale_pairs(&mut v, 0);
                At::L1789
            }
            At::L1789 => {
                let (lt, k) = lt32_b(v[2], v[1]);
                n += k;
                if lt {
                    At::L17b5
                } else {
                    neg(&mut v);
                    n += 4;
                    let (lt, k) = lt32_b(v[2], v[1]);
                    n += k;
                    if lt {
                        At::L195a
                    } else {
                        neg(&mut v);
                        n += 4;
                        At::L17b5
                    }
                }
            }
            At::L17b5 => {
                neg(&mut v);
                n += 4;
                At::L17bf
            }
            At::L192a => {
                neg(&mut v);
                n += 4;
                let (lt, k) = lt32_b(v[2], v[1]);
                n += k;
                if lt {
                    At::L17b5
                } else {
                    neg(&mut v);
                    n += 4;
                    let (ge, k) = ge32_a(v[2], v[1]);
                    n += k;
                    if ge {
                        At::L17bf
                    } else {
                        At::L195a
                    }
                }
            }
            At::L195a => {
                n += scale_pairs(&mut v, 1);
                At::L17bf
            }
            At::L17bf => break,
        }
    }
    // 17bf: OR SI,SI; JNZ; CMP DI,4000h; JNC; CMP DI,1000h; JNC; CMP DI,400h; JNC
    let (si, di) = ((v[2] >> 16) as u16, v[2] as u16);
    let (index, shift, near) = if si != 0 || di >= 0x4000 {
        n += if si != 0 { 2 } else { 4 };
        loop {
            let (si, di) = ((v[2] >> 16) as u16, v[2] as u16);
            if si != 0 {
                if (si as i16) < 0 {
                    return DECLINE; // SAR keeps SI at FFFFh: never returns
                }
                n += 2 + 6;
            } else if di & 0x8000 != 0 {
                n += 4 + 6;
            } else {
                n += 4;
                break;
            }
            for x in v.iter_mut() {
                *x = ((*x as i32) >> 1) as u32;
            }
        }
        n += 12;
        (((v[2] as u16) >> 6) << 1, 5, false)
    } else if di >= 0x1000 {
        n += 6 + 12;
        ((di >> 4) << 1, 3, false)
    } else if di >= 0x400 {
        n += 8 + 11;
        ((di >> 2) << 1, 1, true)
    } else {
        n += 8 + 12;
        (di << 1, 0, true)
    };
    let k = bus.read16(lin(c.cs, index.wrapping_add(RECIPROCAL_TABLE as u16)));
    let scale = |a: u16| {
        let (hi, lo) = imul16(a, k);
        if shift == 0 {
            hi << 1 | lo >> 15
        } else {
            ((hi as i16) >> shift) as u16
        }
    };
    let mut ax = scale(v[0] as u16);
    let mut bx = scale(v[1] as u16);
    let dx = bx;
    bus.write16(lin(c.ss, c.x(ESP).wrapping_sub(2)), ax);
    if near {
        n += clamp(&mut ax, &mut bx);
        n += 1;
    }
    c.set_x(EAX, ax);
    c.set_x(EBX, bx);
    c.set_x(ECX, (v[1] >> 16) as u16);
    c.set_x(EDX, dx);
    c.set_x(ESI, (v[2] >> 16) as u16);
    c.set_x(EDI, k);
    c.retf(bus);
    n
}

/// Main data segment words where [`face_shade`] keeps the face normal (x, y, z).
const NORMAL: u16 = 0x1f73;

/// Port of `EL2MATH3:1bcd` (slot `DS:0308`): visibility and shade of a model face. `AX` picks
/// the face's normal in the model data at `ES:[BP-E0h]+AX−4` (bit 1: the mirrored face, x
/// negated): normal bytes (x, y, z)×100h go to `DS:1f73`. Its high bits pick a vertex of the
/// definitions at `[BP-DCh]`/`[BP-DAh]` (x negated for the mirror partner), scaled by
/// `[BP-D4h]` − `[BP-2Ch]` and taken relative to the eye at `BP-32h`. If the dot product with the
/// normal is not negative the face is hidden: `SS:[DI]` = 8000h. Otherwise the dot product of
/// the normal with the light at `BP-2Ah` picks the shade word `SS:[BP-68h + 2k]` (k from its
/// bits 11–13), or 0 if that is not negative. Leaves `FS` as the definitions' segment.
pub fn face_shade(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut v = V { c, bus, depth: 0 };
    let si0 = v.c.x(ESI);
    v.push16(si0);
    let mut si = v.ss16(v.bp(0xff20));
    let mut ax = v.c.x(EAX);
    let mirrored = ax & 2 != 0;
    let mut n: u64 = 4;
    if mirrored {
        ax &= 0xfffd;
        n += 25;
    } else {
        n += 23;
    }
    si = si.wrapping_add(ax);
    let w0 = v.bus.read16(lin(v.c.es, si.wrapping_sub(4)));
    let w1 = v.bus.read16(lin(v.c.es, si.wrapping_sub(2)));
    let mut nx = (w0 & 0xff) << 8;
    if mirrored {
        nx = nx.wrapping_neg();
    }
    let ny = w1 & 0xff00;
    let nz = (w1 & 0xff) << 8;
    let ax = ((w0 >> 7) & 0xff00) | ((w0 >> 7) & 0xfe);
    v.set_ds16(NORMAL, nx);
    v.set_ds16(NORMAL + 2, ny);
    v.set_ds16(NORMAL + 4, nz);
    let si = (ax & 0xfffd).wrapping_add(v.ss16(v.bp(0xff24)));
    let fs = v.ss16(v.bp(0xff26));
    v.c.fs = fs;
    let bx = v.bus.read16(lin(fs, si));
    let dx = v.bus.read16(lin(fs, si.wrapping_add(2)));
    let odd = ax & 2 != 0;
    let mut bl = bx as u8;
    if mirrored != odd {
        // Mirrored face and even vertex, or the other way round: NEG BL.
        bl = bl.wrapping_neg();
        n += if mirrored { 2 } else { 1 };
    }
    // XCHG AX,BX; CBW; XCHG AX,BX; MOV CX,DX; XCHG AX,DX; CBW; XCHG AX,DX; MOV AL,CH; CBW
    let mut p = [bl as i8 as i16 as u16, (dx >> 8) as u8 as i8 as i16 as u16, dx as u8 as i8 as i16 as u16];
    let shift = v.ss16(v.bp(0xff2c)).wrapping_sub(v.ss16(v.bp(0xffd4)));
    n += 12;
    let mut cl = shift as u8;
    if (shift as i16) < 0 {
        cl = cl.wrapping_neg();
        for x in p.iter_mut() {
            *x = sar_cl(*x, cl);
        }
        n += 5;
    } else {
        n += 1;
        if shift != 0 {
            for x in p.iter_mut() {
                let s = (cl & 31) as u32;
                *x = if s >= 16 { 0 } else { *x << s };
            }
            n += 3;
        }
    }
    // Relative to the eye.
    for (i, x) in p.iter_mut().enumerate() {
        *x = x.wrapping_sub(v.ss16(v.bp(0xffce + 2 * i as u16)));
    }
    // CX:SI… the dot product with the normal: z·nz + y·ny + x·nx, in SI:CX.
    let prod = |a: u16, b: u16| (a as i16 as i32 * b as i16 as i32) as u32;
    let dot = prod(p[2], nz).wrapping_add(prod(p[1], ny)).wrapping_add(prod(p[0], nx));
    let last = prod(p[0], nx);
    n += 16;
    let di = v.c.x(EDI);
    v.c.set_x(ECX, dot as u16);
    if (dot as i32) >= 0 {
        v.set_ss16(di, 0x8000);
        let si = v.pop16();
        v.c.set_x(ESI, si);
        v.c.set_x(EAX, last as u16);
        v.c.set_x(EDX, (last >> 16) as u16);
        v.c.set_x(EBX, p[0]);
        v.retf();
        return n + 3;
    }
    let lp = |v: &mut V, k: u16, nk: u16| prod(v.ss16(v.bp(0xffd6 + 2 * k)), nk);
    let light = lp(&mut v, 0, nx).wrapping_add(lp(&mut v, 1, ny)).wrapping_add(lp(&mut v, 2, nz));
    let last = lp(&mut v, 2, nz);
    v.c.set_x(EAX, last as u16);
    v.c.set_x(EDX, (last >> 16) as u16);
    v.c.set_x(ECX, (light >> 16) as u16);
    n += 13;
    if (light as i32) >= 0 {
        v.c.set_x(EBX, light as u16);
        v.set_ss16(di, 0);
        let si = v.pop16();
        v.c.set_x(ESI, si);
        v.retf();
        return n + 3;
    }
    let bx = ((light >> 16) as u16 & 0xff00) | (light as u16 & 0x00ff);
    let k = bx.rotate_left(6) & 0xe;
    let bx = k.wrapping_add(v.c.x(EBP));
    v.c.set_x(EBX, bx);
    let shade = v.ss16(bx.wrapping_sub(0x68));
    v.c.set_x(EAX, shade);
    v.set_ss16(di, shade);
    let si = v.pop16();
    v.c.set_x(ESI, si);
    v.retf();
    n + 8
}

// ---- 32-bit arithmetic on register pairs -----------------------------------------------------

/// Main data segment dword scratch at `DS:7bc` (the float library's work area, *inferred*):
/// the squared length for [`normalise_pairs`].
const WORK: u16 = 0x07bc;
/// Main data segment dword at `DS:7d4`: the length [`normalise_pairs`] scales to.
const TARGET_LENGTH: u16 = 0x07d4;

fn pair(c: &Cpu, hi: usize, lo: usize) -> u32 {
    (c.x(hi) as u32) << 16 | c.x(lo) as u32
}

fn set_pair(c: &mut Cpu, hi: usize, lo: usize, v: u32) {
    c.set_x(hi, (v >> 16) as u16);
    c.set_x(lo, v as u16);
}

/// Port of `EL2MATH3:1ebb` (slot `DS:02f4`): unsigned 32-bit division by shift and subtract.
/// `DX:AX` / `CX:BX` → quotient `DX:AX`, remainder `CX:BX`. The divisor is shifted up while the
/// dividend is still larger (until its top bit is set), then one quotient bit is decided per
/// shift back down. The quotient's high word is built in `DS:7bc`, which is restored after. A
/// zero divisor with a non-zero dividend never returns.
pub fn divide32(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut num = pair(c, EDX, EAX);
    let mut den = pair(c, ECX, EBX);
    if den == 0 && num != 0 {
        return DECLINE;
    }
    let mut bit: u32 = 1;
    let mut q: u32 = 0;
    let mut n: u64 = 4 + 4 + 2; // pushes; XOR BP; MOV [7bc]; MOV DI,1; XOR SI; OR CX,CX; JS
    if (den as i32) >= 0 {
        loop {
            // 1ed1: SHL DI; RCL SI; SHL BX; RCL CX; OR CX,CX; JS 1eef
            bit <<= 1;
            den <<= 1;
            n += 6;
            if (den as i32) < 0 {
                break;
            }
            // CMP DX,CX; JA 1ed1; JC 1ee7; CMP AX,BX; JA 1ed1
            let (nh, dh) = (num >> 16, den >> 16);
            let more = if nh != dh {
                n += if nh > dh { 2 } else { 3 };
                nh > dh
            } else {
                n += 5;
                (num & 0xffff) > (den & 0xffff)
            };
            if !more {
                // 1ee7: SHR SI; RCR DI; SHR CX; RCR BX
                bit >>= 1;
                den >>= 1;
                n += 4;
                break;
            }
        }
    }
    loop {
        // 1eef: CMP DX,CX; JC; JA; CMP AX,BX; JC
        let (lt, k) = lt32_b(num, den);
        n += k;
        if !lt {
            num = num.wrapping_sub(den);
            q = q.wrapping_add(bit);
            n += 4;
        }
        // SHR SI,1; RCR DI,1; JNZ (ZF from SI); OR DI,DI; JNZ
        bit >>= 1;
        n += 3;
        if bit >> 16 == 0 {
            n += 2;
            if bit == 0 {
                break;
            }
        }
        den >>= 1;
        n += 2; // 1eeb: SHR CX,1; RCR BX,1
    }
    set_pair(c, ECX, EBX, num);
    set_pair(c, EDX, EAX, q);
    c.retf(bus);
    n + 9
}

/// Port of `EL2MATH3:1f1f` (slot `DS:02f8`): the 3×3 matrix of words at `ES:BP` (m0…m8) times
/// (`AX`, `BX`, `CX`), each row's 32-bit sum doubled: `DX:AX` = 2(m0·a + m3·b + m6·c), `CX:BX`
/// with m1/m4/m7, `SI:DI` with m2/m5/m8. `DS:7bc` keeps a.
pub fn matrix_vector(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (a, b, cc) = (c.x(EAX), c.x(EBX), c.x(ECX));
    bus.write16(lin(c.ds, WORK), a);
    let m = |bus: &mut dyn Bus, k: u16| bus.read16(lin(c.es, c.x(EBP).wrapping_add(2 * k)));
    let p = |x: u16, y: u16| (x as i16 as i32 * y as i16 as i32) as u32;
    let mut rows = [0u32; 3];
    for (i, r) in rows.iter_mut().enumerate() {
        let i = i as u16;
        let (m0, m3, m6) = (m(bus, i), m(bus, i + 3), m(bus, i + 6));
        *r = p(m0, a).wrapping_add(p(m3, b)).wrapping_add(p(m6, cc)) << 1;
    }
    set_pair(c, EDX, EAX, rows[0]);
    set_pair(c, ECX, EBX, rows[1]);
    set_pair(c, ESI, EDI, rows[2]);
    // PUSH AX; PUSH DX for the first two rows, popped again: left below SP.
    let sp = c.x(ESP);
    for (k, w) in [rows[0] as u16, (rows[0] >> 16) as u16, rows[1] as u16, (rows[1] >> 16) as u16].into_iter().enumerate() {
        bus.write16(lin(c.ss, sp.wrapping_sub(2 * (k as u16 + 1))), w);
    }
    c.retf(bus);
    51
}

/// The 32-round restoring division of [`divide31`] on register pairs: (quotient, rounds that
/// borrowed). Each round is 8 instructions, 10 when it borrows.
fn restoring(x: u32, d: u32) -> (u32, u64) {
    let (q, n) = divide31(x, d);
    // divide31 counts 5 per round + 1 per borrow + 2 for NOT/SHR.
    let borrows = n - 2 - 5 * 32;
    (q, borrows)
}

/// `(|v|·L)`: the 16 × 16-bit products of `EL2MATH3:20e1`, doubled, with v's sign; with the
/// sign tests 28 instructions, plus 4 for each negation.
fn scale_pair(v: u32, l: u32) -> (u32, u64) {
    let neg = (v as i32) < 0;
    let a = if neg { v.wrapping_neg() } else { v };
    let (hq, lq) = (a >> 16, a & 0xffff);
    let (hl, ll) = (l >> 16, l & 0xffff);
    let sum = hq.wrapping_mul(ll).wrapping_add(lq.wrapping_mul(hl));
    let r = hq.wrapping_mul(hl).wrapping_add(sum >> 16) << 1;
    if neg {
        (r.wrapping_neg(), 28 + 8)
    } else {
        (r, 28)
    }
}

/// `|v| / len` by [`restoring`], with v's sign (`EL2MATH3:2097`…): (result, instructions from
/// the sign test to the signed result).
fn divide_signed(v: u32, len: u32) -> (u32, u64) {
    let neg = (v as i32) < 0;
    let a = if neg { v.wrapping_neg() } else { v };
    let (q, borrows) = restoring(a, len);
    let n = 2 + 2 + 32 * 8 + 2 * borrows + 4 + 3 + if neg { 8 } else { 0 };
    (if neg { q.wrapping_neg() } else { q }, n)
}

/// Body of `EL2MATH3:202e` up to its `RETF`. Returns the instructions, or [`DECLINE`].
fn normalise_pairs_body(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let x = pair(c, EDX, EAX);
    let y = pair(c, ECX, EBX);
    let z = pair(c, ESI, EDI);
    let s2 = bus.read32(lin(c.ds, WORK)) << 1;
    let mut n: u64 = 15;
    let mut len = (s2 >> 1).wrapping_add(0x4000_0000);
    let mut rounds = 0;
    loop {
        let (q, borrows) = restoring(s2, len);
        n += 2 + 2 + 32 * 8 + 2 * borrows + 2 + 2 + 2;
        len = len.wrapping_add(q) >> 1;
        let d = q.wrapping_sub(len);
        n += 4 + 2;
        let hi = (d >> 16) as i16;
        if hi < -1 {
            n += 2;
        } else if hi > -1 {
            n += 3;
            break;
        } else {
            n += 5;
            if d & 0xffff >= 0x8000 {
                break;
            }
        }
        rounds += 1;
        if rounds > 1000 {
            return DECLINE;
        }
    }
    bus.write32(lin(c.ds, WORK), len);
    let target = bus.read32(lin(c.ds, TARGET_LENGTH));
    // POPs and stores after the root; PUSH SI/DI/DX; the moves between components and the
    // PUSH CX / PUSH SI before the y and z divisions.
    n += 6 + 3 + 10 + 6 + 2;
    let mut out = [0u32; 3];
    for (i, v) in [x, y, z].into_iter().enumerate() {
        let (q, k) = divide_signed(v, len);
        let (r, m) = scale_pair(q, target);
        out[i] = r;
        n += k + 4 + m;
    }
    n += 2 + 4;
    set_pair(c, EDX, EAX, out[0]);
    set_pair(c, ECX, EBX, out[1]);
    set_pair(c, ESI, EDI, out[2]);
    n
}

/// Port of `EL2MATH3:202e` (slot `DS:0328`): [`normalise`] on register pairs: scale (`DX:AX`,
/// `CX:BX`, `SI:DI`) to the length at `DS:7d4`, given the squared length at `DS:7bc` (doubled,
/// Newton's square root, kept in `DS:7bc`). `BP` is preserved.
pub fn normalise_pairs(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let n = normalise_pairs_body(c, bus);
    if n == DECLINE {
        return DECLINE;
    }
    c.retf(bus);
    n + 1
}

/// `v`²/2^32, roughly, from 16-bit halves of |v| (`EL2MATH3:1fa0`…): (square, instructions).
fn square_pair(v: u32) -> (u32, u64) {
    let neg = (v as i32) < 0;
    let a = if neg { v.wrapping_neg() } else { v };
    let (hi, lo) = (a >> 16, a & 0xffff);
    let cross = (lo.wrapping_mul(hi) << 1) >> 16;
    (hi.wrapping_mul(hi).wrapping_add(cross), 2 + 9 + if neg { 4 } else { 0 })
}

/// Port of `EL2MATH3:1f93` (slot `DS:0324`): normalise (`DX:AX`, `CX:BX`, `SI:DI`) to the length
/// at `DS:7d4`: the squared length from 16-bit halves, then `CALLF [0328]`
/// ([`normalise_pairs`]). `DS:7bc` is restored after.
pub fn length_normalise(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let saved = bus.read32(lin(c.ds, WORK));
    let (sx, a) = square_pair(pair(c, EDX, EAX));
    let (sy, b) = square_pair(pair(c, ECX, EBX));
    let (sz, d) = square_pair(pair(c, ESI, EDI));
    bus.write32(lin(c.ds, WORK), sx.wrapping_add(sy).wrapping_add(sz));
    let n = normalise_pairs_body(c, bus);
    if n == DECLINE {
        return DECLINE;
    }
    bus.write32(lin(c.ds, WORK), saved);
    c.retf(bus);
    9 + a + 6 + b + 6 + d + 2 + 2 + 6 + 1 + n + 1 + 1 + 1
}

// ---- positions, orbits and the 64-bit work area -----------------------------------------------

/// `MOV DI,angle; BTR DI,15; JNC; SHR DI,3; AND DI,FFFEh; MOV r,[DI+5BFFh]; (NEG r; JMP)`: the
/// sine of a 16-bit angle from [`SINE_TABLE`]: (value, index register, instructions after the
/// `BTR`'s `JNC`: 5 for the second half turn, 3 otherwise).
fn sine_reg(ds: u16, bus: &mut dyn Bus, angle: u16) -> (u16, u16, u64) {
    let negative = angle & 0x8000 != 0;
    let idx = ((angle & 0x7fff) >> 3) & 0xfffe;
    let v = bus.read16(lin(ds, idx.wrapping_add(SINE_TABLE)));
    if negative {
        (v.wrapping_neg(), idx, 5)
    } else {
        (v, idx, 3)
    }
}

/// Port of `EL2MATH3:24f2` (slot `DS:02d8`): a direction from two angles, a (`AX`) and b (`BX`):
/// `AX` = −sb·ca, `BX` = sa·sb, `CX` = cb (Q15, `DX` = sb·ca). `SI` and `DI` are preserved.
pub fn direction(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (a, b) = (c.x(EAX), c.x(EBX));
    let (sb, _, n1) = sine_reg(c.ds, bus, b);
    let (cb, _, n2) = sine_reg(c.ds, bus, b.wrapping_add(0x4000));
    let (sa, _, n3) = sine_reg(c.ds, bus, a);
    let (ca, _, n4) = sine_reg(c.ds, bus, a.wrapping_add(0x4000));
    let sasb = qmul(sa, sb).0;
    let sbca = qmul(sb, ca).0;
    c.set_x(EAX, sbca.wrapping_neg());
    c.set_x(EBX, sasb);
    c.set_x(ECX, cb);
    c.set_x(EDX, sbca);
    c.retf(bus);
    2 + 3 + n1 + 3 + n2 + 4 + n3 + 3 + n4 + 13
}

fn ds16(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u16 {
    bus.read16(lin(c.ds, off))
}
fn set_ds16(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u16) {
    bus.write16(lin(c.ds, off), v)
}
fn ds32(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u32 {
    bus.read32(lin(c.ds, off))
}
fn set_ds32(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u32) {
    bus.write32(lin(c.ds, off), v)
}
fn es16(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u16 {
    bus.read16(lin(c.es, c.x(ESI).wrapping_add(off)))
}
fn set_es16(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u16) {
    bus.write16(lin(c.es, c.x(ESI).wrapping_add(off)), v)
}
fn es32(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u32 {
    bus.read32(lin(c.es, c.x(ESI).wrapping_add(off)))
}
fn set_es32(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u32) {
    bus.write32(lin(c.es, c.x(ESI).wrapping_add(off)), v)
}

/// Two word accesses `ES:[SI+off]`, `ES:[SI+off+2]` (each offset wraps at 64 KB on its own).
fn es_words(c: &Cpu, bus: &mut dyn Bus, off: u16) -> u32 {
    es16(c, bus, off) as u32 | (es16(c, bus, off.wrapping_add(2)) as u32) << 16
}
fn set_es_words(c: &Cpu, bus: &mut dyn Bus, off: u16, v: u32) {
    set_es16(c, bus, off, v as u16);
    set_es16(c, bus, off.wrapping_add(2), (v >> 16) as u16);
}

/// A 64-bit value kept as two dwords of the work area (low at `lo`, high at `hi`).
fn work64(c: &Cpu, bus: &mut dyn Bus, lo: u16, hi: u16) -> u64 {
    ds32(c, bus, lo) as u64 | (ds32(c, bus, hi) as u64) << 32
}
fn set_work64(c: &Cpu, bus: &mut dyn Bus, lo: u16, hi: u16, v: u64) {
    set_ds32(c, bus, lo, v as u32);
    set_ds32(c, bus, hi, (v >> 32) as u32);
}

/// The three 64-bit values the orbit routines build in the work area: (low dword, high dword).
const X64: [(u16, u16); 3] = [(0x7bc, 0x7c8), (0x7c0, 0x7cc), (0x7c4, 0x7d0)];

/// `SAR dword [hi],1; RCR …` loops of `EL2MATH3:2649` and `2706`: the three 64-bit values
/// ([7c8]:DX:AX, [7cc]:CX:BX, [7d0]:DI:BP) shifted right while the count at `DS:7d8`, decremented
/// each time, stays non-negative. Returns the low dwords and instructions.
fn shift_down(c: &Cpu, bus: &mut dyn Bus, count: u16, his: [u16; 3]) -> ([u32; 3], u64) {
    let mut v = [0u64; 3];
    for (x, &h) in v.iter_mut().zip(his.iter()) {
        *x = (ds32(c, bus, h) as u64) << 32;
    }
    let mut cnt = count;
    let mut n = 0;
    loop {
        for x in v.iter_mut() {
            *x = ((*x as i64) >> 1) as u64;
        }
        cnt = cnt.wrapping_sub(1);
        n += 11;
        if (cnt as i16) < 0 {
            break;
        }
    }
    set_ds16(c, bus, 0x7d8, cnt);
    for (x, &h) in v.iter().zip(his.iter()) {
        set_ds32(c, bus, h, (x >> 32) as u32);
    }
    ([v[0] as u32, v[1] as u32, v[2] as u32], n)
}

/// Port of `EL2MATH3:258f` (slot `DS:02e8`): a position on an orbit (*inferred*). Two
/// directions ([`direction`]) at the angles (`[7bc]`, `[7c2]` & FFF0h) and (`[7bc]`,
/// that + 10h); their difference goes to `ES:[SI+8Ah]`. The 64-bit result, per axis, is
/// (`[7c0..7c3]` >> 4) · `[7d4]` (high word) · difference + first direction · `[7d4]`, shifted by
/// 3Dh − `[7d8]`, into `ES:[SI+2Ch]` (low dwords) and `ES:[SI+38h]` (high dwords).
pub fn orbit_position(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let a = ds16(c, bus, 0x7bc);
    let b = ds16(c, bus, 0x7c2) & 0xfff0;
    let sp = c.x(ESP);
    // PUSH BX; PUSH AX; CALLF [02d8]
    bus.write16(lin(c.ss, sp.wrapping_sub(2)), b);
    bus.write16(lin(c.ss, sp.wrapping_sub(4)), a);
    let dir = |c: &mut Cpu, bus: &mut dyn Bus, a: u16, b: u16| -> ([u16; 3], u16, u64) {
        let mut t = *c;
        t.set_x(EAX, a);
        t.set_x(EBX, b);
        let (sb, _, n1) = sine_reg(t.ds, bus, b);
        let (cb, _, n2) = sine_reg(t.ds, bus, b.wrapping_add(0x4000));
        let (sa, _, n3) = sine_reg(t.ds, bus, a);
        let (ca, _, n4) = sine_reg(t.ds, bus, a.wrapping_add(0x4000));
        let sbca = qmul(sb, ca).0;
        ([sbca.wrapping_neg(), qmul(sa, sb).0, cb], sbca, 2 + 3 + n1 + 3 + n2 + 4 + n3 + 3 + n4 + 13)
    };
    let (u1, _, k1) = dir(c, bus, a, b);
    // PUSH CX; PUSH BX; PUSH AX; MOV BP,SP
    let bp = sp.wrapping_sub(10);
    for (k, w) in [u1[2], u1[1], u1[0]].into_iter().enumerate() {
        bus.write16(lin(c.ss, sp.wrapping_sub(6 + 2 * k as u16)), w);
    }
    let (u2, _, k2) = dir(c, bus, a, b.wrapping_add(0x10));
    let d = [u2[0].wrapping_sub(u1[0]), u2[1].wrapping_sub(u1[1]), u2[2].wrapping_sub(u1[2])];
    set_es16(c, bus, 0x8a, d[0]);
    set_es16(c, bus, 0x8c, d[1]);
    set_es16(c, bus, 0x8e, d[2]);
    let r32 = ds32(c, bus, 0x7c0) >> 4;
    let l = ds16(c, bus, 0x7d4);
    let r = ((r32 as u16 as u32 * l as u32) >> 16) as u16;
    let p = |x: u16, y: u16| (x as i16 as i32 * y as i16 as i32) as u32;
    let mut acc = [0u32; 3];
    for i in 0..3 {
        acc[i] = p(d[i], r).wrapping_add(p(u1[i], l));
    }
    let last = p(u1[2], l); // DX:AX after the last IMUL
    for (i, &(_, hi)) in X64.iter().enumerate() {
        set_ds32(c, bus, hi, acc[i]);
    }
    let mut n: u64 = 5 + 1 + k1 + 4 + 3 + 1 + k2 + 3 + 3 + 1 + 10 + 1 + 5 + 4 + 4 + 3 + 1 + 3 + 4 + 4;
    let e = ds16(c, bus, 0x7d8);
    n += 2; // SUB word [7d8],3Dh; JG
    if (e as i16) > 0x3d {
        let mut cnt = e.wrapping_sub(0x3d).wrapping_sub(1);
        n += 2; // DEC; JZ
        let mut v = acc;
        while cnt != 0 {
            for x in v.iter_mut() {
                *x <<= 1;
            }
            cnt = cnt.wrapping_sub(1);
            n += 5;
        }
        if e.wrapping_sub(0x3d).wrapping_sub(1) != 0 {
            n += 1; // JMP 266a
        }
        set_ds16(c, bus, 0x7d8, 0);
        for (i, &(_, hi)) in X64.iter().enumerate() {
            set_ds32(c, bus, hi, v[i]);
        }
        // AX…BP keep what the multiplications left.
        let ax = last as u16;
        let dx = (last >> 16) as u16;
        set_es16(c, bus, 0x2c, ax);
        set_es16(c, bus, 0x2e, dx);
        set_es16(c, bus, 0x30, u1[1]);
        set_es16(c, bus, 0x32, u1[2]);
        set_es16(c, bus, 0x34, bp);
        set_es16(c, bus, 0x36, l);
        c.set_x(EAX, ax);
        c.set_x(EDX, dx);
        c.set_x(EBX, u1[1]);
        c.set_x(ECX, u1[2]);
        c.set_x(EBP, bp);
        c.set_x(EDI, l);
    } else {
        let cnt = 0x3du16.wrapping_sub(e);
        n += 1 + 6;
        let (lo, k) = shift_down(c, bus, cnt, [0x7c8, 0x7cc, 0x7d0]);
        n += k;
        set_es_words(c, bus, 0x2c, lo[0]);
        set_es_words(c, bus, 0x30, lo[1]);
        set_es_words(c, bus, 0x34, lo[2]);
        c.set_x(EAX, lo[0] as u16);
        c.set_x(EDX, (lo[0] >> 16) as u16);
        c.set_x(EBX, lo[1] as u16);
        c.set_x(ECX, (lo[1] >> 16) as u16);
        c.set_x(EBP, lo[2] as u16);
        c.set_x(EDI, (lo[2] >> 16) as u16);
    }
    for (k, &(_, hi)) in X64.iter().enumerate() {
        let v = ds32(c, bus, hi);
        set_es32(c, bus, 0x38 + 4 * k as u16, v);
        if k == 2 {
            c.r[EAX] = v;
        }
    }
    c.retf(bus);
    n + 1 + 6 + 6 + 1
}

/// Port of `EL2MATH3:26be` (slot `DS:02ec`): advance a 64-bit position along the stored
/// difference vector (`ES:[SI+8Ah]`, see [`orbit_position`]): t = high word of `AX` ·
/// `ES:[SI+86h]` (unsigned); each axis's d·t (32-bit) shifted right as a 64-bit value by
/// 3Dh − `ES:[SI+88h]` + 1 is added to `ES:[SI+2Ch]`/`[SI+38h]`.
pub fn orbit_advance(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let w = es16(c, bus, 0x86);
    let t = ((c.x(EAX) as u32 * w as u32) >> 16) as u16;
    let p = |x: u16, y: u16| (x as i16 as i32 * y as i16 as i32) as u32;
    for (k, &(_, hi)) in X64.iter().enumerate() {
        let d = es16(c, bus, 0x8a + 2 * k as u16);
        set_ds32(c, bus, hi, p(d, t));
    }
    let cnt = 0x3du16.wrapping_sub(es16(c, bus, 0x88));
    set_ds16(c, bus, 0x7d8, cnt);
    let mut n: u64 = 2 + 4 + 4 + 4 + 3 + 6;
    let (lo, k) = shift_down(c, bus, cnt, [0x7c8, 0x7cc, 0x7d0]);
    n += k;
    for (k, &(_, hi)) in X64.iter().enumerate() {
        let k = k as u16;
        // ADD word; ADC word; ADC dword
        let old = es_words(c, bus, 0x2c + 4 * k) as u64 | (es32(c, bus, 0x38 + 4 * k) as u64) << 32;
        let add = lo[k as usize] as u64 | (ds32(c, bus, hi) as u64) << 32;
        let v = old.wrapping_add(add);
        set_es_words(c, bus, 0x2c + 4 * k, v as u32);
        set_es32(c, bus, 0x38 + 4 * k, (v >> 32) as u32);
        if k == 2 {
            c.r[EAX] = ds32(c, bus, hi);
        }
    }
    c.set_x(EDX, (lo[0] >> 16) as u16);
    c.set_x(EBX, lo[1] as u16);
    c.set_x(ECX, (lo[1] >> 16) as u16);
    c.set_x(EBP, lo[2] as u16);
    c.set_x(EDI, (lo[2] >> 16) as u16);
    c.retf(bus);
    n + 12 + 1
}

/// Near routine `EL2MATH3:277e`: the 64-bit work value A at `DS:7c4` times the 32-bit B at
/// `DS:7cc` (modulo 2^64, skipping zero partial products) into `DS:7bc`, then `DX:CX:BX:AX` =
/// that << 3. Returns the instructions including its `RETF`.
fn multiply64(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let a = work64(c, bus, 0x7c4, 0x7c8);
    let b = ds32(c, bus, 0x7cc) as u64;
    let r = a.wrapping_mul(b);
    set_work64(c, bus, 0x7bc, 0x7c0, r);
    let mut n: u64 = 12;
    if (a >> 16) as u16 != 0 {
        n += 9;
    }
    n += 3;
    if a >> 32 != 0 {
        n += 10;
    }
    let v = r << 3;
    c.set_x(EAX, v as u16);
    c.set_x(EBX, (v >> 16) as u16);
    c.set_x(ECX, (v >> 32) as u16);
    c.set_x(EDX, (v >> 48) as u16);
    n + 17
}

/// Port of `EL2MATH3:275b` (slot `DS:02e4`): A = `DS:7c4` (64-bit) − `DS:7bc` (64-bit), kept
/// in `DS:7c4`; then `DX:CX:BX:AX` = A × `DS:7cc` (32-bit) << 3, signed by A (the magnitude is
/// multiplied, `EL2MATH3:277e`, then negated back).
pub fn delta_times(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let b = work64(c, bus, 0x7bc, 0x7c0);
    let a = work64(c, bus, 0x7c4, 0x7c8).wrapping_sub(b);
    set_work64(c, bus, 0x7c4, 0x7c8, a);
    c.set_x(EAX, b as u16);
    c.set_x(EDX, (b >> 16) as u16);
    c.set_x(EBX, (b >> 32) as u16);
    c.set_x(ECX, (b >> 48) as u16);
    let n: u64 = 9;
    if (a as i64) >= 0 {
        let m = multiply64(c, bus);
        c.retf(bus);
        return n + m;
    }
    set_work64(c, bus, 0x7c4, 0x7c8, a.wrapping_neg());
    let m = multiply64(c, bus);
    let v = (pair(c, EDX, ECX) as u64) << 32 | pair(c, EBX, EAX) as u64;
    let v = v.wrapping_neg();
    c.set_x(EAX, v as u16);
    c.set_x(EBX, (v >> 16) as u16);
    c.set_x(ECX, (v >> 32) as u16);
    c.set_x(EDX, (v >> 48) as u16);
    c.retf(bus);
    n + 4 + 1 + m + 8 + 1
}

/// `EL2MATH3:2898`: the three 32-bit values at `DS:7cc`, `7d0`, `7d4` times the 15-bit t (kept at
/// `DS:7d8`), as 64-bit values in the work area ([`X64`]), each shifted right by 2. 56
/// instructions including the `RETF`.
fn scale_three(c: &mut Cpu, bus: &mut dyn Bus, t: u16) -> u64 {
    set_ds16(c, bus, 0x7d8, t);
    let mut regs = (0u16, 0u16, 0u16);
    for (k, &(lo, hi)) in X64.iter().enumerate() {
        let v = ds32(c, bus, 0x7cc + 4 * k as u16);
        let x = (v as i32 as i64).wrapping_mul(t as i64);
        set_work64(c, bus, lo, hi, x as u64);
        // MOV BX,AX (low word of the high product); AX:DX the sign-extended high part.
        let hp = ((v >> 16) as i16 as i32 * t as i32) as u32;
        let ll = (v & 0xffff) * t as u32;
        let (_, carry) = ((ll >> 16) as u16).overflowing_add(hp as u16);
        let top = ((hp >> 16) as i16 as i32 as u32).wrapping_add(carry as u32);
        regs = (top as u16, (top >> 16) as u16, hp as u16);
    }
    for &(lo, hi) in X64.iter() {
        let x = work64(c, bus, lo, hi);
        set_work64(c, bus, lo, hi, ((x as i64) >> 2) as u64);
    }
    c.set_x(EAX, regs.0);
    c.set_x(EDX, regs.1);
    c.set_x(EBX, regs.2);
    56
}

/// `SHL dword [lo],1; RCL dword [hi],1` for the three values, `count`+1 times (`EL2MATH3:2977`,
/// with its `POP CX` and `RETF`).
fn shift_up(c: &mut Cpu, bus: &mut dyn Bus, count: u16) -> u64 {
    let mut cnt = count;
    let mut n = 1;
    loop {
        for &(lo, hi) in X64.iter() {
            let x = work64(c, bus, lo, hi);
            set_work64(c, bus, lo, hi, x << 1);
        }
        cnt = cnt.wrapping_sub(1);
        n += 8;
        if (cnt as i16) < 0 {
            break;
        }
    }
    c.set_x(ECX, cnt);
    n + 1
}

/// `EL2MATH3:299a`: the low dword of A, already shifted down by 7, is shifted further until it
/// fits 15 bits; [`scale_three`] with it, then the results shifted back up.
fn scale_wide(c: &mut Cpu, bus: &mut dyn Bus, mut v: u32) -> u64 {
    let mut cx: u16 = 0xffff;
    let mut n: u64 = 1;
    loop {
        cx = cx.wrapping_add(1);
        v >>= 1;
        n += 4;
        if v >> 16 != 0 {
            continue;
        }
        n += 2;
        if v & 0x8000 != 0 {
            continue;
        }
        break;
    }
    c.set_x(EDX, (v >> 16) as u16);
    n += 2 + scale_three(c, bus, v as u16) + 1;
    n + shift_up(c, bus, cx)
}

/// `EL2MATH3:286b`: A's low dword shifted down by 7 as t (else [`scale_wide`]).
fn scale_low(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let v = ds32(c, bus, 0x7c4) >> 7;
    c.set_x(EAX, v as u16);
    c.set_x(EDX, (v >> 16) as u16);
    let mut n: u64 = 17;
    if v >> 16 != 0 {
        return n + scale_wide(c, bus, v);
    }
    n += 2;
    if v & 0x8000 != 0 {
        return n + scale_wide(c, bus, v);
    }
    n + scale_three(c, bus, v as u16)
}

/// `EL2MATH3:2860`: A ≥ 0. If its high dword is not zero A is shifted down until it is
/// (`EL2MATH3:2959`), scaled, and the results shifted back up.
fn scale_positive(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let mut n: u64 = 3;
    if ds32(c, bus, 0x7c8) == 0 {
        return n + scale_low(c, bus);
    }
    let mut a = work64(c, bus, 0x7c4, 0x7c8);
    let mut cx: u16 = 0;
    n += 1;
    loop {
        a >>= 1;
        n += 5;
        if a >> 32 == 0 {
            break;
        }
        cx = cx.wrapping_add(1);
        n += 2;
    }
    set_work64(c, bus, 0x7c4, 0x7c8, a);
    c.set_x(EAX, 0);
    n += 2 + scale_low(c, bus);
    n + shift_up(c, bus, cx)
}

/// Port of `EL2MATH3:2a0e` (slot `DS:02f0`): move an object along a vector by a 64-bit
/// amount: the vector `ES:[SI+62h]` (three dwords, copied to `DS:7cc`) times A = `DS:7c4`
/// (64-bit, signed; near routine `2857`) is added to the 64-bit position `ES:[SI+2Ch]`/`[38h]`.
pub fn advance_position(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    for k in 0..3u16 {
        let v = es32(c, bus, 0x62 + 4 * k);
        set_ds32(c, bus, 0x7cc + 4 * k, v);
        c.r[EAX] = v;
    }
    let mut n: u64 = 6 + 2;
    // 2857: CMP word [7ca],0; JS
    n += 2;
    let a = work64(c, bus, 0x7c4, 0x7c8);
    if (a as i64) < 0 {
        set_work64(c, bus, 0x7c4, 0x7c8, a.wrapping_neg());
        n += 4 + 1 + scale_positive(c, bus);
        for &(lo, hi) in X64.iter() {
            let x = work64(c, bus, lo, hi);
            set_work64(c, bus, lo, hi, x.wrapping_neg());
        }
        n += 12 + 1;
    } else {
        n += scale_positive(c, bus);
    }
    for (k, &(lo, hi)) in X64.iter().enumerate() {
        let k = k as u16;
        let old = es32(c, bus, 0x2c + 4 * k) as u64 | (es32(c, bus, 0x38 + 4 * k) as u64) << 32;
        let add = work64(c, bus, lo, hi);
        let v = old.wrapping_add(add);
        set_es32(c, bus, 0x2c + 4 * k, v as u32);
        set_es32(c, bus, 0x38 + 4 * k, (v >> 32) as u32);
        c.r[EAX] = add as u32;
        c.r[EBX] = (add >> 32) as u32;
    }
    c.retf(bus);
    n + 12 + 1
}

/// Port of `EL2MATH3:2bae` (slot `DS:0270`): add the three signed 32-bit values at `DS:7bc`
/// to the 64-bit position `ES:[SI+2Ch]`/`[38h]`.
pub fn add_position(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    for k in 0..3u16 {
        let d = ds32(c, bus, 0x7bc + 4 * k);
        let old = es32(c, bus, 0x2c + 4 * k) as u64 | (es32(c, bus, 0x38 + 4 * k) as u64) << 32;
        let v = old.wrapping_add(d as i32 as i64 as u64);
        set_es32(c, bus, 0x2c + 4 * k, v as u32);
        set_es32(c, bus, 0x38 + 4 * k, (v >> 32) as u32);
        c.r[EAX] = d;
        c.r[EDX] = ((d as i32) >> 31) as u32;
    }
    c.retf(bus);
    16
}

/// Port of `EL2MATH3:2a67` (slot `DS:0318`): rotate two vectors in their common plane by the
/// angle at `DS:7d8`: u (words `ES:[SI]`, `+6`, `+0Ch`) and w (the same at the far pointer
/// `DS:7e0`) become u·cos + w·sin and w·cos − u·sin (Q15). `DS:7d8` and `7d4` keep the sine and
/// cosine.
pub fn rotate_pair(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let angle = ds16(c, bus, 0x7d8);
    let (s, _, n1) = sine_reg(c.ds, bus, angle);
    set_ds16(c, bus, 0x7d8, s);
    let (co, _, n2) = sine_reg(c.ds, bus, angle.wrapping_add(0x4000));
    set_ds16(c, bus, 0x7d4, co);
    let p = |x: u16, y: u16| (x as i16 as i32 * y as i16 as i32) as u32;
    let u = [es16(c, bus, 0), es16(c, bus, 6), es16(c, bus, 0xc)];
    for (k, &x) in u.iter().enumerate() {
        set_ds32(c, bus, 0x7bc + 4 * k as u16, p(x, co));
    }
    let woff = ds16(c, bus, 0x7e0);
    let wseg = ds16(c, bus, 0x7e2);
    let w = [bus.read16(lin(wseg, woff)), bus.read16(lin(wseg, woff.wrapping_add(6))), bus.read16(lin(wseg, woff.wrapping_add(0xc)))];
    for (k, &x) in w.iter().enumerate() {
        let sum = p(s, x).wrapping_add(ds32(c, bus, 0x7bc + 4 * k as u16)) << 1;
        set_es16(c, bus, 6 * k as u16, (sum >> 16) as u16);
    }
    for (k, &x) in w.iter().enumerate() {
        set_ds16(c, bus, 0x7c8 + 4 * k as u16, x);
    }
    for (k, &x) in u.iter().enumerate() {
        set_ds32(c, bus, 0x7bc + 4 * k as u16, p(x, s));
    }
    let woff2 = ds16(c, bus, 0x7e0);
    let wseg2 = ds16(c, bus, 0x7e2);
    let mut last = 0;
    for k in 0..3u16 {
        let wk = ds16(c, bus, 0x7c8 + 4 * k);
        last = p(wk, co).wrapping_sub(ds32(c, bus, 0x7bc + 4 * k)) << 1;
        bus.write16(lin(wseg2, woff2.wrapping_add(6 * k)), (last >> 16) as u16);
    }
    c.set_x(EAX, last as u16);
    c.set_x(EDX, (last >> 16) as u16);
    c.set_x(EBX, u[1]);
    c.set_x(ECX, u[2]);
    c.set_x(EDI, woff2);
    c.retf(bus);
    4 + n1 + 1 + 3 + n2 + 1 + 87
}
