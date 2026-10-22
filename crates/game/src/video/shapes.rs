//! Discs and the small shapes built from spans (docs/VIDEO.md §3).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.

use super::*;

/// The signed sum the flags of `ADD a,b` describe (`JG`/`JLE` test it, overflow included).
pub(crate) fn sum(a: u16, b: u16) -> i32 {
    a as i16 as i32 + b as i16 as i32
}

/// `MOV CX,n; [PUSH BX; PUSH DI]; CALL 1a99; [POP DI; POP BX]`: one span of the small discs,
/// the registers saved around it unless it is the shape's last.
fn small_span(x: &mut Ctx, len: u16, save: bool) {
    x.set(ECX, len);
    x.steps += if save { 4 } else { 2 };
    let (bx, di) = (x.bx(), x.di());
    span(x, SpanEntry::Unclipped);
    if save {
        x.set(EBX, bx);
        x.set(EDI, di);
        x.steps += 2;
    }
}

/// The steps of one small disc (`EL2MCGA3:2e5a` … `2f15`).
#[derive(Clone, Copy)]
enum Op {
    /// `SUB DI,n` / `DEC DI` (n = 1), `INC DI`.
    Up(u16),
    Down,
    /// `DEC BX`, `INC BX`.
    Left,
    Right,
    /// A span of this length, the last one unsaved.
    Span(u16),
    Last(u16),
}

/// The small discs, by diameter `DX` (the jump table at `CS:2e4e`): rows of 1, 2–2, 1–3–1,
/// 3–3–3, 2–4–4–2 and 1–3–5–3–1 pixels around (`BX`, `CX`).
const SMALL: [&[Op]; 6] = {
    use Op::*;
    [
        &[Last(1)],
        &[Up(1), Left, Span(2), Down, Last(2)],
        &[Up(1), Span(1), Down, Left, Span(3), Down, Right, Last(1)],
        &[Up(1), Left, Span(3), Down, Span(3), Down, Last(3)],
        &[Up(2), Left, Span(2), Down, Left, Span(4), Down, Span(4), Down, Right, Last(2)],
        &[Up(2), Span(1), Down, Left, Span(3), Down, Left, Span(5), Down, Right, Span(3), Down, Right, Last(1)],
    ]
};

/// Port of `EL2MCGA3:2f4e` (slot `DS:01d4`): filled disc of diameter `DX` centred on (`BX`,`CX`)
/// in colour `DS:57fd`, drawn only if some of it can be in the 320×168 view, then as
/// [`disc`].
pub fn disc_clipped(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let (cx_, cy, size) = (x.bx(), x.cx(), x.dx());
    let r = ((size as i16) >> 1) as u16;
    // 2f4e: CMP BX,140h; JAE 2f1c
    x.steps += 2;
    if cx_ >= WIDTH {
        // 2f1c: MOV DI,DX; SAR DI,1; OR BX,BX; JS 2f2e
        x.set(EDI, r);
        x.steps += 4;
        let visible = if (cx_ as i16) < 0 {
            // 2f2e: MOV AX,BX; ADD AX,DI; JG 2f54
            let ax = cx_.wrapping_add(r);
            x.set(EAX, ax);
            x.steps += 3;
            sum(cx_, r) > 0
        } else {
            // MOV AX,BX; SUB AX,DI; CMP AX,140h; JL 2f54
            let ax = cx_.wrapping_sub(r);
            x.set(EAX, ax);
            x.steps += 4;
            (ax as i16) < WIDTH as i16
        };
        if !visible {
            x.retf();
            return Ok(());
        }
    }
    // 2f54: CMP CX,0a8h; JAE 2f35
    x.steps += 2;
    if cy >= VIEW_HEIGHT {
        // 2f35: MOV DI,DX; SAR DI,1; OR CX,CX; JS 2f47
        x.set(EDI, r);
        x.steps += 4;
        let visible = if (cy as i16) < 0 {
            let ax = cy.wrapping_add(r);
            x.set(EAX, ax);
            x.steps += 3;
            sum(cy, r) > 0
        } else {
            let ax = cy.wrapping_sub(r);
            x.set(EAX, ax);
            x.steps += 4;
            (ax as i16) < VIEW_HEIGHT as i16
        };
        if !visible {
            x.retf();
            return Ok(());
        }
    }
    disc(x)
}

/// Port of `EL2MCGA3:2f5a` (slot `DS:01d8`): filled disc of diameter `DX` (< 3F0h) centred on
/// (`BX`,`CX`) in colour `DS:57fd` into the target segment, clipped to the view. Diameters
/// below 6 are fixed shapes ([`SMALL`], drawn only when the centre is 2 pixels inside the
/// view, unclipped). Larger discs are rasterised by a midpoint circle that writes each row's
/// (left x, width) into a table on the stack, the upper and lower halves at once, then fill the
/// rows that fall in the view.
pub fn disc(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let (cx_, cy, size) = (x.bx(), x.cx(), x.dx());
    // 2f5a: CMP DX,3f0h; JAE 2f4d (RETF); CMP DX,6; JB 2e21
    x.steps += 2;
    if size >= 0x3f0 {
        x.retf();
        return Ok(());
    }
    x.steps += 2;
    if size < 6 {
        return small_disc(x);
    }
    // 2f67: store centre and radius; ES; SUB SP,0fc0h
    let r = size >> 1;
    x.ds_ww(0x1f23, cx_);
    x.ds_ww(0x1f25, cy);
    x.ds_ww(0x1f27, r);
    x.c.es = x.ds_rw(TARGET);
    let sp = x.sp().wrapping_sub(0xfc0);
    x.steps += 7;
    // XOR DI,DI; MOV SI,DX; SHL SI,1; AND SI,0fffch; MOV BX,DI; SUB DX,2; CMP DX,5; JNE
    let mut di: u16 = 0;
    let mut si = (size << 1) & 0xfffc;
    let mut bx: u16 = 0;
    let mut dx = size.wrapping_sub(2);
    x.steps += 8;
    if dx == 5 {
        dx = dx.wrapping_add(3);
        x.steps += 1;
    }
    // NEG DX; MOV BP,SP; table pointers [1f29], [1f2b], [1f2d], [1f2f]; JMP 2fde
    dx = dx.wrapping_neg();
    let mut top = sp; // [1f29]: rows from the top, downwards
    let mut upper = sp.wrapping_add(si); // [1f2b]: rows above the middle, upwards
    let mut lower = upper; // [1f2d]: rows below the middle, downwards
    let mut bottom = upper.wrapping_add(si); // [1f2f]: rows from the bottom, upwards
    x.steps += 9;
    let mut cx = r;
    let ss = x.c.ss;
    let put = |x: &mut Ctx, at: u16, left: u16, width: u16| {
        x.bus.write16(lin(ss, at), left);
        x.bus.write16(lin(ss, at.wrapping_add(2)), width);
    };
    let mut skip_add = false;
    loop {
        if !skip_add {
            // 2fde: ADD DX,DI; ADD DX,6
            dx = dx.wrapping_add(di).wrapping_add(6);
            x.steps += 2;
        }
        skip_add = false;
        // 2fe3: ADD DI,4; INC BX; CMP SI,DI; JG 2faf; JNE 3052
        di = di.wrapping_add(4);
        bx = bx.wrapping_add(1);
        x.steps += 4;
        let middle = (si as i16) == (di as i16);
        if (si as i16) > (di as i16) || middle {
            if !middle {
                x.steps += 0;
            } else {
                x.steps += 1;
            }
            // 2faf / 2fed: the rows at ±CX: left = x − CX, width 2·CX, into [1f2b] and [1f2d]
            let left = x.ds_rw(0x1f23).wrapping_sub(cx);
            let width = cx << 1;
            upper = upper.wrapping_sub(4);
            put(x, upper, left, width);
            put(x, lower, left, width);
            lower = lower.wrapping_add(4);
            if middle {
                x.steps += 15;
                break;
            }
            x.steps += 16;
            if (dx as i16) < 0 {
                continue;
            }
            // 301a: the rows at ±BX into [1f29] and [1f2f]; DX += DI − SI + 0ah; SI −= 4;
            // DEC CX; JMP 2fe3
            let left = x.ds_rw(0x1f23).wrapping_sub(bx);
            let width = bx << 1;
            put(x, top, left, width);
            top = top.wrapping_add(4);
            bottom = bottom.wrapping_sub(4);
            put(x, bottom, left, width);
            dx = dx.wrapping_add(di).wrapping_sub(si).wrapping_add(0xa);
            si = si.wrapping_sub(4);
            cx = cx.wrapping_sub(1);
            x.steps += 20;
            skip_add = true;
        } else {
            x.steps += 1;
            break;
        }
    }
    x.ds_ww(0x1f29, top);
    x.ds_ww(0x1f2b, upper);
    x.ds_ww(0x1f2d, lower);
    x.ds_ww(0x1f2f, bottom);
    // 3052: MOV BP,SP; MOV AX,[1f25]; SUB AX,[1f27]; JNS 3066
    let mut bp = sp;
    let mut first = cy.wrapping_sub(r);
    x.steps += 4;
    if (first as i16) < 0 {
        // NEG AX; SHL AX,2; ADD BP,AX; XOR AX,AX
        bp = bp.wrapping_add(first.wrapping_neg() << 2);
        first = 0;
        x.steps += 4;
    }
    // 3066: MOV DI,AX; MOV SI,[1f23]; CMP SI,[1f27]; JL 30b5; ADD SI,[1f27]; CMP SI,140h; JG 30b5
    x.steps += 4;
    let mut inside = false;
    let mut si_out = cx_;
    if (cx_ as i16) >= (r as i16) {
        si_out = cx_.wrapping_add(r);
        x.steps += 3;
        inside = (si_out as i16) <= WIDTH as i16;
    }
    x.set(ESI, si_out);
    // MOV CX,[1f27]; ADD CX,[1f25]; (30b5: JLE 3103); CMP CX,0a8h; JL; MOV CX,0a8h
    let mut last = r.wrapping_add(cy);
    x.steps += if inside { 4 } else { 3 };
    if !inside && sum(r, cy) <= 0 {
        x.set(ECX, last);
        x.set(EDI, first);
        x.set(EAX, first);
        x.set(EBP, bp);
        x.set(EBX, bx);
        x.set(EDX, dx);
        x.steps += 2; // ADD SP,0fc0h; RETF
        x.c.retf(x.bus);
        return Ok(());
    }
    if !inside {
        x.steps += 2; // CMP CX,0a8h; JL
    }
    if (last as i16) >= VIEW_HEIGHT as i16 {
        last = VIEW_HEIGHT;
        x.steps += 1;
    }
    // SUB CX,AX; MOV [57bd],CX; MOV AH,[57fd]; MOVZX EDI,DI
    let rows = last.wrapping_sub(first);
    if rows as i16 <= 0 {
        return Err(Decline("disc with no rows in view (the original runs 65536 rows)"));
    }
    x.ds_ww(ROWS, rows);
    x.set(EAX, first);
    let colour = x.ds_rb(COLOUR);
    x.c.set_h(EAX, colour);
    x.c.r[EDI] = first as u32;
    x.set(ECX, rows);
    x.set(EBX, bx);
    x.set(EDX, dx);
    x.steps += 4;
    let mut n = rows;
    loop {
        // MOV BX,[BP]; MOV CX,[BP+2]; ADD BP,4
        let mut left = x.bus.read16(lin(ss, bp));
        let mut width = x.bus.read16(lin(ss, bp.wrapping_add(2)));
        bp = bp.wrapping_add(4);
        x.set(EBX, left);
        x.set(ECX, width);
        x.set(EBP, bp);
        x.steps += 3;
        let di = x.di();
        if inside {
            // PUSH DI; CALL 1a99; POP DI
            x.steps += 2;
            span(x, SpanEntry::Unclipped);
            x.set(EDI, di);
            x.steps += 1;
        } else {
            // 30df: OR BX,BX; JNS 30e9; ADD CX,BX; JLE 30fc; XOR BX,BX
            x.steps += 2;
            let mut draw = true;
            if (left as i16) < 0 {
                let total = sum(width, left);
                width = width.wrapping_add(left);
                x.set(ECX, width);
                x.steps += 2;
                if total <= 0 {
                    draw = false;
                } else {
                    left = 0;
                    x.set(EBX, 0);
                    x.steps += 1;
                }
            }
            if draw {
                // 30e9: MOV DX,BX; ADD DX,CX; SUB DX,140h; JL 30f7; SUB CX,DX; JLE 30fc
                let over = left.wrapping_add(width).wrapping_sub(WIDTH);
                x.set(EDX, over);
                x.steps += 4;
                if (left.wrapping_add(width) as i16 as i32) - WIDTH as i32 >= 0 {
                    let rest = width as i16 as i32 - over as i16 as i32;
                    width = width.wrapping_sub(over);
                    x.set(ECX, width);
                    x.steps += 2;
                    draw = rest > 0;
                }
            }
            if draw {
                // 30f7: PUSH DI; CALL 1a99; POP DI
                x.steps += 2;
                span(x, SpanEntry::Unclipped);
                x.set(EDI, di);
                x.steps += 1;
            }
        }
        // INC DI; DEC [57bd]; JNE
        x.set(EDI, x.di().wrapping_add(1));
        n = n.wrapping_sub(1);
        x.steps += 3;
        if n == 0 {
            break;
        }
    }
    x.ds_ww(ROWS, 0);
    // ADD SP,0fc0h; RETF
    x.steps += 1;
    x.retf();
    Ok(())
}

/// `EL2MCGA3:2e21`: the small discs, drawn only if the centre is at least 2 pixels inside the
/// view (signed), unclipped.
fn small_disc(x: &mut Ctx) -> Result<(), Decline> {
    let (cx_, cy, size) = (x.bx(), x.cx(), x.dx());
    // CMP BX,2; JL; CMP BX,13eh; JGE; CMP CX,2; JL; CMP CX,0a6h; JGE (RETF at 2e20)
    for ok in [(cx_ as i16) >= 2, (cx_ as i16) < 0x13e, (cy as i16) >= 2, (cy as i16) < 0xa6] {
        x.steps += 2;
        if !ok {
            x.retf();
            return Ok(());
        }
    }
    // MOV ES,[09e2]; MOV AH,[57fd]; MOVZX EDI,DI; MOV DI,CX; MOV SI,DX; SHL SI,1; JMP [SI+2e4e]
    x.c.es = x.ds_rw(TARGET);
    let colour = x.ds_rb(COLOUR);
    x.c.set_h(EAX, colour);
    x.c.r[EDI] = cy as u32;
    x.set(ESI, size << 1);
    x.steps += 7;
    for &op in SMALL[size as usize] {
        match op {
            Op::Up(n) => {
                x.set(EDI, x.di().wrapping_sub(n));
                x.steps += 1;
            }
            Op::Down => {
                x.set(EDI, x.di().wrapping_add(1));
                x.steps += 1;
            }
            Op::Left => {
                x.set(EBX, x.bx().wrapping_sub(1));
                x.steps += 1;
            }
            Op::Right => {
                x.set(EBX, x.bx().wrapping_add(1));
                x.steps += 1;
            }
            Op::Span(n) => small_span(x, n, true),
            Op::Last(n) => small_span(x, n, false),
        }
    }
    x.retf();
    Ok(())
}

/// Code segment offset of the blob shapes [`blob`] draws: per shape, one byte per row, the
/// half-width (0 = an empty row), the shape ended by a negative byte.
pub const BLOB_TABLE: u16 = 0x2c11;

/// Port of `EL2MCGA3:2dd1` (slot `DS:0214`): a small shape from the table at `CS:2c11`
/// ([`BLOB_TABLE`]), starting at byte `BP`, centred on (`BX`+2, `DI`−6) in colour `DS:57fd`:
/// each row of half-width w is a span of 2w−1 pixels, clamped to 0 ≤ x ≤ 319; rows outside the
/// view are skipped.
pub fn blob(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // MOV ES,[09e2]; MOVZX EDI,DI; ADD BX,2; SUB DI,7; MOV DX,BX
    x.c.es = x.ds_rw(TARGET);
    x.c.r[EDI] &= 0xffff;
    let centre = x.bx().wrapping_add(2);
    x.set(EBX, centre);
    x.set(EDI, x.di().wrapping_sub(7));
    x.set(EDX, centre);
    x.steps += 5;
    let cs = x.c.cs;
    loop {
        // 2de1: INC DI; MOV AL,CS:[BP+2c11]; INC BP; OR AL,AL; JE 2de1; JS 2e1f
        x.set(EDI, x.di().wrapping_add(1));
        let bp = x.bp();
        let w = x.bus.read8(lin(cs, bp.wrapping_add(BLOB_TABLE)));
        x.c.set_l(EAX, w);
        x.set(EBP, bp.wrapping_add(1));
        x.steps += 5;
        if w == 0 {
            continue;
        }
        x.steps += 1;
        if (w as i8) < 0 {
            x.retf();
            return Ok(());
        }
        // CMP DI,0a8h; JAE 2de1
        x.steps += 2;
        if x.di() >= VIEW_HEIGHT {
            continue;
        }
        // CBW; MOV CX,AX; MOV BX,DX; SUB BX,CX; JNS; XOR BX,BX
        let mut cx = w as u16;
        let mut bx = centre.wrapping_sub(cx);
        x.steps += 5;
        if (bx as i16) < 0 {
            bx = 0;
            x.steps += 1;
        }
        // SHL CX,1; DEC CX; MOV AX,BX; ADD AX,CX; CMP AX,140h; JL; MOV CX,13fh; SUB CX,BX
        cx = (cx << 1).wrapping_sub(1);
        let ax = bx.wrapping_add(cx);
        x.steps += 6;
        if (ax as i16) >= WIDTH as i16 {
            cx = 0x13f_u16.wrapping_sub(bx);
            x.steps += 2;
        }
        // PUSH BX; PUSH DX; PUSH DI; MOV AH,[57fd]; CALL 1a99; POP DI; POP DX; POP BX; JMP
        x.set(EAX, ax);
        x.set(EBX, bx);
        x.set(ECX, cx);
        let colour = x.ds_rb(COLOUR);
        x.c.set_h(EAX, colour);
        let di = x.di();
        x.steps += 5;
        span(x, SpanEntry::Unclipped);
        x.set(EDI, di);
        x.set(EDX, centre);
        x.set(EBX, bx);
        x.steps += 4;
    }
}
