//! Texture-mapped triangles and quadrilaterals (docs/VIDEO.md §5).
//!
//! A fill looks its bitmap up in the cache ([`cache_lookup`]), copies it into a buffer on the
//! stack, 256 bytes a row, through the colour table at `DS:1fb1` and halved in each direction
//! as often as the polygon's screen size allows (`CS:357c`), then walks the edges in screen and
//! texture space: per vertex order a case of set-up code ([`super::texture_cases`]) and the row
//! mapper ([`map_rows`]), which interpolates the texture coordinates linearly along each row.
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.

use super::texture_cases::{QUAD_CASES, TRIANGLE_CASES};
use super::*;

/// Code segment variables of the textured fills (`CS:351a`…): the vertices (x, y words at
/// `351a`… `3528`), the bitmap number (`357a`), the vertex order (`3576`), the orientation
/// (`3574`), the halvings (`357c`), the buffer (`357d`), the bitmap's width and height (`353c`,
/// `353e`), and for the row mapper the edges' x (`352a`, `352e`, 16.16), the row (`3532`), their
/// x steps (`3534`, `3538`), the texture coordinates at both ends (`3540`–`354c`), their steps
/// (`3550`–`355c`), the per-pixel steps (`3560`, `3562`), the edge padding (`357f`, `3581`,
/// `3583`, `3587`, `3589`) and the row count (`3578`).
pub const TEX_BITMAP: u16 = 0x357a;

fn rw(x: &mut Ctx, off: u16) -> u16 {
    x.bus.read16(lin(x.c.cs, off))
}
fn ww(x: &mut Ctx, off: u16, v: u16) {
    x.bus.write16(lin(x.c.cs, off), v)
}
fn rd(x: &mut Ctx, off: u16) -> u32 {
    x.bus.read32(lin(x.c.cs, off))
}
fn wd(x: &mut Ctx, off: u16, v: u32) {
    x.bus.write32(lin(x.c.cs, off), v)
}
fn rb(x: &mut Ctx, off: u16) -> u8 {
    x.bus.read8(lin(x.c.cs, off))
}
fn wb(x: &mut Ctx, off: u16, v: u8) {
    x.bus.write8(lin(x.c.cs, off), v)
}

/// One instruction of the case tables.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    /// `MOV AX,CS:[a]`, `SUB AX,CS:[a]`, `CMP AX,CS:[a]`, `NEG AX`, `MOV CS:[a],AX`.
    LoadAx(u16),
    SubAx(u16),
    CmpAx(u16),
    NegAx,
    StoreAx(u16),
    /// `MOV r,CS:[a]`, `SUB BX,CS:[a]`.
    Load(usize, u16),
    SubBx(u16),
    /// `MOV EAX,CS:[a]`, `MOV EAX,v`, `CMP EAX,v`, `SHL EAX,10h`, `DEC EAX`, `MOV CS:[a],EAX`,
    /// `MOV dword CS:[a],0`.
    LoadEax(u16),
    SetEax(u32),
    CmpEax(u32),
    ShlEax16,
    DecEax,
    StoreEax(u16),
    Clear(u16),
    /// `CMP r,r` (signed).
    CmpRegs(usize, usize),
    /// `CALL 35cf` ([`divide`]), `CALL 5448` ([`map_rows`]).
    Div,
    Map,
    Je(u16),
    Jg(u16),
    Jmp(u16),
    /// `ADD SP,4100h`, `DEC [1fa9]`, `RETF`.
    Free,
    DecBusy,
    Retf,
    /// The quadrilateral's fallback to a flat fill: `MOV [57e9],DI` / `MOV [57ed],DI`,
    /// `XCHG [57e9],SI`, `XCHG [57ed],BP`, `MOVZX DI,[1fb1]`, `MOV [57fd],DI`,
    /// `JMP 1e41` ([`quad_clipped`]).
    StoreDi(u16),
    XchgSi(u16),
    XchgBp(u16),
    FlatColour,
    StoreColour,
    FlatQuad,
    /// `MOV ES,[09e2]`.
    LoadTarget,
}

/// Execute a case table from `start` to its `RETF` (or the flat fallback).
fn run(x: &mut Ctx, table: &[(u16, Op)], start: u16) -> Result<(), Decline> {
    let mut i = table.binary_search_by_key(&start, |e| e.0).map_err(|_| Decline("case entry outside the table"))?;
    let (mut zf, mut gt) = (false, false);
    loop {
        let (addr, op) = table[i];
        i += 1;
        x.steps += 1;
        let jump = |t: u16| table.binary_search_by_key(&t, |e| e.0).map_err(|_| Decline("jump outside the table"));
        match op {
            Op::LoadAx(a) => {
                let v = rw(x, a);
                x.set(EAX, v);
            }
            Op::SubAx(a) => {
                let v = x.ax().wrapping_sub(rw(x, a));
                x.set(EAX, v);
                zf = v == 0;
            }
            Op::CmpAx(a) => {
                let (l, r) = (x.ax(), rw(x, a));
                zf = l == r;
                gt = (l as i16) > (r as i16);
            }
            Op::NegAx => {
                let v = x.ax().wrapping_neg();
                x.set(EAX, v);
            }
            Op::StoreAx(a) => {
                let v = x.ax();
                ww(x, a, v);
            }
            Op::Load(r, a) => {
                let v = rw(x, a);
                x.set(r, v);
            }
            Op::SubBx(a) => {
                let v = x.bx().wrapping_sub(rw(x, a));
                x.set(EBX, v);
                zf = v == 0;
            }
            Op::LoadEax(a) => x.c.r[EAX] = rd(x, a),
            Op::SetEax(v) => x.c.r[EAX] = v,
            Op::CmpEax(v) => {
                zf = x.c.r[EAX] == v;
                gt = (x.c.r[EAX] as i32) > (v as i32);
            }
            Op::ShlEax16 => x.c.r[EAX] <<= 16,
            Op::DecEax => x.c.r[EAX] = x.c.r[EAX].wrapping_sub(1),
            Op::StoreEax(a) => {
                let v = x.c.r[EAX];
                wd(x, a, v);
            }
            Op::Clear(a) => wd(x, a, 0),
            Op::CmpRegs(a, b) => {
                let (l, r) = (x.c.x(a), x.c.x(b));
                zf = l == r;
                gt = (l as i16) > (r as i16);
            }
            Op::Div => {
                x.push(addr.wrapping_add(3));
                divide(x);
                let _ = x.pop();
            }
            Op::Map => {
                x.push(addr.wrapping_add(3));
                map_rows(x)?;
                let _ = x.pop();
            }
            Op::Je(t) => {
                if zf {
                    i = jump(t)?;
                }
            }
            Op::Jg(t) => {
                if gt {
                    i = jump(t)?;
                }
            }
            Op::Jmp(t) => i = jump(t)?,
            Op::Free => {
                let sp = x.sp().wrapping_add(0x4100);
                x.set(ESP, sp);
            }
            Op::DecBusy => {
                let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
                x.ds_ww(BLIT_BUSY, n);
            }
            Op::Retf => {
                x.steps -= 1;
                x.retf();
                return Ok(());
            }
            Op::StoreDi(a) => {
                let v = x.di();
                x.ds_ww(a, v);
            }
            Op::XchgSi(a) => {
                let (m, r) = (x.ds_rw(a), x.si());
                x.ds_ww(a, r);
                x.set(ESI, m);
            }
            Op::XchgBp(a) => {
                let (m, r) = (x.ds_rw(a), x.bp());
                x.ds_ww(a, r);
                x.set(EBP, m);
            }
            Op::FlatColour => {
                let v = x.ds_rb(0x1fb1);
                x.set(EDI, v as u16);
            }
            Op::StoreColour => {
                let v = x.di();
                x.ds_ww(COLOUR, v);
            }
            Op::FlatQuad => return quad_clipped(x),
            Op::LoadTarget => x.c.es = x.ds_rw(TARGET),
        }
    }
}

/// The divider `EL2MCGA3:35cf` (a near routine, with its `RET`): `AX`/`BX` as a 16.16 step in
/// `EAX`. The dividend is shifted left by 8 with its low byte set (FFh after subtracting 1
/// from a positive one, 1 for a negative one) and divided; on overflow it is divided as
/// `AX`·64 and the quotient shifted by 10; if that overflows too, or `BX` is 0, the result is
/// ±7FFFFFFFh by the sign of `AX`.
pub fn divide(x: &mut Ctx) {
    let (num, den) = (x.ax(), x.bx());
    // OR BX,BX; JZ 3623
    x.steps += 2;
    let saturate = |x: &mut Ctx, v: u16| {
        // 3623: OR AX,AX; JS 362e; MOV EAX,7fffffffh / 80000001h; RET
        x.c.r[EAX] = if (v as i16) < 0 { 0x8000_0001 } else { 0x7fff_ffff };
        x.steps += 4;
    };
    if den == 0 {
        saturate(x, num);
        return;
    }
    // MOV CX,AX; OR AX,AX; JS 35e6
    x.set(ECX, num);
    x.steps += 3;
    let dividend = if (num as i16) < 0 {
        // 35e6: MOV DH,0ffh; MOV DL,AH; MOV AH,AL; MOV AL,1
        x.steps += 4;
        0xff00_0000 | (num as u32) << 8 | 1
    } else {
        // JZ 35dc; DEC AX; 35dc: XOR DH,DH; MOV DL,AH; MOV AH,AL; MOV AL,0ffh; JMP 35ee
        x.steps += 1;
        let n = if num == 0 { 0 } else { x.steps += 1; num - 1 };
        x.steps += 5;
        (n as u32) << 8 | 0xff
    };
    // 35ee: MOV [09dc],35ffh; IDIV BX
    x.ds_ww(DIVIDE_RESUME, 0x35ff);
    x.steps += 2;
    match idiv32(dividend, den) {
        Some((q, r)) => {
            // MOVSX EAX,AX; SHL EAX,8; RET
            x.set(EDX, r);
            x.c.r[EAX] = ((q as i16 as i32) << 8) as u32;
            x.steps += 3;
        }
        None => {
            // 35ff: MOV AX,CX; CWD; MOV DL,AH; MOV AH,AL; XOR AL,AL; SAR DX,1; RCR AX,1 (twice);
            // MOV [09dc],3621h; IDIV BX
            x.steps += DIVIDE_HANDLER_STEPS + 11;
            let shifted = (((num as i16 as i32) << 8) >> 2) as u32;
            x.ds_ww(DIVIDE_RESUME, 0x3621);
            match idiv32(shifted, den) {
                Some((q, r)) => {
                    // MOVSX EAX,AX; SHL EAX,0ah; RET
                    x.set(EDX, r);
                    x.c.r[EAX] = ((q as i16 as i32) << 10) as u32;
                    x.steps += 3;
                }
                None => {
                    // 3621: MOV AX,CX
                    x.steps += DIVIDE_HANDLER_STEPS + 1;
                    x.set(EDX, (shifted >> 16) as u16);
                    saturate(x, num);
                }
            }
        }
    }
}

/// `IDIV r16` of `DX:AX`: (quotient, remainder), `None` on a divide error.
fn idiv32(n: u32, d: u16) -> Option<(u16, u16)> {
    let (n, d) = (n as i32 as i64, d as i16 as i64);
    if d == 0 {
        return None;
    }
    let q = n / d;
    if q < i16::MIN as i64 || q > i16::MAX as i64 {
        return None;
    }
    Some((q as u16, (n % d) as u16))
}

/// The number of padding pixels an edge-padding entry fills (`CS:358b`/`35ad`: k = 1…15 fill
/// k pixels, 16 a loop), from its address, or `None` for none.
fn padding(x: &mut Ctx, table: u16, addr: u16) -> Option<u16> {
    (1..=16u16).find(|&k| rw(x, table + 2 * k) == addr)
}

/// The row mapper `EL2MCGA3:5448` (a near routine, with its `RET`): fills `CS:3578` rows from
/// row `CS:3532` down between the edges, each pixel the buffer texel at the interpolated
/// (u, v), the per-pixel steps from each row's ends by `IDIV`. Rows outside the view are
/// skipped. For bitmaps other than D0h–D8h, the left edge of a steep row is padded with up to
/// 16 pixels of the flat colour `DS:1fb1` to the left (and the right one to the right), so
/// no gap shows next to a neighbour.
pub fn map_rows(x: &mut Ctx) -> Result<(), Decline> {
    // ADD CS:[352e],8000h; SUB CS:[352a],8000h; MOV CS:[357f],552dh; MOV CS:[3581],5541h
    let v = rd(x, 0x352e).wrapping_add(0x8000);
    wd(x, 0x352e, v);
    let v = rd(x, 0x352a).wrapping_sub(0x8000);
    wd(x, 0x352a, v);
    ww(x, 0x357f, 0x552d);
    ww(x, 0x3581, 0x5541);
    x.steps += 4;
    // CMP CS:[357a],0d3h; JB 547c; CMP CS:[357a],0d8h; JBE 54e2; 547c: CMP CS:[357a],0d0h; JE 54e2
    let id = rw(x, TEX_BITMAP);
    x.steps += 2;
    let pad = if id < 0xd3 {
        x.steps += 2;
        id != 0xd0
    } else {
        x.steps += 2;
        if id <= 0xd8 {
            false
        } else {
            x.steps += 2;
            id != 0xd0
        }
    };
    if pad {
        // MOV EAX,CS:[3534]; OR EAX,EAX; JNS 54b5
        let s = rd(x, 0x3534);
        x.c.r[EAX] = s;
        x.steps += 3;
        if (s as i32) < 0 {
            // NEG EAX; ADD EAX,17fffh; SHR EAX,11h; MOV CS:[3587],AX; CMP AX,10h; JB; MOV AX,10h;
            // MOV AX,CS:[EAX*2+358bh]; MOV CS:[357f],AX
            let n = s.wrapping_neg().wrapping_add(0x17fff) >> 17;
            ww(x, 0x3587, n as u16);
            x.steps += 6;
            let k = if n < 0x10 { n } else { x.steps += 1; 0x10 };
            let e = rw(x, 0x358b + 2 * k as u16);
            ww(x, 0x357f, e);
            x.c.r[EAX] = (n & 0xffff_0000) | e as u32;
            x.steps += 2;
        }
        // 54b5: MOV EAX,CS:[3538]; OR EAX,EAX; JS 54e2
        let s = rd(x, 0x3538);
        x.c.r[EAX] = s;
        x.steps += 3;
        if (s as i32) >= 0 {
            // ADD EAX,17fffh; SHR EAX,11h; MOV CS:[3589],AX; CMP AX,10h; JB; MOV AX,10h;
            // MOV AX,CS:[EAX*2+35adh]; MOV CS:[3581],AX
            let n = s.wrapping_add(0x17fff) >> 17;
            ww(x, 0x3589, n as u16);
            x.steps += 5;
            let k = if n < 0x10 { n } else { x.steps += 1; 0x10 };
            let e = rw(x, 0x35ad + 2 * k as u16);
            ww(x, 0x3581, e);
            x.c.r[EAX] = (n & 0xffff_0000) | e as u32;
            x.steps += 2;
        }
    }
    // 54e2: MOV AL,[1fb1]; MOV AH,AL; MOV BX,AX; SHL EAX,10h; MOV AX,BX; MOV CS:[3583],EAX
    let c = x.ds_rb(0x1fb1);
    let fill = u32::from_ne_bytes([c; 4]);
    wd(x, 0x3583, fill);
    x.c.r[EAX] = fill;
    x.set(EBX, fill as u16);
    x.steps += 6;
    // CMP CS:[3578],0; JLE 587f
    x.steps += 2;
    if (rw(x, 0x3578) as i16) > 0 {
        loop {
            map_row(x)?;
            // 582f: the edges and texture coordinates step; INC CS:[3532]; DEC CS:[3578]; JNE
            for (v, s) in [(0x352a, 0x3534), (0x352e, 0x3538), (0x3540, 0x3550), (0x3544, 0x3554), (0x3548, 0x3558), (0x354c, 0x355c)] {
                let step = rd(x, s);
                x.c.r[EAX] = step;
                let n = rd(x, v).wrapping_add(step);
                wd(x, v, n);
            }
            let y = rw(x, 0x3532).wrapping_add(1);
            ww(x, 0x3532, y);
            let n = rw(x, 0x3578).wrapping_sub(1);
            ww(x, 0x3578, n);
            x.steps += 15;
            if n == 0 {
                break;
            }
        }
    }
    // 587f: SUB CS:[352e],8000h; ADD CS:[352a],8000h; RET
    let v = rd(x, 0x352e).wrapping_sub(0x8000);
    wd(x, 0x352e, v);
    let v = rd(x, 0x352a).wrapping_add(0x8000);
    wd(x, 0x352a, v);
    x.steps += 3;
    Ok(())
}

/// Fill `n` bytes of the padding colour at `ES:at`… (one of the padding entries: as many
/// stores as `n` has 1-, 2- and 4-byte pieces).
fn pad_bytes(x: &mut Ctx, at: u16, n: u16) {
    let c = x.c.l(EBX);
    let es = x.c.es;
    for i in 0..n {
        x.bus.write8(lin(es, at.wrapping_add(i)), c);
    }
    x.steps += ((n & 3).count_ones() + (n >> 2) as u32) as u64;
}

/// One row of [`map_rows`] (`54fe`–`582c`).
fn map_row(x: &mut Ctx) -> Result<(), Decline> {
    // 54fe: MOV AX,CS:[3532]; CMP AX,0a8h; JAE 582f
    let y = rw(x, 0x3532);
    x.set(EAX, y);
    x.steps += 3;
    if y >= VIEW_HEIGHT {
        return Ok(());
    }
    // XCHG AL,AH; MOV SI,AX; SHR AX,2; ADD SI,AX; MOV AX,CS:[352c]; ADD SI,AX;
    // MOV EBX,CS:[3583]; CMP AX,10h; JL 552d; CMP AX,140h; JG 552d; JMP CS:[357f]
    let row = y.swap_bytes();
    let left = rw(x, 0x352c);
    let mut si = row.wrapping_add(row >> 2).wrapping_add(left);
    x.set(EAX, left);
    x.c.r[EBX] = rd(x, 0x3583);
    x.steps += 9;
    if (left as i16) >= 0x10 {
        x.steps += 2;
        if (left as i16) <= WIDTH as i16 {
            x.steps += 1;
            let entry = rw(x, 0x357f);
            if entry == 0x5918 {
                // 5918: MOV CX,CS:[3587]; PUSH AX; PUSH SI; DEC SI; DEC AX; JS; MOV ES:[SI],BL;
                // DEC CX; JNZ; POP SI; POP AX; JMP 552d
                let mut cx = rw(x, 0x3587);
                x.push(left);
                x.push(si);
                let (mut p, mut a) = (si, left);
                let c = x.c.l(EBX);
                let es = x.c.es;
                x.steps += 3;
                loop {
                    p = p.wrapping_sub(1);
                    a = a.wrapping_sub(1);
                    x.steps += 3;
                    if (a as i16) < 0 {
                        break;
                    }
                    x.bus.write8(lin(es, p), c);
                    cx = cx.wrapping_sub(1);
                    x.steps += 3;
                    if cx == 0 {
                        break;
                    }
                }
                x.set(ECX, cx);
                let _ = x.pop();
                let _ = x.pop();
                x.steps += 3;
            } else if let Some(k) = padding(x, 0x358b, entry) {
                pad_bytes(x, si.wrapping_sub(k), k);
                x.steps += 1; // JMP 552d
            } else if entry != 0x552d {
                return Err(Decline("unknown left padding entry"));
            }
        }
    } else {
        x.steps += 0;
    }
    // 552d: MOV DX,CS:[3530]; CMP DX,12fh; JA 5543; SUB DX,AX; ADD SI,DX; JMP CS:[3581]; 5541:
    // SUB SI,DX
    let right = rw(x, 0x3530);
    x.set(EDX, right);
    x.steps += 3;
    if right <= 0x12f {
        let dx = right.wrapping_sub(left);
        x.set(EDX, dx);
        si = si.wrapping_add(dx);
        x.steps += 3;
        let entry = rw(x, 0x3581);
        if entry == 0x59b2 {
            // 59b2: PUSH DX; PUSH SI; ADD DX,AX; MOV CX,CS:[3589]; INC SI; INC DX; CMP DX,13fh;
            // JG; MOV ES:[SI],BL; DEC CX; JNE; POP SI; POP DX; JMP 5541
            x.push(dx);
            x.push(si);
            let mut d = dx.wrapping_add(left);
            let mut cx = rw(x, 0x3589);
            let (mut p, c, es) = (si, x.c.l(EBX), x.c.es);
            x.steps += 4;
            loop {
                p = p.wrapping_add(1);
                d = d.wrapping_add(1);
                x.steps += 4;
                if (d as i16) > 0x13f {
                    break;
                }
                x.bus.write8(lin(es, p), c);
                cx = cx.wrapping_sub(1);
                x.steps += 3;
                if cx == 0 {
                    break;
                }
            }
            x.set(ECX, cx);
            let _ = x.pop();
            let _ = x.pop();
            x.steps += 3;
        } else if let Some(k) = padding(x, 0x35ad, entry) {
            pad_bytes(x, si.wrapping_add(1), k);
            x.steps += 1; // JMP 5541
        } else if entry != 0x5541 {
            return Err(Decline("unknown right padding entry"));
        }
        si = si.wrapping_sub(dx);
        x.steps += 1;
    }
    x.set(ESI, si);
    // 5543: MOV BP,CS:[3530]; OR BP,BP; JS 582f; CMP AX,13fh; JG 582f; MOV DI,BP; SUB DI,AX;
    // JLE 582f; CMP BP,13fh; JLE; MOV BP,13fh; SUB BP,AX
    let mut bp = right;
    x.set(EBP, bp);
    x.steps += 3;
    if (bp as i16) < 0 {
        return Ok(());
    }
    x.steps += 2;
    if (left as i16) > 0x13f {
        return Ok(());
    }
    let width = bp.wrapping_sub(left);
    x.set(EDI, width);
    x.steps += 3;
    if (bp as i16 as i32) - (left as i16 as i32) <= 0 {
        return Ok(());
    }
    x.steps += 2;
    if (bp as i16) > 0x13f {
        bp = 0x13f;
        x.steps += 1;
    }
    bp = bp.wrapping_sub(left);
    x.set(EBP, bp);
    x.steps += 1;
    // The per-pixel steps of u and v: (end − start) >> 8, rounded towards 0 by ±1, / width.
    let mut steps = [0u16; 2];
    for (k, (hi, lo, resume, store)) in [(0x3548, 0x3540, 0x5590, 0x3560), (0x354c, 0x3544, 0x55bc, 0x3562)].into_iter().enumerate() {
        // MOV EAX,..; SUB EAX,..; SAR EAX,8; JE; INC EAX; JS; SUB EAX,2
        let mut d = ((rd(x, hi).wrapping_sub(rd(x, lo)) as i32) >> 8) as u32;
        x.steps += 4;
        if d != 0 {
            d = d.wrapping_add(1);
            x.steps += 2;
            if (d as i32) >= 0 {
                d = d.wrapping_sub(2);
                x.steps += 1;
            }
        }
        x.c.r[EAX] = d;
        // MOV EDX,EAX; SHR EDX,10h; MOV [09dc],r; IDIV DI; MOV CS:[r],AX
        x.c.r[EDX] = d >> 16;
        x.ds_ww(DIVIDE_RESUME, resume);
        x.steps += 4;
        let q = match idiv32(d, width) {
            Some((q, r)) => {
                x.set(EAX, q);
                x.set(EDX, r);
                q
            }
            None => {
                x.steps += DIVIDE_HANDLER_STEPS;
                d as u16
            }
        };
        ww(x, store, q);
        steps[k] = q;
        x.steps += 1;
    }
    // SHL EBP,10h; MOV CX,CS:[3541]; MOV DX,CS:[3545]; MOV DI,CS:[3560]; MOV BP,CS:[3562]
    let mut count = (bp as u32) << 16;
    let mut u = rw(x, 0x3541);
    let mut v = rw(x, 0x3545);
    let (du, dv) = (steps[0], steps[1]);
    count |= dv as u32;
    x.set(EDI, du);
    x.steps += 5;
    // CMP CS:[352c],0; JGE 5603; else start at x = 0
    x.steps += 2;
    if (left as i16) < 0 {
        // MOV AX,..; NEG AX; IMUL AX,DI; ADD CX,AX; MOV AX,..; NEG AX; IMUL AX,BP; ADD DX,AX;
        // MOV AX,CS:[352c]; SUB SI,AX; SHL EAX,10h; ADD EBP,EAX
        let skip = left.wrapping_neg();
        u = u.wrapping_add(skip.wrapping_mul(du));
        v = v.wrapping_add(skip.wrapping_mul(dv));
        si = si.wrapping_sub(left);
        x.c.r[EAX] = (left as u32) << 16;
        count = count.wrapping_add((left as u32) << 16);
        x.steps += 12;
    }
    // 5603: pairs of pixels, the count in the high word of EBP
    let (ss, es) = (x.c.ss, x.c.es);
    let sp = x.sp();
    // A texel offset past FFFFh wraps below the buffer, into the words the fill pushed (its
    // copy loop, the call of this routine, divide-overflow frames), not modelled here.
    let base = rw(x, 0x357d);
    let pushed = |a: u16| a < base && a >= base.wrapping_sub(16);
    let mut pair = 0;
    let mut bx;
    let mut ax = x.ax();
    loop {
        // MOV BL,CH; MOV BH,DH; ADD CX,DI; ADD BX,SP; MOV AL,SS:[BX+2]; ADD DX,BP;
        // SUB EBP,10000h; JS 582c
        bx = ((v & 0xff00) | (u >> 8)).wrapping_add(sp);
        u = u.wrapping_add(du);
        if pushed(bx.wrapping_add(2)) {
            return Err(Decline("texel below the buffer"));
        }
        let t0 = x.bus.read8(lin(ss, bx.wrapping_add(2)));
        ax = (ax & 0xff00) | t0 as u16;
        v = v.wrapping_add(dv);
        count = count.wrapping_sub(0x10000);
        x.steps += 8;
        if (count as i32) < 0 {
            // 582c: MOV ES:[SI],AL
            x.bus.write8(lin(es, si), t0);
            x.steps += 1;
            break;
        }
        // the same into AH; MOV ES:[SI],AX; ADD SI,2; SUB EBP,10000h; JS 582f
        bx = ((v & 0xff00) | (u >> 8)).wrapping_add(sp);
        u = u.wrapping_add(du);
        if pushed(bx.wrapping_add(2)) {
            return Err(Decline("texel below the buffer"));
        }
        let t1 = x.bus.read8(lin(ss, bx.wrapping_add(2)));
        ax = (t1 as u16) << 8 | t0 as u16;
        v = v.wrapping_add(dv);
        x.bus.write16(lin(es, si), ax);
        si = si.wrapping_add(2);
        count = count.wrapping_sub(0x10000);
        x.steps += 10;
        if (count as i32) < 0 {
            break;
        }
        pair += 1;
        if pair == 10 {
            // JMP 5603
            pair = 0;
            x.steps += 1;
        }
    }
    x.set(EAX, ax);
    x.set(EBX, bx);
    x.set(ECX, u);
    x.set(EDX, v);
    x.set(ESI, si);
    x.c.r[EBP] = count;
    Ok(())
}

/// The bitmap's texels into the buffer at `CS:357d` (`EL2MCGA3:3787`–`3cd1`, `47b1`–`4cfb`):
/// `rows`·`cols` texels from `ES:si`, `col_step` apart in a row and `row_step` between rows,
/// each through the colour table `DS:1fb1`, the rows 256 bytes apart. The loop is unrolled
/// by 64.
fn copy_texels(x: &mut Ctx, mut si: u16, rows: u16, cols: u16, col_step: u16, row_step: u16) {
    let (es, ss) = (x.c.es, x.c.ss);
    let mut di = rw(x, 0x357d);
    let mut n = rows;
    let mut last;
    loop {
        // MOV CX,DX; PUSH DI; PUSH SI
        let (row_di, row_si) = (di, si);
        let (mut d, mut s) = (di, si);
        x.steps += 3;
        let mut c = cols;
        let mut k = 0;
        loop {
            // MOV AL,ES:[SI]; ADD SI,BP; MOV AL,[EAX+1fb1h]; MOV SS:[DI+k],AL; DEC CX; JZ
            let t = x.bus.read8(lin(es, s));
            s = s.wrapping_add(col_step);
            last = x.ds_rb(0x1fb1u16.wrapping_add(t as u16));
            x.bus.write8(lin(ss, d.wrapping_add(k)), last);
            c = c.wrapping_sub(1);
            x.steps += 6;
            if c == 0 {
                break;
            }
            k += 1;
            if k == 64 {
                // ADD DI,40h; JMP
                d = d.wrapping_add(0x40);
                k = 0;
                x.steps += 2;
            }
        }
        // POP SI; POP DI; ADD DI,100h; ADD SI,BX; SUB ECX,10000h; JNS
        di = row_di.wrapping_add(0x100);
        si = row_si.wrapping_add(row_step);
        x.steps += 6;
        n = n.wrapping_sub(1);
        if n == 0 {
            break;
        }
    }
    x.c.r[EAX] = last as u32;
    x.c.r[ECX] = 0xffff_0000;
    x.set(ESI, si);
    x.set(EDI, di);
}

/// The halvings of the bitmap (`CS:357c`): the screen extent `ax` (the halved sum of the edges'
/// |dx| and |dy|) against the bitmap's (width + height)·¾, halved first (`372c`): count the
/// halvings while the extent stays below.
fn halvings(x: &mut Ctx, ax: u16, mut dx: u16) {
    let mut n: u8 = 0;
    loop {
        // SHR DX,1; CMP AX,DX; JAE; INC CS:[357c]; JMP
        dx >>= 1;
        x.steps += 3;
        if ax >= dx {
            break;
        }
        n = n.wrapping_add(1);
        x.steps += 2;
    }
    wb(x, 0x357c, n);
    x.set(EAX, ax);
    x.set(EDX, dx);
}

/// `MOV r,CS:[a]; SUB r,CS:[b]; JNS; NEG r`: |a − b|.
fn absdiff(x: &mut Ctx, a: u16, b: u16) -> u16 {
    let d = rw(x, a).wrapping_sub(rw(x, b));
    x.steps += 3;
    if (d as i16) < 0 {
        x.steps += 1;
        d.wrapping_neg()
    } else {
        d
    }
}

/// The halved buffer's sizes (`3746`–`3784`, `4775`–`47ae`): steps `BP`, `BX` shifted left and
/// width `DX`, height `AH` and the bitmap sizes shifted right by the halvings, each set to 1
/// when it becomes 0 (`SHR; JNE; INC`). With no halving the shifts leave the flags alone and
/// each `JNE` tests the zero flag of the instruction before (`zf`, then each `INC`'s): the
/// original then adds 1 to a size when that flag is set (*measured*: orientation 0 of the
/// quadrilateral copies one extra column). Returns (steps, row step, width, height).
fn halve(x: &mut Ctx, mut bp: u16, mut bx: u16, mut dx: u16, mut ah: u8, mut zf: bool) -> Result<(u16, u16, u16, u16), Decline> {
    // MOV DI,CS:[357d]; MOV CL,CS:[357c]; SHL BP,CL; SHL BX,CL; SHR DX,CL; JNE; INC DX;
    // SHR AH,CL; JNE; INC AH; SHR CS:[353c],CL; JNE; INC; SHR CS:[353e],CL; JNE; INC
    let cl = rb(x, 0x357c);
    x.c.set_l(ECX, cl);
    let n = (cl & 31) as u32;
    let shl = |v: u16| if n >= 16 { 0 } else { v << n };
    let shr = |v: u16| if n >= 16 { 0 } else { v >> n };
    bp = shl(bp);
    bx = shl(bx);
    dx = shr(dx);
    x.steps += 6;
    if n != 0 {
        zf = dx == 0;
    }
    if zf {
        dx = dx.wrapping_add(1);
        zf = dx == 0;
        x.steps += 1;
    }
    ah = if n >= 8 { 0 } else { ah >> n };
    x.steps += 2;
    if n != 0 {
        zf = ah == 0;
    }
    if zf {
        ah = ah.wrapping_add(1);
        zf = ah == 0;
        x.steps += 1;
    }
    for off in [0x353c, 0x353e] {
        let mut v = shr(rw(x, off));
        x.steps += 2;
        if n != 0 {
            zf = v == 0;
        }
        if zf {
            v = v.wrapping_add(1);
            zf = v == 0;
            x.steps += 1;
        }
        ww(x, off, v);
    }
    // MOVZX CX,AH; DEC CX; SHL ECX,10h; XOR EAX,EAX
    x.steps += 4;
    Ok((bp, bx, dx, ah as u16))
}

/// The vertices into `CS:351a`… and the vertex order bits (bit 0: y0 > y1, bit 1: y0 > y2,
/// bit 2: y1 > y2) (`3670`–`36a1`).
fn store_vertices(x: &mut Ctx, v: [u16; 6]) -> u16 {
    for (i, w) in v.into_iter().enumerate() {
        ww(x, 0x351a + 2 * i as u16, w);
    }
    // XOR DI,DI; CMP BX,DX; JLE; OR DI,1; CMP BX,BP; JLE; OR DI,2; CMP DX,BP; JLE; OR DI,4
    let mut order = 0;
    x.steps += 6 + 1;
    for (bit, (a, b)) in [(1, (v[1], v[3])), (2, (v[1], v[5])), (4, (v[3], v[5]))] {
        x.steps += 2;
        if (a as i16) > (b as i16) {
            order |= bit;
            x.steps += 1;
        }
    }
    x.set(EDI, order);
    order
}

/// The signed area test of three vertices (`3650`–`3669`): `SUB`s, two `IMUL`s and
/// `SUB AX,BP; SBB DX,CX`; true if the difference of the products is negative.
fn turns_back(a: (u16, u16), b: (u16, u16), c: (u16, u16)) -> bool {
    let p1 = (c.0.wrapping_sub(b.0) as i16 as i32).wrapping_mul(b.1.wrapping_sub(a.1) as i16 as i32);
    let p2 = (c.1.wrapping_sub(b.1) as i16 as i32).wrapping_mul(b.0.wrapping_sub(a.0) as i16 as i32);
    (p2.wrapping_sub(p1)) < 0
}

/// Port of `EL2MCGA3:3635` (slot `DS:0208`): bitmap `DI` mapped onto the triangle (`AX`,`BX`),
/// (`CX`,`DX`), (`SI`,`BP`), in the target segment. Vertices in clockwise order (else 1 and 2
/// are swapped); degenerate orders (2 and 5) draw nothing. Ends at the cache lookup if the
/// bitmap is not cached ([`textured_triangle_after_lookup`] goes on).
pub fn textured_triangle(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // AND DI,0fffh; MOV CS:[357a],DI; INC [1fa9]; SUB SP,4100h; MOV CS:[357d],SP
    let id = x.di() & 0xfff;
    ww(x, TEX_BITMAP, id);
    let n = x.ds_rw(BLIT_BUSY).wrapping_add(1);
    x.ds_ww(BLIT_BUSY, n);
    let sp = x.sp().wrapping_sub(0x4100);
    x.set(ESP, sp);
    ww(x, 0x357d, sp);
    x.steps += 5;
    // PUSH AX..BP; the area test; POP BP..AX; JNS 3670; XCHG SI,CX; XCHG BP,DX
    let mut v = [x.ax(), x.bx(), x.cx(), x.dx(), x.si(), x.bp()];
    let back = turns_back((v[0], v[1]), (v[2], v[3]), (v[4], v[5]));
    x.steps += 5 + 11 + 5 + 1;
    if back {
        v.swap(2, 4);
        v.swap(3, 5);
        x.steps += 2;
    }
    for (r, w) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
        x.set(r, w);
    }
    let order = store_vertices(x, v);
    // CMP DI,2; JE 44c8; CMP DI,5; JE 44c8
    x.steps += 2;
    if order == 2 {
        return end_textured(x);
    }
    x.steps += 2;
    if order == 5 {
        return end_textured(x);
    }
    // MOV CS:[3576],DI; MOV BX,CS:[357a]; PUSH CS; CALL 173
    ww(x, 0x3576, order);
    x.set(EBX, id);
    x.steps += 2;
    let cs = x.c.cs;
    x.far_call(cs, 0x0173, 0x36c0);
    x.steps += 1; // PUSH CS
    if cache_lookup(x) == Lookup::Miss {
        return Ok(());
    }
    textured_triangle_after_lookup(x)
}

/// `EL2MCGA3:44c8`: `ADD SP,4100h; DEC [1fa9]; RETF`.
fn end_textured(x: &mut Ctx) -> Result<(), Decline> {
    let sp = x.sp().wrapping_add(0x4100);
    x.set(ESP, sp);
    let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
    x.ds_ww(BLIT_BUSY, n);
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of the textured triangle after the cache lookup (`EL2MCGA3:36c0`): the halvings, the
/// texel copy, and the case for the vertex order (the jump table at `CS:3ce5`).
pub fn textured_triangle_after_lookup(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // MOV DX,ES:[SI+2]; MOV BL,ES:[SI+1]; XOR BH,BH; MOV CS:[353c],DX; MOV CS:[353e],BX;
    // MOV CS:[357c],0
    let (es, si) = (x.c.es, x.si());
    let w = x.bus.read16(lin(es, si.wrapping_add(2)));
    let h = x.bus.read8(lin(es, si.wrapping_add(1))) as u16;
    ww(x, 0x353c, w);
    ww(x, 0x353e, h);
    wb(x, 0x357c, 0);
    x.steps += 6;
    // The screen extent: (|x0−x1| + |x1−x2|) / 2 + (|y0−y1| + |y1−y2|) / 2
    let ax = absdiff(x, 0x351a, 0x351e);
    let bx = absdiff(x, 0x351e, 0x3522);
    let ax = ax.wrapping_add(bx);
    let dx = absdiff(x, 0x351c, 0x3520);
    let bx = absdiff(x, 0x3520, 0x3524);
    let dx = dx.wrapping_add(bx);
    let extent = (ax >> 1).wrapping_add(dx >> 1);
    x.steps += 1 + 1 + 3;
    // MOV DX,CS:[353c]; ADD DX,CS:[353e]; MOV BX,DX; SHR BX,2; SUB DX,BX
    let size = w.wrapping_add(h);
    let size = size.wrapping_sub(size >> 2);
    x.steps += 5;
    halvings(x, extent, size);
    // 3739: MOV AH,CS:[353e]; MOV DX,CS:[353c]; ADD SI,8; MOV BP,1; MOV BX,DX
    x.steps += 5;
    // The zero flag of ADD SI,8 reaches the JNEs when nothing is halved.
    let (bp, bx, cols, rows) = halve(x, 1, w, w, h as u8, si.wrapping_add(8) == 0)?;
    copy_texels(x, si.wrapping_add(8), rows, cols, bp, bx);
    x.set(EBX, bx);
    x.set(EDX, cols);
    x.set(EBP, bp);
    // 3cd5: MOV ES,[09e2]; MOV DI,CS:[3576]; SHL DI,1; JMP CS:[DI+3ce5]
    x.c.es = x.ds_rw(TARGET);
    let order = rw(x, 0x3576);
    x.set(EDI, order << 1);
    let case = rw(x, 0x3ce5u16.wrapping_add(order << 1));
    x.steps += 4;
    run(x, TRIANGLE_CASES, case)
}

/// Port of `EL2MCGA3:44d1` (slot `DS:0204`): bitmap `DI` & 0FFFh mapped onto the quadrilateral
/// (`AX`,`BX`), (`CX`,`DX`), (`SI`,`BP`), (`DS:57e9`,`DS:57ed`) (perimeter 0–1–2–3 here),
/// `DI` bits 12–15 its orientation (bit 14: mirrored). A quadrilateral that is not convex is
/// filled flat in colour `DS:1fb1` ([`quad_clipped`]). Ends at the cache lookup if the bitmap
/// is not cached ([`textured_quad_after_lookup`] goes on).
pub fn textured_quad(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // MOV CS:[357a],DI; SHR DI,0ch; MOV CS:[3574],DI; AND DI,4; MOV CS:[3576],DI;
    // AND CS:[357a],0fffh; INC [1fa9]; SUB SP,4100h; MOV CS:[357d],SP
    let di = x.di();
    ww(x, TEX_BITMAP, di & 0xfff);
    let turn = di >> 12;
    ww(x, 0x3574, turn);
    ww(x, 0x3576, turn & 4);
    x.set(EDI, turn & 4);
    let n = x.ds_rw(BLIT_BUSY).wrapping_add(1);
    x.ds_ww(BLIT_BUSY, n);
    let sp = x.sp().wrapping_sub(0x4100);
    x.set(ESP, sp);
    ww(x, 0x357d, sp);
    x.steps += 9;
    let mut v = [(x.ax(), x.bx()), (x.cx(), x.dx()), (x.si(), x.bp()), (x.ds_rw(QUAD_X3), x.ds_rw(QUAD_Y3))];
    // PUSH..; the area test of 0, 1, 2; POP..; JNS 452e; XCHG [57e9],CX; XCHG [57ed],DX;
    // XOR CS:[3576],4; NEG CS:[3574]
    x.steps += 5 + 11 + 5 + 1;
    if turns_back(v[0], v[1], v[2]) {
        v.swap(1, 3);
        let o = rw(x, 0x3576) ^ 4;
        ww(x, 0x3576, o);
        let t = rb(x, 0x3574).wrapping_neg();
        wb(x, 0x3574, t);
        x.steps += 4;
    }
    // 452e: the area test of 1, 0, 3 (PUSH..; SUB..; IMUL; ..; POP..); JNS 53fb
    x.steps += 4 + 4 + 6 + 4 + 1;
    let set_all = |x: &mut Ctx, v: &[(u16, u16); 4]| {
        for (r, w) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip([v[0].0, v[0].1, v[1].0, v[1].1, v[2].0, v[2].1]) {
            x.set(r, w);
        }
        x.ds_ww(QUAD_X3, v[3].0);
        x.ds_ww(QUAD_Y3, v[3].1);
    };
    set_all(x, &v);
    if !convex_second(v) {
        return run(x, QUAD_CASES, 0x53fb);
    }
    // 4551: rotate (0, 1, 2, 3) ← (1, 2, 3, 0) until vertex 0 is the top one, counting the
    // turns in CS:3574
    loop {
        let y0 = v[0].1 as i16;
        let mut top = true;
        for other in [v[1].1, v[2].1, v[3].1] {
            x.steps += 2;
            if y0 > other as i16 {
                top = false;
                break;
            }
        }
        if top {
            break;
        }
        // INC CS:[3574]; XCHG AX,CX; XCHG DX,BX; XCHG SI,CX; XCHG BP,DX; XCHG [57e9],SI;
        // XCHG [57ed],BP; JMP
        let t = rb(x, 0x3574).wrapping_add(1);
        wb(x, 0x3574, t);
        v = [v[1], v[2], v[3], v[0]];
        x.steps += 8;
    }
    set_all(x, &v);
    // 4575: AND CS:[3574],3; the vertices into CS:351a…3528; MOV BX,CS:[357a]; PUSH CS;
    // CALL 173
    let t = rb(x, 0x3574) & 3;
    wb(x, 0x3574, t);
    for (i, (vx, vy)) in v.into_iter().enumerate() {
        ww(x, 0x351a + 4 * i as u16, vx);
        ww(x, 0x351c + 4 * i as u16, vy);
    }
    x.set(EDI, v[3].1);
    let id = rw(x, TEX_BITMAP);
    x.set(EBX, id);
    x.steps += 1 + 6 + 4 + 1;
    let cs = x.c.cs;
    x.far_call(cs, 0x0173, 0x45b3);
    x.steps += 1; // PUSH CS
    if cache_lookup(x) == Lookup::Miss {
        return Ok(());
    }
    textured_quad_after_lookup(x)
}

/// The second area test of the quadrilateral (`452e`–`4547`, vertices 1, 0 and 3, after the
/// first may have swapped 1 and 3): true if convex (`JNS` taken past the flat fallback).
fn convex_second(v: [(u16, u16); 4]) -> bool {
    // SUB CX,AX; SUB DX,BX; SUB AX,[57e9]; SUB BX,[57ed]; IMUL DX; XCHG BX,AX; XCHG CX,DX;
    // IMUL DX; SUB AX,BX; SBB DX,CX
    let (a, b, d) = (v[0], v[1], v[3]);
    let p1 = (a.0.wrapping_sub(d.0) as i16 as i32).wrapping_mul(b.1.wrapping_sub(a.1) as i16 as i32);
    let p2 = (a.1.wrapping_sub(d.1) as i16 as i32).wrapping_mul(b.0.wrapping_sub(a.0) as i16 as i32);
    p2.wrapping_sub(p1) < 0
}

/// Port of the textured quadrilateral after the cache lookup (`EL2MCGA3:45b3`): the
/// halvings, the texel copy in the orientation (the jump table at `CS:469c`), and the cases.
pub fn textured_quad_after_lookup(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let (es, mut si) = (x.c.es, x.si());
    let w = x.bus.read16(lin(es, si.wrapping_add(2)));
    let h = x.bus.read8(lin(es, si.wrapping_add(1))) as u16;
    ww(x, 0x353c, w);
    ww(x, 0x353e, h);
    wb(x, 0x357c, 0);
    x.steps += 6;
    // The screen extent: the four edges' |dx| and |dy|, each sum quartered
    let mut ax = absdiff(x, 0x351a, 0x351e);
    for (a, b) in [(0x351e, 0x3522), (0x3522, 0x3526), (0x3526, 0x351a)] {
        let d = absdiff(x, a, b);
        ax = ax.wrapping_add(d);
        x.steps += 1;
    }
    let mut dx = absdiff(x, 0x351c, 0x3520);
    for (a, b) in [(0x3520, 0x3524), (0x3524, 0x3528), (0x3528, 0x351c)] {
        let d = absdiff(x, a, b);
        dx = dx.wrapping_add(d);
        x.steps += 1;
    }
    let extent = (ax >> 2).wrapping_add(dx >> 2);
    x.steps += 3;
    // MOV DX,..; ADD DX,..; MOV BX,DX; SHR BX,2; SUB DX,BX; CMP CS:[357a],0dfh; JNE; SHR DX,1
    let size = w.wrapping_add(h);
    let mut size = size.wrapping_sub(size >> 2);
    x.steps += 7;
    if rw(x, TEX_BITMAP) == 0xdf {
        size >>= 1;
        x.steps += 1;
    }
    halvings(x, extent, size);
    // 4679: MOV AH,CS:[353e]; MOV DX,CS:[353c]; ADD SI,8; XOR EBX,EBX; MOV BL,CS:[3574];
    // OR BL,CS:[3576]; JMP CS:[EBX*2+469c]
    si = si.wrapping_add(8);
    let mut ah = h as u8;
    let mut dx = w;
    let turn = (rb(x, 0x3574) | rb(x, 0x3576)) as u16;
    x.steps += 7;
    // The orientations: the texel steps along a row (BP) and between rows (BX), and the start.
    let (bp, bx, zf);
    let swap_sizes = |x: &mut Ctx, ah: &mut u8, dx: &mut u16| {
        // MOV DI,CS:[353c]; XCHG CS:[353e],DI; MOV CS:[353c],DI; SHR AX,8; XCHG DX,AX; MOV AH,AL
        let a = rw(x, 0x353c);
        let b = rw(x, 0x353e);
        ww(x, 0x353e, a);
        ww(x, 0x353c, b);
        let old_dx = *dx;
        *dx = *ah as u16;
        *ah = old_dx as u8;
        x.steps += 6;
    };
    let last_row = |x: &mut Ctx, si: &mut u16, ah: u8, dx: u16| {
        // PUSH AX; MOV AL,DL; DEC AH; MUL AH; ADD SI,AX; POP AX
        *si = si.wrapping_add((dx as u8) as u16 * ah.wrapping_sub(1) as u16);
        x.steps += 6;
    };
    match turn {
        0 => {
            // 46ac: MOV BP,1; MOV BX,DX; JMP 4775
            bp = 1;
            bx = dx;
            zf = turn == 0; // OR BL,CS:[3576]
            x.steps += 3;
        }
        4 => {
            // 46b4: MOV BP,DX; MOV BX,1; JMP 4775
            bp = dx;
            bx = 1;
            zf = false; // OR BL,CS:[3576]
            x.steps += 3;
        }
        1 => {
            // 46bc: MOV BP,DX; MOV BX,-1; ADD SI,DX; DEC SI; swap the sizes; JMP 4775
            bp = dx;
            bx = 0xffff;
            si = si.wrapping_add(dx).wrapping_sub(1);
            x.steps += 4;
            swap_sizes(x, &mut ah, &mut dx);
            zf = h == 0; // SHR AX,8
            x.steps += 1;
        }
        7 => {
            // 46dc: MOV BP,-1; MOV BX,DX; ADD SI,DX; DEC SI; swap the sizes; JMP 4775
            bp = 0xffff;
            bx = dx;
            si = si.wrapping_add(dx).wrapping_sub(1);
            x.steps += 4;
            swap_sizes(x, &mut ah, &mut dx);
            zf = h == 0;
            x.steps += 1;
        }
        2 => {
            // 46fb: MOV BP,-1; MOV BX,DX; NEG BX; ADD SI,DX; DEC SI; the last row; JMP 4775
            bp = 0xffff;
            bx = dx.wrapping_neg();
            si = si.wrapping_add(dx).wrapping_sub(1);
            x.steps += 5;
            last_row(x, &mut si, ah, dx);
            zf = si == 0; // ADD SI,AX
            x.steps += 1;
        }
        6 => {
            // 4711: MOV BX,-1; MOV BP,DX; NEG BP; ADD SI,DX; DEC SI; the last row; JMP 4775
            bx = 0xffff;
            bp = dx.wrapping_neg();
            si = si.wrapping_add(dx).wrapping_sub(1);
            x.steps += 5;
            last_row(x, &mut si, ah, dx);
            zf = si == 0;
            x.steps += 1;
        }
        3 => {
            // 4727: MOV BP,DX; NEG BP; MOV BX,1; the last row; swap the sizes; JMP 4775
            bp = dx.wrapping_neg();
            bx = 1;
            x.steps += 3;
            last_row(x, &mut si, ah, dx);
            swap_sizes(x, &mut ah, &mut dx);
            zf = h == 0;
            x.steps += 1;
        }
        5 => {
            // 474f: MOV BX,DX; NEG BX; MOV BP,1; the last row; swap the sizes
            bx = dx.wrapping_neg();
            bp = 1;
            x.steps += 3;
            last_row(x, &mut si, ah, dx);
            swap_sizes(x, &mut ah, &mut dx);
            zf = h == 0;
        }
        _ => return Err(Decline("orientation outside the table")),
    }
    let (bp, bx, cols, rows) = halve(x, bp, bx, dx, ah, zf)?;
    copy_texels(x, si, rows, cols, bp, bx);
    x.set(EBX, bx);
    x.set(EDX, cols);
    x.set(EBP, bp);
    // 4cff: MOV ES,[09e2]; then the cases from 4d03
    x.c.es = x.ds_rw(TARGET);
    x.steps += 1;
    run(x, QUAD_CASES, 0x4d03)
}
