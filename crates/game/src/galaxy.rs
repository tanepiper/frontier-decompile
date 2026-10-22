//! Galaxy generation (docs/GALAXY.md).
//!
//! The galaxy is 8192 × 8192 sectors (13-bit coordinates). How many stars a sector holds comes
//! from a 128-wide density map in `EL2M19` (one cell per 64 × 64 sectors), interpolated
//! bilinearly inside the cell, perturbed by a hashed noise table and scaled, by
//! `EL2M02:3b34` ([`sector_density_port`]).
//!
//! The ports read the tables from the overlays as they sit in memory (nothing of the game is
//! embedded here) and are register- and memory-exact with the original's instruction counts.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};

/// Main data segment: module table entry for slot 19 (`EL2M19`), `DS:0396 + 13h·8`.
const MODULE_19: u16 = 0x0396 + 0x98;
/// Lock count byte of slot 19, `DS:0596 + 13h`.
const LOCK_19: u16 = 0x05a9;
/// `EL2M19` handler `+20` (`EL2M19:4126`): the density map lookup. Any other target (the lazy
/// load thunk while the module is not resident) makes the port decline.
const M19_LOOKUP: u16 = 0x4126;
/// Offsets in `EL2M19` of the map row read by the lookup and the row below it.
const MAP: u16 = 0x00a3;
const MAP_NEXT_ROW: u16 = 0x0123;
/// `EL2M02` tables: 128 noise words, and the per-call scale words indexed by `CX`.
const NOISE: u16 = 0x1b92;
const SCALE: u16 = 0x1c90;
/// Scratch dword the original accumulates into, `DS:07c0`.
const ACC: u16 = 0x07c0;

fn near_ret(c: &mut Cpu, bus: &mut dyn Bus) {
    let sp = c.x(ESP);
    c.ip = bus.read16(lin(c.ss, sp));
    c.set_x(ESP, sp.wrapping_add(2));
}

/// The bilinear density at sector (`x`, `y`) given the 2×2 map cell `m` = [top-left, top-right,
/// bottom-left, bottom-right] and the position inside the cell (`fx`, `fy` = 9-bit fractions
/// shifted left 6). Returns (`AX`, and the scratch registers the original leaves:
/// `BX`, `CX`, `DX`, `SI`, the `DS:07c0` dword).
fn interpolate(m: [u8; 4], fx: u16, fy: u16) -> (u16, [u16; 4], u32) {
    let m00 = m[0] as u16;
    let bx = (m[1] as u16).wrapping_sub(m00);
    let dx0 = m[2] as u16;
    let cx = (m[3] as u16).wrapping_sub(bx).wrapping_sub(dx0);
    let dx = dx0.wrapping_sub(m00);
    let imul = |a: u16, b: u16| (a as i16 as i32).wrapping_mul(b as i16 as i32) as u32;
    let mut acc = imul(fy, dx);
    acc = acc.wrapping_add(imul(bx, fx));
    let si = (imul(fy, fx) << 1 >> 16) as u16;
    let mut sum = imul(cx, si).wrapping_add(acc);
    sum <<= 1;
    let dx_out = ((sum >> 16) as u16).wrapping_add(m00);
    let ax = ((sum >> 8) & 0xFF) as u16 | (dx_out & 0xFF) << 8;
    (ax, [bx, cx, dx_out, si], acc)
}

/// Port of `EL2M02:3b34`, *observed*: the star density of sector (`AX` = x, `BX` = y), with
/// `CX` selecting the noise scale. A near routine.
///
/// - `AH` ≥ 20h or `BH` ≥ 20h (outside the 8192-sector galaxy): `AX` = 0.
/// - Otherwise the 2×2 cell at map index `((y·8 & FE00h) + (x·8 >> 7)) >> 2` is fetched through
///   `EL2M19`'s handler `+20` (the module must be resident; otherwise the port declines and the
///   original loads it), interpolated by the position in the cell (`x·8 & 1FFh`, `y·8 & 1FFh`,
///   each shifted left 6) into `AX`.
/// - `CX` ≥ 12h: returned as is. Otherwise noise `N = EL2M02:1b92[((xfrac + AX) xor SI) >> 4 & FEh]`;
///   `CX` = 0 scales `AX` by `N / 65536`, else by `not(N × EL2M02:1c90[CX] / 65536) / 65536`.
pub fn sector_density_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (x, y, sel) = (c.x(EAX), c.x(EBX), c.x(ECX));
    if x >> 8 >= 0x20 {
        c.set_x(EAX, 0);
        near_ret(c, bus);
        return 6;
    }
    if y >> 8 >= 0x20 {
        c.set_x(EAX, 0);
        near_ret(c, bus);
        return 8;
    }
    // LES BP,[DS:0396 + 98h]; CALLF ES:[BP + 20h] — the module header of slot 19.
    let hdr_off = bus.read16(lin(c.ds, MODULE_19));
    let hdr_seg = bus.read16(lin(c.ds, MODULE_19 + 2));
    let target_off = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x20)));
    let target_seg = bus.read16(lin(hdr_seg, hdr_off.wrapping_add(0x22)));
    if target_off != M19_LOOKUP {
        return DECLINE;
    }
    let ax8 = x << 3;
    let bx8 = y << 3;
    let fx = (ax8 & 0x1ff) << 6;
    let fy = (bx8 & 0x1ff) << 6;
    let idx = ((bx8 & 0xfe00).wrapping_add(ax8 >> 7)) >> 2;
    let cell = [
        bus.read8(lin(target_seg, idx.wrapping_add(MAP))),
        bus.read8(lin(target_seg, idx.wrapping_add(MAP).wrapping_add(1))),
        bus.read8(lin(target_seg, idx.wrapping_add(MAP_NEXT_ROW))),
        bus.read8(lin(target_seg, idx.wrapping_add(MAP_NEXT_ROW).wrapping_add(1))),
    ];
    // INC/DEC of the lock byte around the call leave it unchanged.
    let lock = bus.read8(lin(c.ds, LOCK_19));
    bus.write8(lin(c.ds, LOCK_19), lock);
    let (mut ax, [bx, cx, dx, si], acc) = interpolate(cell, fx, fy);
    bus.write32(lin(c.ds, ACC), acc);
    c.es = hdr_seg;
    c.set_x(EBX, bx);
    c.set_x(ECX, cx);
    c.set_x(ESI, si);
    c.set_x(EBP, sel);
    let mut di = fx;
    let mut dxo = dx;
    let steps;
    if sel as i16 >= 0x12 {
        steps = 66;
    } else {
        let h = ((di.wrapping_add(ax) ^ si) >> 4) & 0xfe;
        di = bus.read16(lin(c.cs, NOISE.wrapping_add(h)));
        if sel == 0 {
            let p = ax as u32 * di as u32;
            ax = (p >> 16) as u16;
            dxo = ax;
            steps = 75;
        } else {
            let bp = sel << 1;
            c.set_x(EBP, bp);
            let t = bus.read16(lin(c.cs, SCALE.wrapping_add(bp)));
            di = !((di as u32 * t as u32 >> 16) as u16);
            let p = ax as u32 * di as u32;
            ax = (p >> 16) as u16;
            dxo = ax;
            steps = 81;
        }
    }
    c.set_x(EAX, ax);
    c.set_x(EDX, dxo);
    c.set_x(EDI, di);
    near_ret(c, bus);
    steps
}

/// Offsets of the sector seed in the main data segment: `A` = `DS:2155:2153`, `B` = `DS:2159:2157`.
pub const SEED_A: u16 = 0x2153;
pub const SEED_B: u16 = 0x2157;

/// One step of the sector seed (`EL2M02:3abe`): `B += A; A = rol(A, 3); A += B; B = rol(B, 5)`.
/// Returns the new (`A`, `B`).
pub fn seed_step(a: u32, b: u32) -> (u32, u32) {
    let b = b.wrapping_add(a);
    let a = a.rotate_left(3).wrapping_add(b);
    (a, b.rotate_left(5))
}

/// Port of `EL2M02:3abe`, *observed*: advance the sector seed in `DS:2153`–`215a` by
/// [`seed_step`] and return the new `A` in `DX:AX`. `BX` and `CX` are pushed and popped, so
/// they are preserved. A near routine of 41 instructions.
pub fn seed_step_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (pa, pb) = (lin(c.ds, SEED_A), lin(c.ds, SEED_B));
    let (a, b) = seed_step(bus.read32(pa), bus.read32(pb));
    bus.write32(pa, a);
    bus.write32(pb, b);
    c.set_x(EAX, a as u16);
    c.set_x(EDX, (a >> 16) as u16);
    near_ret(c, bus);
    41
}

/// Sector coordinates the generator reads: `DS:2149` (x) and `DS:214d` (y).
pub const SECTOR_X: u16 = 0x2149;
pub const SECTOR_Y: u16 = 0x214d;
/// `EL2M02` tables used by the sector generator: the catalogue of hand-made sectors (47 entries of
/// x, y words ending at `015e`), their star ranges (`0162`), catalogue star records (`0246`) and
/// extras (`041a`), and the star-type tables indexed by the seed (`1cb2`, `1cd2`).
const CATALOGUE_END: u16 = 0x015e;
const CATALOGUE_RANGES: u16 = 0x0162;
const CATALOGUE_STARS: u16 = 0x0246;
const CATALOGUE_EXTRA: u16 = 0x041a;
const STAR_KIND_A: u16 = 0x1cb2;
const STAR_KIND_B: u16 = 0x1cd2;
const CATALOGUE_EXTRA_MODEL: u16 = 0x1cf2;

/// Port of `EL2M02:3c2f`, *observed*: build the star list of the sector at (`DS:2149`,
/// `DS:214d`) in the caller's frame (`SS:BP`). A near routine.
///
/// `SI:DI += DS:2151:214f`; `AX`, `DX`, `BX`, `CX`, `DI`, `SI` are saved at `[BP+14h]`…`[BP+1eh]`
/// and the sector at `[BP+76h]`/`[BP+78h]`. If the sector is one of the 47 catalogue sectors, its
/// stars are copied (byte-swapped words) from the catalogue. Otherwise the star count is
/// `density >> 10` (the high byte of [`sector_density_port`]'s result, `>> 2`), at most 62; the
/// sector seed is set to `A = (x << 16) | y`, `B = (y << 16) | x` and stepped three times; then each
/// star takes one [`seed_step`]: its position words from `A` (`DX` with `DH = 9`, `AX` halved with
/// its low byte halved again) at `[BP+FAh]` + 4 after a `0900h, 0` header, and its kind bytes
/// from `EL2M02:1cb2[B.lo & 1F]` and `1cd2[B.hi & 1F]` at `[BP+7Ch]`. `[BP+7Ah]` = stars + 1 (0
/// for an empty sector), `[BP+5Ah]` = 4.
///
/// When bit 7 of `DS:215b` is set the original then hands the list to the renderer with
/// `CALLF [DS:00f0]` (`[DS:07e4]` = the list, `SI` = `BP`, `ES` = `SS`). The port does the same
/// set-up and ends by entering that routine with the original's return address pushed
/// (`3d20` or `3dbe`), so the original's tail runs after it. An empty catalogue sector with an
/// extra model (`1cf2`) makes the port decline.
pub fn sector_stars_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let ds = c.ds;
    let ss = c.ss;
    let bp = c.x(EBP);
    let at = |off: u16| lin(ss, bp.wrapping_add(off));
    let mut steps: u64 = 0;
    // ADD DI,[214f]; ADC SI,[2151]
    let sidi = ((c.x(ESI) as u32) << 16 | c.x(EDI) as u32).wrapping_add(bus.read32(lin(ds, 0x214f)));
    let (mut si, mut di) = ((sidi >> 16) as u16, sidi as u16);
    let (ax0, dx0, bx0, cx0) = (c.x(EAX), c.x(EDX), c.x(EBX), c.x(ECX));
    let (x, y) = (bus.read16(lin(ds, SECTOR_X)), bus.read16(lin(ds, SECTOR_Y)));
    bus.write16(at(0x14), ax0);
    bus.write16(at(0x16), dx0);
    bus.write16(at(0x18), bx0);
    bus.write16(at(0x1a), cx0);
    bus.write16(at(0x1c), di);
    bus.write16(at(0x1e), si);
    bus.write16(at(0x76), x);
    bus.write16(at(0x78), y);
    steps += 14;
    // Catalogue search: entries from 015a down to 00a2, CX from 2Eh down to -1.
    let cs = c.cs;
    let mut cx: u16 = 0x2e;
    si = CATALOGUE_END;
    let mut hit = None;
    loop {
        si = si.wrapping_sub(4);
        if x != bus.read16(lin(cs, si)) {
            steps += 5;
        } else if y != bus.read16(lin(cs, si.wrapping_add(2))) {
            steps += 7;
        } else {
            steps += 5;
            hit = Some(cx);
            break;
        }
        cx = cx.wrapping_sub(1);
        if cx & 0x8000 != 0 {
            break;
        }
    }
    let (mut ax, mut bx, mut dx);
    if let Some(i) = hit {
        let bxi = i << 1;
        let start = bus.read16(lin(cs, CATALOGUE_RANGES.wrapping_add(bxi)));
        di = start.wrapping_add(CATALOGUE_EXTRA);
        si = CATALOGUE_STARS.wrapping_add(start << 1);
        dx = start;
        let end = bus.read16(lin(cs, CATALOGUE_RANGES.wrapping_add(bxi).wrapping_add(2)));
        cx = end.wrapping_sub(dx) >> 1;
        steps += 14;
        if cx == 0 {
            if bus.read16(lin(cs, CATALOGUE_EXTRA_MODEL.wrapping_add(bxi))) != 0 {
                // The original then draws the extra model through DS:00f4.
                return DECLINE;
            }
            bus.write16(at(0x7a), 0);
            // JMP 3dbf: POP BX; MOV AX,CS:[BX+1cf2]; OR AX,AX; JZ; RET
            c.set_x(EAX, 0);
            c.set_x(EBX, bxi);
            c.set_x(ECX, 0);
            c.set_x(EDX, dx);
            c.set_x(ESI, si);
            c.set_x(EDI, di);
            near_ret(c, bus);
            return steps + 2 + 4 + 1;
        }
        bus.write16(at(0x5a), 4);
        cx = cx.wrapping_add(1);
        bus.write16(at(0x7a), cx);
        cx = cx.wrapping_sub(2);
        bx = bp.wrapping_add(0xfa);
        let mut bp2 = bp.wrapping_add(0x7c);
        bus.write16(lin(ss, bx), 0x0900);
        bus.write16(lin(ss, bx.wrapping_add(2)), 0);
        bx = bx.wrapping_add(4);
        steps += 10;
        loop {
            ax = bus.read16(lin(cs, si)).swap_bytes();
            bus.write16(lin(ss, bx), ax);
            ax = bus.read16(lin(cs, si.wrapping_add(2))).swap_bytes();
            bus.write16(lin(ss, bx.wrapping_add(2)), ax);
            si = si.wrapping_add(4);
            bx = bx.wrapping_add(4);
            ax = bus.read16(lin(cs, di)).swap_bytes();
            bus.write16(lin(ss, bp2), ax);
            di = di.wrapping_add(2);
            bp2 = bp2.wrapping_add(2);
            steps += 15;
            cx = cx.wrapping_sub(1);
            if cx & 0x8000 != 0 {
                break;
            }
        }
        // POP BP; TEST [215b],80h; JZ 3dd9; ADD SP,2; RET
        c.set_x(EAX, ax);
        c.set_x(EBX, bx);
        c.set_x(ECX, cx);
        c.set_x(EDX, dx);
        c.set_x(ESI, si);
        c.set_x(EDI, di);
        if bus.read16(lin(ds, 0x215b)) & 0x80 != 0 {
            // The pushed CX (catalogue index × 2) stays on the stack for the tail's POP BX.
            let sp = c.x(ESP).wrapping_sub(2);
            bus.write16(lin(ss, sp), bxi);
            c.set_x(ESP, sp);
            return steps + 3 + hand_to_renderer(c, bus, 0x3dbe);
        }
        near_ret(c, bus);
        return steps + 3 + 2;
    }
    // Procedural sector: MOV AX,[2149]; MOV BX,[214d]; XOR CX,CX; PUSH BP; CALL 3b34
    let mut sub = *c;
    sub.set_x(EAX, x);
    sub.set_x(EBX, y);
    sub.set_x(ECX, 0);
    let sp = sub.x(ESP).wrapping_sub(4);
    bus.write16(lin(ss, sp.wrapping_add(2)), bp);
    bus.write16(lin(ss, sp), 0x3c7c);
    sub.set_x(ESP, sp);
    let d = sector_density_port(&mut sub, bus);
    if d == DECLINE {
        return DECLINE;
    }
    steps += 5 + d;
    // POP BP; XOR CH,CH; MOV CL,AH; SHR CX,2; JZ
    c.es = sub.es;
    ax = sub.x(EAX);
    bx = sub.x(EBX);
    dx = sub.x(EDX);
    si = sub.x(ESI);
    di = sub.x(EDI);
    cx = (ax >> 8) >> 2;
    steps += 5;
    if cx == 0 {
        bus.write16(at(0x7a), 0);
        c.set_x(EAX, ax);
        c.set_x(EBX, bx);
        c.set_x(ECX, cx);
        c.set_x(EDX, dx);
        c.set_x(ESI, si);
        c.set_x(EDI, di);
        near_ret(c, bus);
        return steps + 2;
    }
    bus.write16(at(0x5a), 4);
    steps += 3;
    if cx as i16 >= 0x3f {
        cx = 0x3e;
        steps += 1;
    }
    di = bp.wrapping_add(0xfa);
    let (pa, pb) = (lin(ds, SEED_A), lin(ds, SEED_B));
    let mut a = (x as u32) << 16 | y as u32;
    let mut b = (y as u32) << 16 | x as u32;
    steps += 7;
    for _ in 0..3 {
        (a, b) = seed_step(a, b);
        steps += 42;
    }
    si = bp.wrapping_add(0x7c);
    cx = cx.wrapping_add(1);
    bus.write16(at(0x7a), cx);
    cx = cx.wrapping_sub(2);
    bus.write16(lin(ss, di), 0x0900);
    bus.write16(lin(ss, di.wrapping_add(2)), 0);
    di = di.wrapping_add(4);
    steps += 7;
    loop {
        (a, b) = seed_step(a, b);
        ax = a as u16;
        dx = (a >> 16) as u16;
        ax = ((ax as i16) >> 1) as u16;
        ax = (ax & 0xff00) | (((ax as u8 as i8) >> 1) as u8 as u16);
        dx = (dx & 0x00ff) | 0x0900;
        bus.write16(lin(ss, di), dx);
        bus.write16(lin(ss, di.wrapping_add(2)), ax);
        di = di.wrapping_add(4);
        bx = (b as u16) & 0x1f;
        let lo = bus.read8(lin(cs, STAR_KIND_A.wrapping_add(bx)));
        bx = ((b >> 16) as u16) & 0x1f;
        let hi = bus.read8(lin(cs, STAR_KIND_B.wrapping_add(bx)));
        ax = (hi as u16) << 8 | lo as u16;
        bus.write16(lin(ss, si), ax);
        si = si.wrapping_add(2);
        steps += 58;
        cx = cx.wrapping_sub(1);
        if cx & 0x8000 != 0 {
            break;
        }
    }
    bus.write32(pa, a);
    bus.write32(pb, b);
    c.set_x(EAX, ax);
    c.set_x(EBX, bx);
    c.set_x(ECX, cx);
    c.set_x(EDX, dx);
    c.set_x(ESI, si);
    c.set_x(EDI, di);
    if bus.read16(lin(ds, 0x215b)) & 0x80 != 0 {
        return steps + 2 + hand_to_renderer(c, bus, 0x3d20);
    }
    near_ret(c, bus);
    steps + 2 + 1
}

/// The original's hand-off to the renderer (`EL2M02:3d0a`/`3da8`): `[DS:07e4]` = `BP + FAh`,
/// `[DS:07e6]` = `SS`, `PUSH BP`, `SI` = `BP`, `AX` = `ES` = `SS`, `CALLF [DS:00f0]` returning to
/// `ret`. Leaves the CPU at the renderer's entry; 8 instructions.
fn hand_to_renderer(c: &mut Cpu, bus: &mut dyn Bus, ret: u16) -> u64 {
    let (ds, ss, bp) = (c.ds, c.ss, c.x(EBP));
    bus.write16(lin(ds, 0x07e4), bp.wrapping_add(0xfa));
    bus.write16(lin(ds, 0x07e6), ss);
    let mut sp = c.x(ESP).wrapping_sub(2);
    bus.write16(lin(ss, sp), bp);
    c.set_x(ESI, bp);
    c.set_x(EAX, ss);
    c.es = ss;
    let (off, seg) = (bus.read16(lin(ds, 0x00f0)), bus.read16(lin(ds, 0x00f2)));
    sp = sp.wrapping_sub(2);
    bus.write16(lin(ss, sp), c.cs);
    sp = sp.wrapping_sub(2);
    bus.write16(lin(ss, sp), ret);
    c.set_x(ESP, sp);
    c.cs = seg;
    c.ip = off;
    8
}

/// The seed of a generated star's name (`EL2M02:24ec`), *observed*: from the sector (`x`, `y`)
/// and the star's index `i` in it. `cx += i; dx += cx; cx = rol(cx,3) + dx; dx = rol(dx,5) + cx;
/// dx = rol(dx,4); bx = rol(cx, i) + dx`. Returns (`bx`, `cx` after the hash, `dx`).
pub fn star_name_seed(x: u16, y: u16, i: u16) -> (u16, u16, u16) {
    let mut cx = x.wrapping_add(i);
    let mut dx = y.wrapping_add(cx);
    cx = cx.rotate_left(3).wrapping_add(dx);
    dx = dx.rotate_left(5).wrapping_add(cx);
    dx = dx.rotate_left(4);
    let bx = cx.rotate_left((i & 31) as u32).wrapping_add(dx);
    (bx, cx, dx)
}

/// The three syllable numbers (0–31, into the table of 4-byte syllables at `EL2M02:01c6`) of a
/// generated star's name: `bx & 7Ch`, then after `ror 5` twice. The first letter is capitalised.
pub fn star_name_syllables(x: u16, y: u16, i: u16) -> [u16; 3] {
    let (bx, _, _) = star_name_seed(x, y, i);
    let b2 = bx.rotate_right(5);
    let b3 = b2.rotate_right(5);
    [(bx & 0x7c) >> 2, (b2 & 0x7c) >> 2, (b3 & 0x7c) >> 2]
}

/// Every ported galaxy routine: (`EL2M02` offset, 0 for a near routine with no vector slot, name, port).
pub const PORTS_M02: &[(u16, u16, &str, Port)] = &[
    (0x3abe, 0, "seed_step", seed_step_port),
    (0x3b34, 0, "sector_density", sector_density_port),
    (0x3c2f, 0, "sector_stars", sector_stars_port),
];
