//! Text, rectangles, pixels, the draw target and screen clears (docs/VIDEO.md §4).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.

use super::*;

/// Paragraph offset of the font data in the driver's image (`EL2MCGA3` segment `05de`): glyph
/// bitmaps and widths ([`glyph`], [`text_width`], [`char_width`]).
pub const FONT_PARAS: u16 = 0x5de;

fn font(x: &Ctx) -> u16 {
    x.c.cs.wrapping_add(FONT_PARAS)
}

/// Port of `EL2MCGA3:1a1b` (slot `DS:0218`): character `AL` at (`CX`, `DX`) in colour `BL`, into
/// the target segment. The glyph is 9 rows of bits at font offset 184h + 10·`AL` (most
/// significant bit leftmost; a row ends at its last set bit), unclipped; returns its advance
/// (the byte after the rows) in `AL`, or 0 without drawing when y ≥ 191.
pub fn glyph(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // CMP DX,0bfh; JAE 1a68 (XOR AL,AL; RETF)
    x.steps += 2;
    let (col, row) = (x.cx(), x.dx());
    if row >= 0xbf {
        x.c.set_l(EAX, 0);
        x.steps += 1;
        x.retf();
        return Ok(());
    }
    // PUSH CX; PUSH DX; XCHG DL,DH; MOV DI,DX; SHR DX,2; ADD DI,DX; ADD DI,CX; XOR AH,AH;
    // SHL AX,1; MOV SI,AX; SHL SI,2; ADD SI,AX; ADD SI,184h; PUSH DS; MOV ES,[09e2];
    // MOV AX,05deh; MOV DS,AX; MOV CX,9
    let swapped = row.swap_bytes();
    let mut di = swapped.wrapping_add(swapped >> 2).wrapping_add(col);
    let ch = x.c.l(EAX) as u16;
    let mut si = (ch << 1).wrapping_shl(2).wrapping_add(ch << 1).wrapping_add(0x184);
    x.c.es = x.ds_rw(TARGET);
    let (seg, es, colour) = (font(x), x.c.es, x.c.l(EBX));
    x.steps += 18;
    let mut bits = 0u8;
    for _ in 0..9 {
        // LODSB; MOV AH,AL; MOV AL,BL; PUSH DI
        bits = x.bus.read8(lin(seg, si));
        si = si.wrapping_add(1);
        let start = di;
        x.steps += 4;
        loop {
            // SHL AH,1; JAE 1a57; STOSB; JMP / 1a57: JE 1a5c; INC DI; JMP
            let carry = bits & 0x80 != 0;
            bits <<= 1;
            x.steps += 2;
            if carry {
                x.bus.write8(lin(es, di), colour);
                di = di.wrapping_add(1);
                x.steps += 2;
            } else {
                x.steps += 1;
                if bits == 0 {
                    break;
                }
                di = di.wrapping_add(1);
                x.steps += 2;
            }
        }
        // POP DI; ADD DI,140h; LOOP
        di = start.wrapping_add(WIDTH);
        x.steps += 3;
    }
    // LODSB; POP DS; POP DX; POP CX; RETF
    let advance = x.bus.read8(lin(seg, si));
    si = si.wrapping_add(1);
    x.set(EAX, (bits as u16) << 8 | advance as u16);
    x.set(ESI, si);
    x.set(EDI, di);
    x.steps += 4;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:012b` (slot `DS:0258`): the width in pixels of the zero-terminated string
/// at `DS:SI` (`AX`), the sum of the signed widths at font offset 18eh + 11·(c − 20h); bytes
/// below 20h or from A0h are skipped. Leaves `ES` = the font segment.
pub fn text_width(x: &mut Ctx) -> Result<(), Decline> {
    // PUSH BX; PUSH CX; PUSH SI; MOV AX,05deh; MOV ES,AX; XOR CX,CX
    let seg = font(x);
    x.c.es = seg;
    let mut si = x.si();
    let mut total: u16 = 0;
    let mut ax = x.ax();
    x.steps += 6;
    loop {
        // LODSB; OR AL,AL; JE 155
        let c = x.bus.read8(lin(x.c.ds, si));
        si = si.wrapping_add(1);
        ax = (ax & 0xff00) | c as u16;
        x.steps += 3;
        if c == 0 {
            break;
        }
        // SUB AL,20h; JS 135
        let c = c.wrapping_sub(0x20);
        ax = (ax & 0xff00) | c as u16;
        x.steps += 2;
        if (c as i8) < 0 {
            continue;
        }
        // XOR AH,AH; MOV BX,AX; SHL BX,2; ADD BX,AX; SHL BX,1; ADD BX,AX;
        // MOV AL,ES:[BX+18eh]; CBW; ADD CX,AX; JMP
        let bx = (c as u16) * 11;
        let w = x.bus.read8(lin(seg, bx.wrapping_add(0x18e)));
        ax = w as i8 as i16 as u16;
        total = total.wrapping_add(ax);
        x.steps += 10;
    }
    // MOV AX,CX; POP SI; POP CX; POP BX; RETF
    x.set(EAX, total);
    x.steps += 4;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:015b` (slot `DS:01d0`): the byte at font offset 2eh + 11·`AL` in `AL`
/// (a character's width, *inferred*), `BX` = 11·`AL`, `ES` = the font segment.
pub fn char_width(x: &mut Ctx) -> Result<(), Decline> {
    // MOV BX,05deh; MOV ES,BX; XOR AH,AH; MOV BX,AX; SHL BX,2; ADD BX,AX; SHL BX,1; ADD BX,AX;
    // MOV AL,ES:[BX+2eh]; RETF
    let seg = font(x);
    x.c.es = seg;
    let c = x.c.l(EAX) as u16;
    let bx = c * 11;
    x.set(EBX, bx);
    let w = x.bus.read8(lin(seg, bx.wrapping_add(0x2e)));
    x.set(EAX, w as u16);
    x.steps += 9;
    x.retf();
    Ok(())
}

/// The rectangle set-up of `EL2MCGA3:06ba` and `072a`: (`AX`,`CL`)–(`BX`,`DL`) inclusive.
/// Returns (offset of the top left, width, rows − 1 as `DX` counts them, `BP` = 320 − width).
fn rect(x: &mut Ctx) -> (u16, u16, u16, u16) {
    // XOR CH,CH; MOV DH,CH; PUSH AX..ES; MOV ES,[09e2]; SUB DX,CX; XCHG CL,CH; MOV DI,CX;
    // SHR CX,2; ADD DI,CX; ADD DI,AX; SUB BX,AX; INC BX; MOV AX,BP; MOV BP,140h; SUB BP,BX
    let (x0, x1, y0, y1) = (x.ax(), x.bx(), x.c.l(ECX) as u16, x.c.l(EDX) as u16);
    x.c.es = x.ds_rw(TARGET);
    let rows = y1.wrapping_sub(y0);
    let top = (y0 << 8).wrapping_add(y0 << 6).wrapping_add(x0);
    let width = x1.wrapping_sub(x0).wrapping_add(1);
    let gap = WIDTH.wrapping_sub(width);
    x.set(EBP, gap);
    x.steps += 20;
    (top, width, rows, gap)
}

/// `POP ES; POP DI; POP DX; POP CX; POP BX; POP AX; RETF`: the rectangle fills keep the
/// registers but `BP` (and the upper halves).
fn rect_done(x: &mut Ctx, es: u16) {
    x.c.es = es;
    x.c.set_h(ECX, 0);
    x.c.set_h(EDX, 0);
    x.steps += 6;
    x.retf();
}

/// Port of `EL2MCGA3:06ba` (slot `DS:01ec`): fill the rectangle (`AX`,`CL`)–(`BX`,`DL`), corners
/// included, with colour `BP` (low byte) in the target segment: by bytes, words or dwords
/// (`REP STOS`) as the width is odd, twice odd or a multiple of 4. At least one row is filled.
pub fn fill_rect(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let colour = x.c.l(EBP);
    let entry_es = x.c.es;
    let (mut di, width, mut rows, gap) = rect(x);
    let es = x.c.es;
    // TEST BX,1; JE 6f5
    x.steps += 2;
    let unit = if width & 1 != 0 {
        1
    } else {
        // SHR BX,1; MOV AH,AL; TEST BX,1; JE 70f
        x.steps += 4;
        if (width >> 1) & 1 != 0 {
            2
        } else {
            // SHR BX,1; MOV CX,AX; SHL EAX,10h; MOV AX,CX
            let w = (colour as u32) << 8 | colour as u32;
            x.c.r[EAX] = (x.c.r[EAX] & 0xffff) | w << 16;
            x.steps += 4;
            4
        }
    };
    let count = width / unit;
    loop {
        // MOV CX,BX; REP STOS; ADD DI,BP; DEC DX; JNS
        di = fill(x, es, di, count.wrapping_mul(unit), colour);
        di = di.wrapping_add(gap);
        x.steps += 5;
        rows = rows.wrapping_sub(1);
        if (rows as i16) < 0 {
            break;
        }
    }
    rect_done(x, entry_es);
    Ok(())
}

/// Port of `EL2MCGA3:072a` (slot `DS:01f0`): as [`fill_rect`] pixel by pixel, leaving pixels of
/// colours 28h–2Bh untouched (*inferred*: the cockpit's see-through colours).
pub fn fill_rect_masked(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let colour = x.c.l(EBP);
    let entry_es = x.c.es;
    let (mut di, width, mut rows, gap) = rect(x);
    let es = x.c.es;
    loop {
        // MOV CX,BX; per pixel MOV AH,ES:[DI]; AND AH,0fch; CMP AH,28h; JE; MOV ES:[DI],AL;
        // INC DI; LOOP
        x.steps += 1;
        let mut n = width;
        loop {
            let p = x.bus.read8(lin(es, di));
            x.steps += 6;
            if p & 0xfc != 0x28 {
                x.bus.write8(lin(es, di), colour);
                x.steps += 1;
            }
            di = di.wrapping_add(1);
            n = n.wrapping_sub(1);
            if n == 0 {
                break;
            }
        }
        // ADD DI,BP; DEC DX; JNS
        di = di.wrapping_add(gap);
        x.steps += 3;
        rows = rows.wrapping_sub(1);
        if (rows as i16) < 0 {
            break;
        }
    }
    rect_done(x, entry_es);
    Ok(())
}

/// Port of `EL2MCGA3:076e` (slot `DS:0224`): pixel (`AX`, `BL`) in colour `DL`, unclipped.
/// Leaves `BH` = 0.
pub fn pixel(x: &mut Ctx) -> Result<(), Decline> {
    // XOR BH,BH; PUSH BX; PUSH DI; PUSH DS; MOV DS,[09e2]; XCHG BL,BH; MOV DI,BX; SHR BX,2;
    // ADD DI,BX; ADD DI,AX; MOV [DI],DL; POP DS; POP DI; POP BX; RETF
    let y = x.c.l(EBX) as u16;
    x.set(EBX, y);
    let seg = x.ds_rw(TARGET);
    let at = (y << 8).wrapping_add(y << 6).wrapping_add(x.ax());
    let colour = x.c.l(EDX);
    x.bus.write8(lin(seg, at), colour);
    x.steps += 14;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:0788` (slot `DS:0268`): draw into the back buffer (`DS:09e2` = `DS:09e6`).
pub fn target_back(x: &mut Ctx) -> Result<(), Decline> {
    // MOV AX,[09e6]; MOV [09e2],AX; RETF
    let v = x.ds_rw(BACK_BUFFER);
    x.set(EAX, v);
    x.ds_ww(TARGET, v);
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:078f` (slot `DS:026c`): draw into video memory (`DS:09e2` = A000h).
pub fn target_screen(x: &mut Ctx) -> Result<(), Decline> {
    // MOV [09e2],0a000h; RETF
    x.ds_ww(TARGET, 0xa000);
    x.steps += 1;
    x.retf();
    Ok(())
}

/// `MOV ES,[09e2]; XOR DI,DI; MOV EAX,v; MOV CX,3480h; REP STOSD; RETF`: the 168 rows of the
/// view in one colour.
fn clear_view(x: &mut Ctx, colour: u8) -> Result<(), Decline> {
    x.forward()?;
    x.c.es = x.ds_rw(TARGET);
    let es = x.c.es;
    let di = fill(x, es, 0, WIDTH * VIEW_HEIGHT, colour);
    x.set(EDI, di);
    x.c.r[EAX] = u32::from_ne_bytes([colour; 4]);
    x.set(ECX, 0);
    x.steps += 5;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:3489` (slot `DS:01dc`): fill the 3D view (rows 0–167) with colour 7Fh.
pub fn clear_view_7f(x: &mut Ctx) -> Result<(), Decline> {
    clear_view(x, 0x7f)
}

/// Port of `EL2MCGA3:349c` (slot `DS:01e0`): fill the 3D view (rows 0–167) with colour 0.
pub fn clear_view_0(x: &mut Ctx) -> Result<(), Decline> {
    clear_view(x, 0)
}
