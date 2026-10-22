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

/// Paragraph offset of the buffer the mouse pointer's background is saved in (`EL2MCGA3`
/// segment `05d7`, from offset 0Dh).
pub const UNDER_PARAS: u16 = 0x5d7;
/// The pointer's position (`DS:0a0c`, `0a0e`) and bitmap number (`DS:0a0a`).
pub const POINTER_X: u16 = 0x0a0c;
pub const POINTER_Y: u16 = 0x0a0e;
pub const POINTER_BITMAP: u16 = 0x0a0a;

fn cs_byte(x: &mut Ctx, off: u16) -> u8 {
    x.bus.read8(lin(x.c.cs, off))
}
fn cs_word(x: &mut Ctx, off: u16) -> u16 {
    x.bus.read16(lin(x.c.cs, off))
}

/// Port of `EL2MCGA3:04d8` (slot `DS:0240`): save the screen under the mouse pointer from `ES`.
/// The pointer bitmap's header is at `CS:0106` (flags, height, width, x and y offsets); its
/// rectangle at the pointer position, clipped to 320×200 and widened to whole dwords, is copied
/// to the buffer at `CS:05d7` paragraphs + 0Dh. `SS:0` = the rectangle's screen offset, or FFFFh
/// when nothing was saved; `CS:0124`… keep its size for [`restore_under`].
pub fn save_under(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let ss = x.c.ss;
    // MOV SS:[0],0ffffh; MOVZX DX,CS:[107]; MOV AX,[0a0e]; ADD AX,CS:[10c]; JNS 4fb
    x.bus.write16(lin(ss, 0), 0xffff);
    let mut dx = cs_byte(x, 0x107) as u16;
    let mut ax = x.ds_rw(POINTER_Y).wrapping_add(cs_word(x, 0x10c));
    x.set(EDX, dx);
    x.set(EAX, ax);
    x.steps += 5;
    if (ax as i16) < 0 {
        // ADD DX,AX; JS 582; JE 582; XOR AX,AX
        dx = dx.wrapping_add(ax);
        x.set(EDX, dx);
        x.steps += 2;
        if (dx as i16) < 0 {
            x.retf();
            return Ok(());
        }
        x.steps += 1;
        if dx == 0 {
            x.retf();
            return Ok(());
        }
        ax = 0;
        x.set(EAX, 0);
        x.steps += 1;
    }
    // 4fb: MOV BP,AX; ADD BP,DX; SUB BP,0c8h; JLE 509; SUB DX,BP; JLE 582
    let end = ax.wrapping_add(dx);
    let bp = end.wrapping_sub(200);
    x.set(EBP, bp);
    x.steps += 4;
    if (end as i16 as i32) - 200 > 0 {
        let left = dx as i16 as i32 - bp as i16 as i32;
        dx = dx.wrapping_sub(bp);
        x.set(EDX, dx);
        x.steps += 2;
        if left <= 0 {
            x.retf();
            return Ok(());
        }
    }
    // 509: MOV CS:[129],AX; XCHG AH,AL; MOV SI,AX; SHR AX,2; ADD SI,AX
    let cs = x.c.cs;
    x.bus.write16(lin(cs, 0x129), ax);
    let sw = ax.swap_bytes();
    let mut si = sw.wrapping_add(sw >> 2);
    x.set(ESI, si);
    x.steps += 5;
    // 516: MOV BX,CS:[108]; MOV AX,[0a0c]; ADD AX,CS:[10a]; AND AX,0fffch; JNS 530
    let mut bx = cs_word(x, 0x108);
    ax = x.ds_rw(POINTER_X).wrapping_add(cs_word(x, 0x10a)) & 0xfffc;
    x.set(EBX, bx);
    x.set(EAX, ax);
    x.steps += 5;
    if (ax as i16) < 0 {
        // ADD BX,AX; JS 582; JE 582; XOR AX,AX
        bx = bx.wrapping_add(ax);
        x.set(EBX, bx);
        x.steps += 2;
        if (bx as i16) < 0 {
            x.retf();
            return Ok(());
        }
        x.steps += 1;
        if bx == 0 {
            x.retf();
            return Ok(());
        }
        ax = 0;
        x.set(EAX, 0);
        x.steps += 1;
    }
    // 530: MOV BP,AX; ADD BP,BX; SUB BP,140h; JLE 53e; SUB BX,BP; JLE 582
    let end = ax.wrapping_add(bx);
    let bp = end.wrapping_sub(WIDTH);
    x.set(EBP, bp);
    x.steps += 4;
    if (end as i16 as i32) - WIDTH as i32 > 0 {
        let left = bx as i16 as i32 - bp as i16 as i32;
        bx = bx.wrapping_sub(bp);
        x.set(EBX, bx);
        x.steps += 2;
        if left <= 0 {
            x.retf();
            return Ok(());
        }
    }
    // 53e: ADD BX,AX; ADD BX,7; AND BL,0fch; SUB BX,AX: the width in whole dwords
    bx = bx.wrapping_add(ax).wrapping_add(7);
    bx &= 0xfffc;
    bx = bx.wrapping_sub(ax);
    // MOV CS:[127],AX; ADD SI,AX; MOV CS:[124],BX; MOV CS:[126],DL; MOV BP,140h; SUB BP,BX;
    // SHR BX,2; PUSH DS; MOV AX,ES; MOV DS,AX; MOV AX,05d7h; MOV ES,AX; MOV SS:[0],SI;
    // MOV DI,0dh
    x.bus.write16(lin(cs, 0x127), ax);
    si = si.wrapping_add(ax);
    x.bus.write16(lin(cs, 0x124), bx);
    x.bus.write8(lin(cs, 0x126), dx as u8);
    let gap = WIDTH.wrapping_sub(bx);
    let dwords = bx >> 2;
    let src = x.c.es;
    let buf = cs.wrapping_add(UNDER_PARAS);
    x.bus.write16(lin(ss, 0), si);
    let mut di: u16 = 0xd;
    x.steps += 18;
    let mut rows = dx as u8;
    loop {
        // MOV CX,BX; REP MOVSD; ADD SI,BP; DEC DL; JNE
        for _ in 0..dwords as u32 * 4 {
            let b = x.bus.read8(lin(src, si));
            x.bus.write8(lin(buf, di), b);
            si = si.wrapping_add(1);
            di = di.wrapping_add(1);
        }
        si = si.wrapping_add(gap);
        rows = rows.wrapping_sub(1);
        x.steps += 5;
        if rows == 0 {
            break;
        }
    }
    // MOV AX,DS; MOV ES,AX; POP DS; RETF
    x.set(EAX, src);
    x.set(EBX, dwords);
    x.set(ECX, 0);
    x.set(EDX, dx & 0xff00);
    x.set(ESI, si);
    x.set(EDI, di);
    x.set(EBP, gap);
    x.steps += 3;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:0583` (slot `DS:0230`): put the background saved by [`save_under`] back
/// into `ES` at `SS:0` (taking it: `SS:0` = FFFFh after), if any was saved.
pub fn restore_under(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // PUSH DS; MOV AX,05d7h; MOV DS,AX; MOV DI,0ffffh; XCHG SS:[0],DI; MOV SI,0dh;
    // CMP DI,-1; JE 5b6
    let buf = x.c.cs.wrapping_add(UNDER_PARAS);
    x.set(EAX, buf);
    let ss = x.c.ss;
    let mut di = x.bus.read16(lin(ss, 0));
    x.bus.write16(lin(ss, 0), 0xffff);
    let mut si: u16 = 0xd;
    x.set(EDI, di);
    x.set(ESI, si);
    x.steps += 8;
    if di != 0xffff {
        // MOV DL,CS:[126]; MOV BX,CS:[124]; MOV BP,140h; SUB BP,BX; SHR BX,2
        let mut rows = cs_byte(x, 0x126);
        let width = cs_word(x, 0x124);
        let gap = WIDTH.wrapping_sub(width);
        let dwords = width >> 2;
        let es = x.c.es;
        x.steps += 5;
        loop {
            // MOV CX,BX; REP MOVSD; ADD DI,BP; DEC DL; JNE
            for _ in 0..dwords as u32 * 4 {
                let b = x.bus.read8(lin(buf, si));
                x.bus.write8(lin(es, di), b);
                si = si.wrapping_add(1);
                di = di.wrapping_add(1);
            }
            di = di.wrapping_add(gap);
            rows = rows.wrapping_sub(1);
            x.steps += 5;
            if rows == 0 {
                break;
            }
        }
        x.set(EDX, x.dx() & 0xff00);
        x.set(EBX, dwords);
        x.set(ECX, 0);
        x.set(EBP, gap);
        x.set(ESI, si);
        x.set(EDI, di);
    }
    // POP DS; RETF
    x.steps += 1;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:0681` (slot `DS:0254`): make bitmap `BX` the mouse pointer, unless it is
/// already (`DS:0a0a`): hide the pointer (the main program's `DS:0018`), then
/// [`pointer_after_hide`].
pub fn set_pointer(x: &mut Ctx) -> Result<(), Decline> {
    // CMP BX,[0a0a]; JE 6b9; PUSH BX; CALLF [0018]
    x.steps += 2;
    if x.bx() == x.ds_rw(POINTER_BITMAP) {
        x.retf();
        return Ok(());
    }
    let bx = x.bx();
    x.push(bx);
    x.steps += 1;
    x.call_slot(0x0018, 0x068c);
    Ok(())
}

/// Port of `EL2MCGA3:068c`: look the new pointer bitmap up through the blitter's slot
/// (`DS:0220` with `DI` = 0).
pub fn pointer_after_hide(x: &mut Ctx) -> Result<(), Decline> {
    // POP BX; PUSH BX; XOR DI,DI; INC [1fa9]; CALLF [0220]
    let bx = x.pop();
    x.push(bx);
    x.set(EBX, bx);
    x.set(EDI, 0);
    let n = x.ds_rw(BLIT_BUSY).wrapping_add(1);
    x.ds_ww(BLIT_BUSY, n);
    x.steps += 4;
    x.call_slot(0x0220, 0x0698);
    Ok(())
}

/// Port of `EL2MCGA3:0698`: copy the new pointer's header (`ES:BP`) to `CS:0106`, record its
/// number in `DS:0a0a`, and show the pointer again (the main program's `DS:0024`).
pub fn pointer_after_lookup(x: &mut Ctx) -> Result<(), Decline> {
    // MOV EAX,ES:[BP]; MOV CS:[106],EAX; MOV EAX,ES:[BP+4]; MOV CS:[10a],EAX; DEC [1fa9];
    // POP BX; XCHG [0a0a],BX; CALLF [0024]
    let (es, bp, cs) = (x.c.es, x.bp(), x.c.cs);
    let a = x.bus.read32(lin(es, bp));
    x.bus.write32(lin(cs, 0x106), a);
    let b = x.bus.read32(lin(es, bp.wrapping_add(4)));
    x.bus.write32(lin(cs, 0x10a), b);
    x.c.r[EAX] = b;
    let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
    x.ds_ww(BLIT_BUSY, n);
    let bx = x.pop();
    let old = x.ds_rw(POINTER_BITMAP);
    x.ds_ww(POINTER_BITMAP, bx);
    x.set(EBX, old);
    x.steps += 7;
    x.call_slot(0x0024, 0x06b9);
    Ok(())
}

/// Port of `EL2MCGA3:06b9`, `34c8` and `34e6`: the `RETF` after a call out.
pub fn retf_after_call(x: &mut Ctx) -> Result<(), Decline> {
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:34ac` (slot `DS:01e8`): hide the pointer (`DS:0018`), then
/// [`copy_after_hide`].
pub fn copy_screen(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x0018, 0x34b0);
    Ok(())
}

/// Port of `EL2MCGA3:34b0`: copy the whole screen from segment `DS:09e4` to the target
/// (`DS:09e2`), then show the pointer (`DS:0024`).
pub fn copy_after_hide(x: &mut Ctx) -> Result<(), Decline> {
    copy_block(x, false, 0, 0x3e80, 0x34c8)
}

/// Port of `EL2MCGA3:34c9` (slot `DS:01e4`): hide the pointer (`DS:0018`), then
/// [`panel_after_hide`].
pub fn copy_panel(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x0018, 0x34cd);
    Ok(())
}

/// Port of `EL2MCGA3:34cd`: copy rows 168–199 (the control panel) from the target (`DS:09e2`)
/// to segment `DS:09e4`, then show the pointer (`DS:0024`).
pub fn panel_after_hide(x: &mut Ctx) -> Result<(), Decline> {
    copy_block(x, true, 0xd200, 0xa00, 0x34e6)
}

/// `PUSH DS; MOV ES,..; MOV DS,..; MOV SI,start; MOV DI,SI; MOV CX,n; REP MOVSD; POP DS;
/// CALLF [0024]`: `n` dwords between the target and `DS:09e4` (`to_other` from the target).
fn copy_block(x: &mut Ctx, to_other: bool, start: u16, dwords: u16, ret: u16) -> Result<(), Decline> {
    x.forward()?;
    let (target, other) = (x.ds_rw(TARGET), x.ds_rw(0x09e4));
    let (src, dst) = if to_other { (target, other) } else { (other, target) };
    x.c.es = dst;
    let mut si = start;
    for _ in 0..dwords as u32 * 4 {
        let b = x.bus.read8(lin(src, si));
        x.bus.write8(lin(dst, si), b);
        si = si.wrapping_add(1);
    }
    x.set(ESI, si);
    x.set(EDI, si);
    x.set(ECX, 0);
    x.steps += 8;
    x.call_slot(0x0024, ret);
    Ok(())
}
