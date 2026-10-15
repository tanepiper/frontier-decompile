//! Flat-shaded rasterisation: the span filler and the fills built on it (docs/VIDEO.md §3).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.

use super::*;

/// The span filler's three entries, which the fills pick by clipping need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanEntry {
    /// `EL2MCGA3:1a99`: no clipping.
    Unclipped,
    /// `EL2MCGA3:1a8c`: clipped to 0 ≤ x < 320.
    ClipX,
    /// `EL2MCGA3:1a86`: rows y ≥ 168 skipped, then as [`SpanEntry::ClipX`].
    ClipXY,
}

impl SpanEntry {
    /// The near address the fills keep in `DS:57cf` for this entry.
    pub fn address(self) -> u16 {
        match self {
            SpanEntry::Unclipped => 0x1a99,
            SpanEntry::ClipX => 0x1a8c,
            SpanEntry::ClipXY => 0x1a86,
        }
    }

    pub fn from_address(a: u16) -> Option<Self> {
        match a {
            0x1a99 => Some(SpanEntry::Unclipped),
            0x1a8c => Some(SpanEntry::ClipX),
            0x1a86 => Some(SpanEntry::ClipXY),
            _ => None,
        }
    }
}

/// Port of the span filler `EL2MCGA3:1a86` / `1a8c` / `1a99` (a near routine): fills row `DI`
/// from x = `BX` for `CX` pixels with colour `AH` in segment `ES`, rows 320 bytes apart. It
/// stores aligned dwords (`REP STOSD`) between a byte/word head and a byte tail. A length of 0
/// still fills one pixel when 0 ≤ x < 320 (`1a6d`); a negative length fills nothing.
///
/// Leaves `EAX` = the colour in all four bytes, `DX` = the colour word, `EBX` = x & 3,
/// `ECX` = 0 (FFFFh after a one-pixel span at x ≡ 2 mod 4), `DI` past the span and
/// `EDI` = 5·`EDI` before the row shift, as the original's `LEA` leaves it. Includes the `RET`.
pub fn span(x: &mut Ctx, entry: SpanEntry) {
    let mut at = entry;
    if at == SpanEntry::ClipXY {
        // 1a86: CMP DI,0a8h; JNC 1a85
        x.steps += 2;
        if x.di() >= VIEW_HEIGHT {
            x.steps += 1; // RET
            return;
        }
        at = SpanEntry::ClipX;
    }
    if at == SpanEntry::ClipX {
        // 1a8c: OR BX,BX; JS 1a75
        x.steps += 2;
        if (x.bx() as i16) < 0 {
            // 1a75: ADD CX,BX; JS 1a85; JZ 1a85; XOR BX,BX; JMP 1a90
            let cx = x.cx().wrapping_add(x.bx());
            x.set(ECX, cx);
            x.steps += 2;
            if (cx as i16) < 0 {
                x.steps += 1;
                return;
            }
            x.steps += 1;
            if cx == 0 {
                x.steps += 1;
                return;
            }
            x.set(EBX, 0);
            x.steps += 2;
        }
        // 1a90: MOV DX,140h; SUB DX,BX; SUB DX,CX; JS 1a7f
        let dx = WIDTH.wrapping_sub(x.bx()).wrapping_sub(x.cx());
        x.set(EDX, dx);
        x.steps += 4;
        if (dx as i16) < 0 {
            // 1a7f: ADD CX,DX; JS 1a85; JNZ 1a99; RET
            let cx = x.cx().wrapping_add(dx);
            x.set(ECX, cx);
            x.steps += 2;
            if (cx as i16) < 0 {
                x.steps += 1;
                return;
            }
            x.steps += 1;
            if cx == 0 {
                x.steps += 1;
                return;
            }
        }
    }
    // 1a99: OR CX,CX; JLE 1a6b
    x.steps += 2;
    if (x.cx() as i16) <= 0 {
        // 1a6b: JS 1a85; INC CX; CMP BX,140h; JC 1a9d; RET
        x.steps += 1;
        if (x.cx() as i16) < 0 {
            x.steps += 1;
            return;
        }
        x.set(ECX, 1);
        x.steps += 3;
        if x.bx() >= WIDTH {
            x.steps += 1;
            return;
        }
    }
    // 1a9d: LEA EDI,[EDI+EDI*4]; SHL DI,6; ADD DI,BX; MOV AL,AH; MOV DX,AX; SHL EAX,10h;
    // MOV AX,DX; AND ECX,0ffffh; AND EBX,3; JMP [EBX*2+1ac6]
    let edi = x.c.r[EDI].wrapping_mul(5);
    x.c.r[EDI] = edi;
    let di = (edi as u16).wrapping_shl(6).wrapping_add(x.bx());
    x.set(EDI, di);
    let colour = x.c.h(EAX);
    let word = (colour as u16) << 8 | colour as u16;
    x.set(EDX, word);
    x.c.r[EAX] = (word as u32) << 16 | word as u32;
    x.c.r[ECX] &= 0xffff;
    x.c.r[EBX] &= 3;
    x.steps += 10;
    let mut n = x.cx();
    let es = x.c.es;
    let mut di = di;
    // Head: x ≡ 1 → byte + word, x ≡ 2 → word, x ≡ 3 → byte, unless the span is too short.
    match x.c.r[EBX] {
        0 => {}
        1 => {
            // 1adc: SUB CX,3; JS 1af1
            x.steps += 2;
            if n < 3 {
                // 1af1: ADD CX,3; REP STOSB; RET
                di = fill(x, es, di, n, colour);
                x.set(EDI, di);
                x.set(ECX, 0);
                x.steps += 3;
                return;
            }
            n -= 3;
            di = fill(x, es, di, 3, colour); // STOSB; STOSW
            x.steps += 2;
        }
        2 => {
            // 1af8: SUB CX,2; JS 1b0c
            x.steps += 2;
            if n < 2 {
                // 1b0c: STOSB; RET
                di = fill(x, es, di, 1, colour);
                x.set(EDI, di);
                x.set(ECX, n.wrapping_sub(2));
                x.steps += 2;
                return;
            }
            n -= 2;
            di = fill(x, es, di, 2, colour); // STOSW
            x.steps += 1;
        }
        _ => {
            // 1b0e: DEC CX; STOSB
            n -= 1;
            di = fill(x, es, di, 1, colour);
            x.steps += 2;
        }
    }
    // ROR ECX,2; REP STOSD; ROL ECX,2; REP STOSB; RET
    di = fill(x, es, di, n, colour);
    x.set(EDI, di);
    x.c.r[ECX] = 0;
    x.steps += 5;
}

/// Store `n` bytes of `colour` at `seg:di` upwards (the offset wraps at 64 KB); returns the
/// offset after them.
pub fn fill(x: &mut Ctx, seg: u16, mut di: u16, n: u16, colour: u8) -> u16 {
    for _ in 0..n {
        x.bus.write8(lin(seg, di), colour);
        di = di.wrapping_add(1);
    }
    di
}

/// Divide-overflow resume offsets of one copy of the edge-slope block: the second `IDIV` and
/// the code after it (e.g. `1ba3` and `1bb3` for the triangle's first edge).
#[derive(Clone, Copy)]
pub struct EdgeBlock {
    pub resume: [u16; 2],
}

/// One copy of the edge-slope block (e.g. `EL2MCGA3:1b92`–`1be1`), entered after `SUB AX,..;
/// SUB BX,..` with dx in `AX` and a non-zero dy in `BX`. The slope is dx·256/dy as a signed
/// quotient in 16.16 fixed point (dx·256 is divided, the quotient shifted left by 8). If the
/// quotient overflows, the main program's `int 0` handler resumes at `resume[0]`, where dx·64
/// is divided and the quotient taken ×4; if that overflows too, `resume[1]` takes the shifted
/// dividend's low word, sign-extended, ×4. The edge's start, x0 + ½ − slope/2, is stored at
/// `start` (a 16.16 dword). Returns the slope (`DX:AX`).
pub fn edge_slope(x: &mut Ctx, dx: u16, dy: u16, x0: u16, block: EdgeBlock, start: u16) -> u32 {
    // CWD; MOV DL,AH; MOV AH,AL; XOR AL,AL; MOV [09dc],r0; IDIV BX
    let dividend = ((dx as i16 as i32) << 8) as u32;
    x.ds_ww(DIVIDE_RESUME, block.resume[0]);
    x.steps += 6;
    let w = match idiv(dividend, dy) {
        Some(q) => {
            // JMP; CWD
            x.steps += 2;
            q as i16 as i32 as u32
        }
        None => {
            // SAR DX,1; RCR AX,1 (twice); MOV [09dc],r1; IDIV BX
            x.steps += DIVIDE_HANDLER_STEPS + 6;
            let shifted = ((dividend as i32) >> 2) as u32;
            x.ds_ww(DIVIDE_RESUME, block.resume[1]);
            let low = match idiv(shifted, dy) {
                Some(q) => q,
                None => {
                    x.steps += DIVIDE_HANDLER_STEPS;
                    shifted as u16
                }
            };
            // CWD; SHL AX,1; RCL DX,1 (twice); JMP
            x.steps += 6;
            ((low as i16 as i32) << 2) as u32
        }
    };
    // MOV DL,AH; MOV AH,AL; XOR AL,AL: the bytes 3, 1, 0 of DX:AX, shifted by 8
    let v = (w & 0xff00_0000) | (w & 0xffff) << 8;
    // SAR DX,1; RCR AX,1; start = (x0:8000h) − half; SHL AX,1; RCL DX,1
    let half = ((v as i32) >> 1) as u32;
    x.ds_wd(start, ((x0 as u32) << 16 | 0x8000).wrapping_sub(half));
    x.steps += 11;
    half << 1
}

/// `IDIV BX` on `DX:AX`: the quotient, or `None` on a divide error (it does not fit 16 bits).
fn idiv(n: u32, d: u16) -> Option<u16> {
    let d = d as i16 as i64;
    if d == 0 {
        return None;
    }
    let q = n as i32 as i64 / d;
    if q < i16::MIN as i64 || q > i16::MAX as i64 {
        None
    } else {
        Some(q as u16)
    }
}

/// The slope a fill stores for a horizontal edge (`CWD; XCHG AX,DX; JMP`, e.g. `1bbe`): `DX`
/// = dx and `AX` = its sign, so `DX:AX` = dx·65536 + sign. The edge's start is not written.
fn flat_slope(x: &mut Ctx, dx: u16) -> u32 {
    x.steps += 3;
    let sign = if (dx as i16) < 0 { 0xffff } else { 0 };
    (dx as u32) << 16 | sign
}

/// Swap the two stored slopes (`MOV SI,[57bf]; XCHG [57c3],SI; MOV [57bf],SI; MOV SI,[57c1];
/// XCHG [57c5],SI; MOV [57c1],SI`). Returns `SI` (the old high word of [`SLOPE_B`]).
fn swap_slopes(x: &mut Ctx) -> u16 {
    let a = x.ds_rd(SLOPE_A);
    let b = x.ds_rd(SLOPE_B);
    x.ds_wd(SLOPE_A, b);
    x.ds_wd(SLOPE_B, a);
    x.steps += 6;
    (b >> 16) as u16
}

/// The two edge accumulators of a fill: left in `BX:AX`, right in `CX:DX` (16.16).
fn left(x: &Ctx) -> u32 {
    (x.bx() as u32) << 16 | x.ax() as u32
}
fn right(x: &Ctx) -> u32 {
    (x.cx() as u32) << 16 | x.dx() as u32
}
fn set_left(x: &mut Ctx, v: u32) {
    x.set(EBX, (v >> 16) as u16);
    x.set(EAX, v as u16);
}
fn set_right(x: &mut Ctx, v: u32) {
    x.set(ECX, (v >> 16) as u16);
    x.set(EDX, v as u16);
}

/// `XCHG AX,DX; XCHG CX,BX`: swap the two accumulators.
fn swap_edges(x: &mut Ctx) {
    let (l, r) = (left(x), right(x));
    set_left(x, r);
    set_right(x, l);
    x.steps += 2;
}

/// The scanline loop of a fill (e.g. `1cbb`–`1ceb`): while `[57bd]` > 0, step both edges by
/// their slopes, then fill from the left edge's integer part to the right edge's (`CX` − `BX`
/// pixels) on row `DI` through the span routine in `DS:57cf`, and go down a row.
fn rows(x: &mut Ctx, entry: SpanEntry) {
    // CMP [57bd],0; JLE
    x.steps += 2;
    if (x.ds_rw(ROWS) as i16) <= 0 {
        return;
    }
    loop {
        // ADD AX,[57bf]; ADC BX,[57c1]; ADD DX,[57c3]; ADC CX,[57c5]; PUSH AX..DI; SUB CX,BX;
        // MOV AH,[57fd]; CALL [57cf]
        let l = left(x).wrapping_add(x.ds_rd(SLOPE_A));
        let r = right(x).wrapping_add(x.ds_rd(SLOPE_B));
        set_left(x, l);
        set_right(x, r);
        let saved = [x.ax(), x.bx(), x.cx(), x.dx(), x.di()];
        x.set(ECX, x.cx().wrapping_sub(x.bx()));
        let colour = x.ds_rb(COLOUR);
        x.c.set_h(EAX, colour);
        x.steps += 12;
        span(x, entry);
        // POP DI..AX; INC DI; DEC [57bd]; JNZ
        for (r, v) in [EAX, EBX, ECX, EDX, EDI].into_iter().zip(saved) {
            x.set(r, v);
        }
        x.set(EDI, x.di().wrapping_add(1));
        let n = x.ds_rw(ROWS).wrapping_sub(1);
        x.ds_ww(ROWS, n);
        x.steps += 8;
        if n == 0 {
            break;
        }
    }
}

/// `CMP [891d],0; JNZ`, and if the edges were swapped into left/right order, swap them back
/// (accumulators and slopes).
fn unswap(x: &mut Ctx) {
    x.steps += 2;
    if x.ds_rb(EDGES_SWAPPED) == 0 {
        swap_edges(x);
        let si = swap_slopes(x);
        x.set(ESI, si);
    }
}

/// Port of `EL2MCGA3:1b62` (slot `DS:0264`): flat-shaded triangle (`AX`,`BX`), (`CX`,`DX`),
/// (`SI`,`BP`) in colour `DS:57fd` into the target segment `DS:09e2`, all vertices inside the 320×168 view.
pub fn triangle(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // 1b62: MOV [57cf],1a99h
    x.ds_ww(SPAN_ROUTINE, SpanEntry::Unclipped.address());
    x.steps += 1;
    fill_triangle(x, SpanEntry::Unclipped);
    Ok(())
}

/// Port of `EL2MCGA3:1b3f` (slot `DS:0260`): the same triangle clipped to the view. A triangle
/// with every vertex outside the view is not drawn at all, even if it covers it.
pub fn triangle_clipped(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let inside = |px: u16, py: u16| px < WIDTH && py < VIEW_HEIGHT;
    let v = [(x.ax(), x.bx()), (x.cx(), x.dx()), (x.si(), x.bp())];
    // 1b3f: CMP AX,140h; JNC 1b26; CMP BX,0a8h; JNC 1b26
    x.steps += 2;
    let first = v[0].0 < WIDTH && {
        x.steps += 2;
        v[0].1 < VIEW_HEIGHT
    };
    let clipped = if first {
        // 1b4a–1b60: CMP/JNC for the other four coordinates, to 1b1e on the first outside
        let mut out = false;
        for c in [v[1].0 < WIDTH, v[1].1 < VIEW_HEIGHT, v[2].0 < WIDTH, v[2].1 < VIEW_HEIGHT] {
            x.steps += 2;
            if !c {
                out = true;
                break;
            }
        }
        out
    } else {
        // 1b26: CMP CX,140h; JNC 1b32; CMP DX,0a8h; JC 1b1e; 1b32: CMP SI,140h; JNC 1b3e;
        // CMP BP,0a8h; JC 1b1e; 1b3e: RETF
        let mut any = false;
        for (px, py) in [v[1], v[2]] {
            x.steps += 2;
            if px < WIDTH {
                x.steps += 2;
                if inside(px, py) {
                    any = true;
                    break;
                }
            }
        }
        if !any {
            x.retf();
            return Ok(());
        }
        true
    };
    if clipped {
        // 1b1e: MOV [57cf],1a86h; JMP 1b68
        x.ds_ww(SPAN_ROUTINE, SpanEntry::ClipXY.address());
        x.steps += 2;
        fill_triangle(x, SpanEntry::ClipXY);
    } else {
        // 1b62
        x.ds_ww(SPAN_ROUTINE, SpanEntry::Unclipped.address());
        x.steps += 1;
        fill_triangle(x, SpanEntry::Unclipped);
    }
    Ok(())
}

/// The triangle fill from `EL2MCGA3:1b68`, with the span routine already chosen: sorts the
/// vertices by y (signed), fills from the top vertex down to the middle one between edges
/// 0–1 and 0–2, then to the bottom one between 1–2 and 0–2. Each row is sampled half a row
/// down, the left edge taken ½ pixel left and the right ½ pixel right. Ends with the `RETF`.
fn fill_triangle(x: &mut Ctx, entry: SpanEntry) {
    // 1b68: MOV ES,[09e2]
    x.c.es = x.ds_rw(TARGET);
    x.steps += 1;
    let (mut ax, mut bx, mut cx, mut dx, mut si, mut bp) = (x.ax(), x.bx(), x.cx(), x.dx(), x.si(), x.bp());
    // 1b6c: sort by y with three compare-and-exchange steps (CMP; JLE; XCHG; XCHG)
    x.steps += 2;
    if (bx as i16) > (dx as i16) {
        std::mem::swap(&mut ax, &mut cx);
        std::mem::swap(&mut dx, &mut bx);
        x.steps += 2;
    }
    x.steps += 2;
    if (bx as i16) > (bp as i16) {
        std::mem::swap(&mut ax, &mut si);
        std::mem::swap(&mut bp, &mut bx);
        x.steps += 2;
    }
    x.steps += 2;
    if (dx as i16) > (bp as i16) {
        std::mem::swap(&mut si, &mut cx);
        std::mem::swap(&mut bp, &mut dx);
        x.steps += 2;
    }
    let ((x0, y0), (x1, y1), (x2, y2)) = ((ax, bx), (cx, dx), (si, bp));
    // 1b82: PUSH CX; PUSH DX; PUSH SI; PUSH BP; XCHG AX,CX; XCHG DX,BX; MOV DI,DX; PUSH DI;
    // SUB AX,CX; SUB BX,DI; JZ 1bbe
    x.steps += 11;
    let dy = y1.wrapping_sub(y0);
    let slope = if dy == 0 {
        flat_slope(x, x1.wrapping_sub(x0))
    } else {
        edge_slope(x, x1.wrapping_sub(x0), dy, x0, EdgeBlock { resume: [0x1ba3, 0x1bb3] }, START_A)
    };
    // 1be3: MOV [57bf],AX; MOV [57c1],DX; MOV [57bd],BX
    x.ds_wd(SLOPE_A, slope);
    x.ds_ww(ROWS, dy);
    // 1bee: POP DI; MOV AX,SI; MOV BX,BP; SUB AX,CX; SUB BX,DI; JZ 1c25
    x.steps += 3 + 6;
    let dy = y2.wrapping_sub(y0);
    let slope = if dy == 0 {
        flat_slope(x, x2.wrapping_sub(x0))
    } else {
        edge_slope(x, x2.wrapping_sub(x0), dy, x0, EdgeBlock { resume: [0x1c0a, 0x1c1a] }, START_B)
    };
    // 1c4a: MOV [57c3],AX; MOV [57c5],DX; 1c51: MOVZX EDI,DI; MOV BX,[57c9]; MOV AX,[57c7];
    // MOV CX,[57cd]; MOV DX,[57cb]
    x.ds_wd(SLOPE_B, slope);
    x.c.r[EDI] = y0 as u32;
    let (a, b) = (x.ds_rd(START_A), x.ds_rd(START_B));
    set_left(x, a);
    set_right(x, b);
    x.steps += 2 + 5;
    // MOV SI,[57c5]; CMP SI,[57c1]; JG 1ca9; JL 1c7a; MOV SI,[57c3]; CMP SI,[57bf]; JA 1ca9
    let (sa, sb) = (x.ds_rd(SLOPE_A), x.ds_rd(SLOPE_B));
    let (ha, hb) = ((sa >> 16) as i16, (sb >> 16) as i16);
    x.set(ESI, hb as u16);
    x.steps += 3;
    let b_steeper = if hb > ha {
        true
    } else {
        x.steps += 1;
        if hb < ha {
            false
        } else {
            x.set(ESI, sb as u16);
            x.steps += 3;
            sb as u16 > sa as u16
        }
    };
    if b_steeper {
        // 1ca9: MOV [891d],0ffh; SUB AX,8000h; SBB BX,0; ADD DX,8000h; ADC CX,0
        x.ds_wb(EDGES_SWAPPED, 0xff);
        set_left(x, left(x).wrapping_sub(0x8000));
        set_right(x, right(x).wrapping_add(0x8000));
        x.steps += 5;
    } else {
        // 1c7a: MOV [891d],0; ADD AX,8000h; ADC BX,0; SUB DX,8000h; SBB CX,0; swap; JMP 1cbb
        x.ds_wb(EDGES_SWAPPED, 0);
        set_left(x, left(x).wrapping_add(0x8000));
        set_right(x, right(x).wrapping_sub(0x8000));
        x.steps += 5;
        swap_edges(x);
        let si = swap_slopes(x);
        x.set(ESI, si);
        x.steps += 1;
    }
    rows(x, entry);
    unswap(x);
    // 1d0f: POP BX; POP AX; POP BP; POP SI; PUSH DX; SUB AX,SI; SUB BX,BP; JZ 1d46
    x.set(EBP, y1);
    x.set(ESI, x1);
    let edge_b = x.dx();
    x.steps += 8;
    let dy = y2.wrapping_sub(y1);
    let slope = if dy == 0 {
        flat_slope(x, x2.wrapping_sub(x1))
    } else {
        edge_slope(x, x2.wrapping_sub(x1), dy, x1, EdgeBlock { resume: [0x1d2b, 0x1d3b] }, START_A)
    };
    // 1d6b: MOV [57bf],AX; MOV [57c1],DX; MOV [57bd],BX; POP DX; MOV AX,[57c7]; MOV BX,[57c9]
    x.ds_wd(SLOPE_A, slope);
    x.ds_ww(ROWS, dy);
    x.set(EDX, edge_b);
    let a = x.ds_rd(START_A);
    set_left(x, a);
    x.steps += 6;
    // CMP CX,BX; JG 1db0; JL 1d88; CMP DX,AX; JA 1db0
    let (r, l) = (right(x), left(x));
    let (hr, hl) = ((r >> 16) as i16, (l >> 16) as i16);
    x.steps += 2;
    let right_of = if hr > hl {
        true
    } else {
        x.steps += 1;
        if hr < hl {
            false
        } else {
            x.steps += 2;
            r as u16 > l as u16
        }
    };
    if right_of {
        // 1db0: MOV [891d],0ffh; SUB AX,8000h; SBB BX,0
        x.ds_wb(EDGES_SWAPPED, 0xff);
        set_left(x, left(x).wrapping_sub(0x8000));
        x.steps += 3;
    } else {
        // 1d88: MOV [891d],0; ADD AX,8000h; ADC BX,0; swap; JMP 1dbb
        x.ds_wb(EDGES_SWAPPED, 0);
        set_left(x, left(x).wrapping_add(0x8000));
        x.steps += 3;
        swap_edges(x);
        let si = swap_slopes(x);
        x.set(ESI, si);
        x.steps += 1;
    }
    rows(x, entry);
    unswap(x);
    x.retf();
}
