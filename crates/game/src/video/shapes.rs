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
enum SmallOp {
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
const SMALL: [&[SmallOp]; 6] = {
    use SmallOp::*;
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
            SmallOp::Up(n) => {
                x.set(EDI, x.di().wrapping_sub(n));
                x.steps += 1;
            }
            SmallOp::Down => {
                x.set(EDI, x.di().wrapping_add(1));
                x.steps += 1;
            }
            SmallOp::Left => {
                x.set(EBX, x.bx().wrapping_sub(1));
                x.steps += 1;
            }
            SmallOp::Right => {
                x.set(EBX, x.bx().wrapping_add(1));
                x.steps += 1;
            }
            SmallOp::Span(n) => small_span(x, n, true),
            SmallOp::Last(n) => small_span(x, n, false),
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

/// Offsets in the scanline-list frame ([`fill_rows`]), from `BP` = `SP` at entry.
pub const ROWS_COUNT: u16 = 0xe78;
pub const ROWS_TOP: u16 = 0xe76;
pub const ROWS_INSIDE: u16 = 0xe74;
/// Distance between the crossing arrays of the frame (one word per row, 168 rows).
pub const ROWS_STRIDE: u16 = 0x150;

/// One pair of crossings of [`fill_rows`]: `SUB CX,BX; PUSH DI; MOV AH,[57fd]; CALL 1a99;
/// POP DI`, after clamping to 0 ≤ x ≤ 320 when clipping (`323b`: a left end at or past 320 or
/// a negative right end skips the pair).
fn crossing_pair(x: &mut Ctx, clip: bool) {
    if clip {
        // CMP BX,140h; JB; JGE skip; XOR BX,BX
        x.steps += 2;
        if x.bx() >= WIDTH {
            x.steps += 1;
            if (x.bx() as i16) >= WIDTH as i16 {
                return;
            }
            x.set(EBX, 0);
            x.steps += 1;
        }
        // CMP CX,141h; JB; JL skip; MOV CX,140h
        x.steps += 2;
        if x.cx() >= WIDTH + 1 {
            x.steps += 1;
            if (x.cx() as i16) < (WIDTH + 1) as i16 {
                return;
            }
            x.set(ECX, WIDTH);
            x.steps += 1;
        }
    }
    x.set(ECX, x.cx().wrapping_sub(x.bx()));
    let colour = x.ds_rb(COLOUR);
    x.c.set_h(EAX, colour);
    let di = x.di();
    x.steps += 4;
    span(x, SpanEntry::Unclipped);
    x.set(EDI, di);
    x.steps += 1;
}

/// Insert `v` into the ascending registers `regs` with a compare chain (`CMP v,r; JLE`) and
/// an exchange chain that rotates it in (e.g. `3172`–`319a`); the last register gets the
/// largest value.
fn insert(x: &mut Ctx, sorted: &[usize], v: usize) {
    for (i, &r) in sorted.iter().enumerate() {
        x.steps += 2;
        if (x.c.x(v) as i16) <= (x.c.x(r) as i16) {
            // XCHG down the chain: each register takes the value below it.
            let mut carry = x.c.x(v);
            for &s in &sorted[i..] {
                let old = x.c.x(s);
                x.set(s, carry);
                carry = old;
            }
            x.set(v, carry);
            x.steps += (sorted.len() - i) as u64;
            return;
        }
    }
    x.steps += 1; // JMP past the exchanges
}

/// Port of `EL2MCGA3:3108` (slot `DS:01fc`): fill the polygon described by the scanline lists
/// in the caller's stack frame (as `EL2MATH3`'s `frame_e76` gives it), then free the frame
/// (`RETF 0e76h`). From `BP` = `SP`: `[BP+0e78h]` the number of rows, `[BP+0e76h]` the top row,
/// `[BP+0e74h]` zero if the polygon may cross the screen's sides; per row y the word at
/// `BP+6+2y` is the end of its crossing list (0 = empty, 2A0h = 2 crossings, 540h = 4, more
/// = 6) and the crossings are at `+150h`, `+2a0h`, … `+7e0h` from it. Rows are filled from the
/// bottom up: the crossings sorted and filled in pairs, colour `DS:57fd`. An empty row at or
/// above the top row ends the fill.
pub fn fill_rows(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // MOVZX EDI,DI; MOV ES,[09e2]; MOV BP,SP; MOV AX,[BP+0e78h]; DEC AX; JS 3218
    x.c.r[EDI] &= 0xffff;
    x.c.es = x.ds_rw(TARGET);
    let bp = x.sp();
    x.set(EBP, bp);
    let ss = x.c.ss;
    let frame = |x: &mut Ctx, off: u16| x.bus.read16(lin(ss, bp.wrapping_add(off)));
    let n = frame(x, ROWS_COUNT).wrapping_sub(1);
    x.set(EAX, n);
    x.steps += 6;
    if (n as i16) >= 0 {
        // MOV SI,AX; SHL SI,1; LEA SI,[BP+SI+6]; MOV DI,AX; MOV [57bd],AX; CMP [BP+0e74h],0; JE
        x.set(ESI, bp.wrapping_add(n << 1).wrapping_add(6));
        x.set(EDI, n);
        x.ds_ww(ROWS, n);
        let clip = frame(x, ROWS_INSIDE) == 0;
        x.steps += 7;
        let top = frame(x, ROWS_TOP);
        #[derive(Clone, Copy)]
        enum At {
            Row,
            Next,
            Check,
        }
        let mut pc = At::Row;
        loop {
            match pc {
                At::Row => {
                    // 3130: SUB SI,2; MOV DX,[SI]; OR DX,DX; JE 3166
                    x.set(ESI, x.si().wrapping_sub(2));
                    let end = x.bus.read16(lin(ss, x.si()));
                    x.set(EDX, end);
                    x.steps += 4;
                    if end == 0 {
                        pc = At::Check;
                        continue;
                    }
                    fill_crossings(x, end, clip);
                    pc = At::Next;
                }
                At::Next => {
                    // 315b: DEC DI; DEC [57bd]; JNS 3130; DEC [57bd]
                    x.set(EDI, x.di().wrapping_sub(1));
                    let r = x.ds_rw(ROWS).wrapping_sub(1);
                    x.ds_ww(ROWS, r);
                    x.steps += 3;
                    if (r as i16) >= 0 {
                        pc = At::Row;
                        continue;
                    }
                    x.ds_ww(ROWS, r.wrapping_sub(1));
                    x.steps += 1;
                    pc = At::Check;
                }
                At::Check => {
                    // 3166: MOV AX,[57bd]; CMP AX,[BP+0e76h]; JG 315b; JMP 3218
                    let row = x.ds_rw(ROWS);
                    x.set(EAX, row);
                    x.steps += 3;
                    if (row as i16) > (top as i16) {
                        pc = At::Next;
                        continue;
                    }
                    x.steps += 1;
                    break;
                }
            }
        }
    }
    // RETF 0e76h
    x.retf();
    x.set(ESP, x.sp().wrapping_add(0xe76));
    Ok(())
}

/// One row of [`fill_rows`] with crossings, the list ending at `end` (`313a`, `3225`): the
/// crossings read, sorted by insertion and filled in pairs from the left.
fn fill_crossings(x: &mut Ctx, end: u16, clip: bool) {
    let ss = x.c.ss;
    let at = |x: &mut Ctx, off: u16| x.bus.read16(lin(ss, x.si().wrapping_add(off)));
    // MOV BX,[SI+150h]; MOV CX,[SI+2a0h]; CMP CX,BX; JG; XCHG CX,BX; CMP DX,2a0h; JG
    let (b, c) = (at(x, ROWS_STRIDE), at(x, 2 * ROWS_STRIDE));
    x.set(EBX, b);
    x.set(ECX, c);
    x.steps += 4;
    if (c as i16) <= (b as i16) {
        x.set(EBX, c);
        x.set(ECX, b);
        x.steps += 1;
    }
    x.steps += 2;
    if (end as i16) > 0x2a0 {
        // 3172: MOV AX,[SI+3f0h]; MOV DX,[SI+540h]; insert both; CMP [SI],540h; JG
        let (a, d) = (at(x, 3 * ROWS_STRIDE), at(x, 4 * ROWS_STRIDE));
        x.set(EAX, a);
        x.set(EDX, d);
        x.steps += 2;
        insert(x, &[EBX, ECX], EAX);
        insert(x, &[EBX, ECX, EAX], EDX);
        x.steps += 2;
        let mut pending = vec![(x.ax(), x.dx())];
        if (end as i16) > 0x540 {
            // 31b3: save DI, SI; MOV DI,[SI+690h]; MOV SI,[SI+7e0h]; insert both
            let (row, list) = (x.di(), x.si());
            x.ds_ww(0x1f31, row);
            x.ds_ww(0x1f33, list);
            let (e, f) = (at(x, 5 * ROWS_STRIDE), at(x, 6 * ROWS_STRIDE));
            x.set(EDI, e);
            x.set(ESI, f);
            x.steps += 4;
            insert(x, &[EBX, ECX, EAX, EDX], EDI);
            insert(x, &[EBX, ECX, EAX, EDX, EDI], ESI);
            // PUSH DI; PUSH SI; PUSH AX; PUSH DX; MOV DI,[1f31]; MOV SI,[1f33]
            pending = vec![(x.di(), x.si()), (x.ax(), x.dx())];
            x.set(EDI, row);
            x.set(ESI, list);
            x.steps += 6;
        } else {
            // PUSH AX; PUSH DX
            x.steps += 2;
        }
        // Each pair, then POP CX; POP BX; JMP to the next pair.
        while let Some((b, c)) = pending.pop() {
            crossing_pair(x, clip);
            x.set(ECX, c);
            x.set(EBX, b);
            x.steps += 3;
        }
    }
    crossing_pair(x, clip);
}

/// The `EL2MATH3` work area words that [`fill_masks`] saves into the frame and
/// [`fill_masks_args`] replaces (`DS:07bc`, `07c0`, … `07d8`; docs/MATH3.md).
const WORK: [u16; 8] = [0x7bc, 0x7c0, 0x7c4, 0x7c8, 0x7cc, 0x7d0, 0x7d4, 0x7d8];

/// Port of `EL2MCGA3:3343` (slot `DS:01f8`): [`fill_masks`] with the 8 words the caller pushed
/// above its return address taken as the new work area words (the first pushed is `DS:07bc`);
/// the old ones are saved into the frame at `BP+34h`… first. Frees the pushed words as well.
pub fn fill_masks_args(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // POP BX; POP CX; MOV BP,SP; save the work area at [BP+34h]…
    let (ip, cs) = (x.pop(), x.pop());
    x.set(EBX, ip);
    x.set(ECX, cs);
    let bp = x.sp();
    let ss = x.c.ss;
    for (i, &w) in WORK.iter().enumerate() {
        let v = x.ds_rw(w);
        x.bus.write16(lin(ss, bp.wrapping_add(0x34 + 4 * i as u16)), v);
    }
    x.steps += 3 + 16;
    // POP [07d8] … POP [07bc]; PUSH CX; PUSH BX
    for &w in WORK.iter().rev() {
        let v = x.pop();
        x.ds_ww(w, v);
    }
    x.push(cs);
    x.push(ip);
    x.steps += 8 + 2;
    fill_masks(x)
}

/// Port of `EL2MCGA3:3399` (slot `DS:01f4`): fill the rows of a model rendered into the caller's
/// stack frame (as `EL2MATH3`'s `frame_5598` gives it), then free the frame (`RETF 5598h`).
/// From `BP` = `SP`: `[BP+6]` the number of rows; per row y a list at `BP+4ah+80h·y` of a byte
/// size and (x, polygon mask) word pairs, sorted by x; the masks of the rows are at
/// `BP+544ah+2y`. Rows are filled from the bottom up. Along a row the masks of the crossings
/// passed so far are XOR-ed together (starting from the masks of the rows below), and each
/// stretch between crossings is filled, clamped to 0 ≤ x < 320, with the colour byte the
/// combined mask selects in the frame (`[BP+mask+8]`). The work area words `DS:07bc`… are
/// saved into the frame at `BP+8`… first; `DS:07bc` and `07c8` are used as counters.
pub fn fill_masks(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    let bp = x.sp();
    x.set(EBP, bp);
    let ss = x.c.ss;
    let rw = |x: &mut Ctx, off: u16| x.bus.read16(lin(ss, off));
    let ww = |x: &mut Ctx, off: u16, v: u16| x.bus.write16(lin(ss, off), v);
    // 3399: MOV BP,SP; save the work area at [BP+8]…
    for (i, &w) in WORK.iter().enumerate() {
        let v = x.ds_rw(w);
        ww(x, bp.wrapping_add(8 + 4 * i as u16), v);
        x.set(EAX, v);
    }
    // MOV SI,[BP+6]; DEC SI; JS 3486
    let rows = rw(x, bp.wrapping_add(6)).wrapping_sub(1);
    x.set(ESI, rows);
    x.steps += 17 + 3;
    if (rows as i16) >= 0 {
        // MOV [07bc],SI; SHL SI,7; LEA BX,[BP+SI+4ah]; LEA SI,[BP+544ah]; MOV AX,[SI+14eh];
        // MOV [BP+48h],AX; MOV DI,[07bc]; ADD SI,DI (twice); ADD SI,2; CMP DI,0a7h; JL
        x.ds_ww(0x7bc, rows);
        let mut bx = bp.wrapping_add(rows << 7).wrapping_add(0x4a);
        let masks = bp.wrapping_add(0x544a);
        let below = rw(x, masks.wrapping_add(0x14e));
        let acc = bp.wrapping_add(0x48);
        ww(x, acc, below);
        let mut si = masks.wrapping_add(rows << 1).wrapping_add(2);
        x.set(EAX, below);
        x.steps += 12;
        if (rows as i16) >= 0xa7 {
            ww(x, acc, 0);
            x.steps += 1;
        }
        // MOVZX EDI,DI; MOV ES,[09e2]
        x.c.r[EDI] = rows as u32;
        x.c.es = x.ds_rw(TARGET);
        x.steps += 2;
        loop {
            // 3407: SUB SI,2; PUSH SI; MOV SI,[SI]; MOV AX,[BP+48h]; XOR [BP+48h],SI;
            // MOV DX,[BX]; OR DX,DX; JE 347a
            si = si.wrapping_sub(2);
            let list = si;
            // The pushes are written: a colour index below the frame reads them back.
            ww(x, bp.wrapping_sub(2), list);
            let mut mask = rw(x, si);
            let prev = rw(x, acc);
            ww(x, acc, prev ^ mask);
            let mut ax = prev;
            let size = rw(x, bx);
            let mut dx = size;
            x.set(EDX, dx);
            x.set(EAX, ax);
            x.steps += 8;
            if size != 0 {
                // SHR DX,2; PUSH BX
                dx >>= 2;
                let row = bx;
                ww(x, bp.wrapping_sub(4), row);
                x.steps += 2;
                // 341f: skip to the first entry with a zero mask, folding the masks into SI
                let mut found = false;
                loop {
                    // ADD BX,2; XOR SI,AX; MOV AX,[BX]; ADD BX,2; OR AX,AX; JE 3433
                    bx = bx.wrapping_add(2);
                    mask ^= ax;
                    ax = rw(x, bx);
                    bx = bx.wrapping_add(2);
                    x.steps += 6;
                    if ax == 0 {
                        found = true;
                        break;
                    }
                    // DEC DX; JNE 341f; JMP 3479
                    dx = dx.wrapping_sub(1);
                    x.steps += 2;
                    if dx == 0 {
                        x.steps += 1;
                        break;
                    }
                }
                x.set(EDX, dx);
                if found {
                    // 3433: MOV [07c8],DX
                    x.ds_ww(0x7c8, dx);
                    x.steps += 1;
                    loop {
                        // 3437: MOV AX,[BX]; ADD BX,2; OR AX,AX; JNS; XOR AX,AX; CMP AX,140h; JGE
                        ax = rw(x, bx);
                        bx = bx.wrapping_add(2);
                        x.steps += 4;
                        if (ax as i16) < 0 {
                            ax = 0;
                            x.steps += 1;
                        }
                        x.set(EAX, ax);
                        x.steps += 2;
                        if (ax as i16) >= WIDTH as i16 {
                            break;
                        }
                        // MOV CX,[BX+2]; OR CX,CX; JS 3467
                        let mut cx = rw(x, bx.wrapping_add(2));
                        x.set(ECX, cx);
                        x.steps += 3;
                        if (cx as i16) >= 0 {
                            // CMP CX,140h; JL; MOV CX,140h; SUB CX,AX; PUSH BX; PUSH DI;
                            // MOV BX,AX; MOV AH,[BP+SI+8]; CALL 1a99; POP DI; POP BX
                            x.steps += 2;
                            if (cx as i16) >= WIDTH as i16 {
                                cx = WIDTH;
                                x.steps += 1;
                            }
                            x.set(ECX, cx.wrapping_sub(ax));
                            ww(x, bp.wrapping_sub(6), bx);
                            ww(x, bp.wrapping_sub(8), x.di());
                            x.set(EBX, ax);
                            x.set(ESI, mask);
                            let colour = x.bus.read8(lin(ss, bp.wrapping_add(mask).wrapping_add(8)));
                            x.c.set_h(EAX, colour);
                            let di = x.di();
                            x.steps += 6;
                            ww(x, bp.wrapping_sub(10), 0x3465);
                            span(x, SpanEntry::Unclipped);
                            x.set(EDI, di);
                            x.steps += 2;
                        }
                        // 3467: MOV AX,[BX]; ADD BX,2; OR AX,AX; JE 3479; XOR SI,AX; DEC [07c8]; JNE
                        ax = rw(x, bx);
                        bx = bx.wrapping_add(2);
                        x.set(EAX, ax);
                        x.steps += 4;
                        if ax == 0 {
                            break;
                        }
                        mask ^= ax;
                        let n = x.ds_rw(0x7c8).wrapping_sub(1);
                        x.ds_ww(0x7c8, n);
                        x.steps += 3;
                        if n == 0 {
                            break;
                        }
                    }
                }
                x.set(EAX, ax);
                x.set(ESI, mask);
                // 3479: POP BX
                bx = row;
                x.steps += 1;
            }
            // 347a: POP SI; SUB BX,80h; DEC DI; DEC [07bc]; JNS 3407
            si = list;
            bx = bx.wrapping_sub(0x80);
            x.set(EDI, x.di().wrapping_sub(1));
            let n = x.ds_rw(0x7bc).wrapping_sub(1);
            x.ds_ww(0x7bc, n);
            x.steps += 5;
            if (n as i16) < 0 {
                break;
            }
        }
        x.set(EBX, bx);
        x.set(ESI, si);
    }
    // RETF 5598h
    x.retf();
    x.set(ESP, x.sp().wrapping_add(0x5598));
    Ok(())
}
