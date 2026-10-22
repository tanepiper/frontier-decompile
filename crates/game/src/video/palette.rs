//! The palette: the colour allocator that maps 12-bit colours to DAC entries, and the DAC
//! uploads (docs/VIDEO.md §5).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.
//! Port accesses go through [`Io`] with the instruction count before each `IN` or `OUT`, so the
//! retrace waits end where the original's would.

use super::*;

/// The allocator's tables in the main data segment (names *inferred* from use): per DAC entry
/// 0–254 a dword at `0a22`+4·n of (state, entry, 12-bit colour); per 12-bit colour the entry it
/// has (FFh = none) at `0f21`; the list of free entries at `0e1e`, and its read pointer at
/// `0f1f`. States: 1 = to be freed, 2 = new, 3 = free, 4 = in use, 5–6 = recently used.
pub const SLOTS: u16 = 0x0a22;
pub const COLOUR_MAP: u16 = 0x0f21;
pub const FREE_LIST: u16 = 0x0e1e;
pub const FREE_NEXT: u16 = 0x0f1f;

fn slot(n: u32) -> u32 {
    SLOTS as u32 + 4 * n
}

/// Port of `EL2MCGA3:59ce` (slot `DS:0238`): the DAC entry for 12-bit colour `BX` (0RGBh) in
/// `BX`. A known colour is marked recently used; a new one takes the next free entry (state
/// 2, uploaded by [`upload_colours`]); with none free, the entry of colour 0.
pub fn colour(x: &mut Ctx) -> Result<(), Decline> {
    // AND BH,0fh; XOR EAX,EAX; MOV AL,[BX+0f21]; CMP AL,0ffh; JE 59f3
    let key = x.bx() & 0x0fff;
    x.set(EBX, key);
    x.c.r[EAX] = 0;
    let ds = x.c.ds;
    let e = x.ds_rb(COLOUR_MAP.wrapping_add(key));
    x.c.r[EAX] = e as u32;
    x.steps += 5;
    if e != 0xff {
        // CMP [EAX*4+0a22],4; JB; MOV [..],6; MOV BX,AX; RETF
        let a = lin32(ds, slot(e as u32));
        x.steps += 2;
        if x.bus.read8(a) >= 4 {
            x.bus.write8(a, 6);
            x.steps += 1;
        }
        x.set(EBX, e as u16);
        x.steps += 1;
        x.retf();
        return Ok(());
    }
    // 59f3: MOV AX,[0f1f]; MOVZX AX,[EAX]; CMP AL,0ffh; JE 5a1a
    let next = x.ds_rw(FREE_NEXT);
    let f = x.bus.read8(lin32(ds, next as u32));
    x.c.r[EAX] = f as u32;
    x.steps += 4;
    if f == 0xff {
        // MOVZX BX,[0f21]; RETF
        let b = x.ds_rb(COLOUR_MAP);
        x.set(EBX, b as u16);
        x.steps += 1;
        x.retf();
        return Ok(());
    }
    // INC [0f1f]; MOV [BX+0f21],AL; MOV [EAX*4+0a22],2; MOV [EAX*4+0a24],BX; MOV BX,AX; RETF
    x.ds_ww(FREE_NEXT, next.wrapping_add(1));
    x.ds_wb(COLOUR_MAP.wrapping_add(key), f);
    x.bus.write8(lin32(ds, slot(f as u32)), 2);
    x.bus.write16(lin32(ds, slot(f as u32) + 2), key);
    x.set(EBX, f as u16);
    x.steps += 5;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:5a20` (slot `DS:0200`): the DAC entry of allocator slot `DI` in `DI`.
pub fn colour_entry(x: &mut Ctx) -> Result<(), Decline> {
    // SHL DI,2; MOVZX DI,[DI+0a23]; RETF
    let at = (x.di() << 2).wrapping_add(SLOTS + 1);
    let v = x.ds_rb(at);
    x.set(EDI, v as u16);
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:5a29` (slot `DS:023c`): with `AL` ≠ 0 reset the allocator first (every
/// colour unknown, every entry free, the greys 000h, 111h, … FFFh held by
/// slots 0Fh … 0, the slots' entries 10h–1Fh). Then age the entries: recently used ones count down to "in use", those
/// to be freed become free, and the free entries are listed again.
pub fn colours_tick(x: &mut Ctx) -> Result<(), Decline> {
    // OR AL,AL; JNE 5a7b
    x.steps += 2;
    let ds = x.c.ds;
    if x.c.l(EAX) != 0 {
        x.forward()?;
        // 5a7b: MOV DI,0f21h; MOV AX,DS; MOV ES,AX; MOV EAX,-1; MOV CX,400h; REP STOSD;
        // MOV DI,0a22h; MOV EAX,3; MOV CX,0ffh; REP STOSD
        x.c.es = ds;
        for i in 0..0x1000u16 {
            x.ds_wb(COLOUR_MAP.wrapping_add(i), 0xff);
        }
        for n in 0..0xff {
            x.bus.write32(lin32(ds, slot(n)), 3);
        }
        x.c.r[EAX] = 3;
        x.set(ECX, 0);
        x.steps += 10;
        // The greys: MOV [0f21+111h·k],k and MOV [0a22+4k],(10h+k)·100h for k = 0…15
        for k in 0..16u16 {
            x.ds_wb(COLOUR_MAP + 0x111 * k, 15 - k as u8);
            x.ds_ww(SLOTS + 4 * k, (0x10 + k) << 8);
        }
        // JMP 5a2d
        x.steps += 33;
    }
    // 5a2d: MOV DI,0e1eh; XOR EDX,EDX
    let mut di = FREE_LIST;
    x.c.r[EDX] = 0;
    x.steps += 2;
    for n in 0..0xffu32 {
        // MOV AL,[EDX*4+0a22]; CMP AL,3; JE 5a67; CMP AL,1; JE 5a51; CMP AL,4; JBE 5a6a;
        // DEC [EDX*4+0a22]; JMP 5a6a
        let a = lin32(ds, slot(n));
        let state = x.bus.read8(a);
        x.c.set_l(EAX, state);
        x.steps += 3;
        let mut free = state == 3;
        if !free {
            x.steps += 2;
            if state == 1 {
                // 5a51: MOV BX,[EDX*4+0a24]; MOV [BX+0f21],0ffh; MOV [EDX*4+0a22],3
                let key = x.bus.read16(a + 2);
                x.set(EBX, key);
                x.ds_wb(COLOUR_MAP.wrapping_add(key), 0xff);
                x.bus.write8(a, 3);
                x.steps += 3;
                free = true;
            } else {
                x.steps += 2;
                if state > 4 {
                    x.bus.write8(a, state - 1);
                    x.steps += 2;
                }
            }
        }
        if free {
            // 5a67: MOV [DI],DL; INC DI
            x.ds_wb(di, n as u8);
            di = di.wrapping_add(1);
            x.steps += 2;
        }
        // 5a6a: INC DL; CMP DL,0ffh; JB 5a33
        x.steps += 3;
    }
    x.c.set_l(EDX, 0xff);
    // MOV [DI],0ffh; MOV [0f1f],0e1eh; RETF
    x.ds_wb(di, 0xff);
    x.set(EDI, di);
    x.ds_ww(FREE_NEXT, FREE_LIST);
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Wait on the retrace bit of port 3DAh: `IN AL,DX; TEST AL,8; JNZ` while set (`set` false) or
/// `JZ` while clear (`set` true).
fn wait_retrace(x: &mut Ctx, set: bool) {
    loop {
        let v = x.io.inb(x.steps, 0x3da);
        x.c.set_l(EAX, v);
        x.steps += 3;
        if (v & 8 != 0) == set {
            break;
        }
    }
}

fn out(x: &mut Ctx, port: u16, v: u8) {
    x.io.outb(x.steps, port, v);
    x.steps += 1;
}

/// Port of `EL2MCGA3:5b50` (slot `DS:0234`): give the new colours DAC entries and upload them.
/// Entries 80h–FFh not held by an in-use slot are free; a new colour takes the next free one
/// (freeing the slot that held it), else the nearest known colour found by stepping its red,
/// then green, then blue down. After the next vertical retrace, the new entries are written to
/// the DAC with interrupts off.
pub fn upload_colours(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // SUB SP,200h; MOV DI,SP; MOV AX,SS; MOV ES,AX; XOR EAX,EAX; MOV CX,40h; REP STOSD
    let sp = x.sp().wrapping_sub(0x200);
    x.set(ESP, sp);
    let ss = x.c.ss;
    x.c.es = ss;
    for i in 0..0x100u16 {
        x.bus.write8(lin(ss, sp.wrapping_add(i)), 0);
    }
    x.c.r[EAX] = 0;
    x.set(EDI, sp.wrapping_add(0x100));
    x.set(ECX, 0);
    x.steps += 7;
    let ds = x.c.ds;
    let esp = x.c.r[ESP];
    let table = |e: u32| lin32(ss, esp.wrapping_add(2 * e));
    // XOR ECX,ECX; per slot: CMP [ECX*4+0a22],4; JB 5b8f; MOV AL,[ECX*4+0a23]; MOV DL,1; JZ;
    // INC DL; MOV [ESP+EAX*2-100h],DL; MOV [ESP+EAX*2-0ffh],CL; INC CL; CMP CL,0ffh; JB
    x.c.r[ECX] = 0;
    x.steps += 1;
    for n in 0..0xffu32 {
        let state = x.bus.read8(lin32(ds, slot(n)));
        x.steps += 2;
        if state >= 4 {
            let e = x.bus.read8(lin32(ds, slot(n) + 1)) as u32;
            x.c.set_l(EAX, e as u8);
            let mark = if state == 4 { 1 } else { 2 };
            x.c.set_l(EDX, mark);
            x.steps += 3 + (mark == 2) as u64 + 2;
            x.bus.write8(table(e).wrapping_sub(0x100), mark);
            x.bus.write8(table(e).wrapping_sub(0xff), n as u8);
        }
        x.steps += 3;
    }
    x.c.set_l(ECX, 0xff);
    // MOV DI,0e1eh; XOR EAX,EAX; the unmarked entries 80h–FFh, then those in use (mark 1) with
    // their slot: per entry CMP [ESP+EAX*2],m; JNZ; MOV DL,AL; ADD DL,80h; MOV [DI],DL; …;
    // ADD DI,2; INC AL; CMP AL,80h; JB
    let mut di = FREE_LIST;
    x.c.r[EAX] = 0;
    x.steps += 2;
    for e in 0..0x80u32 {
        x.steps += 2;
        if x.bus.read8(table(e)) == 0 {
            x.ds_wb(di, 0x80 + e as u8);
            x.ds_wb(di.wrapping_add(1), 0xff);
            di = di.wrapping_add(2);
            x.c.set_l(EDX, 0x80 + e as u8);
            x.steps += 5;
        }
        x.steps += 3;
    }
    x.steps += 1; // XOR AX,AX
    for e in 0..0x80u32 {
        x.steps += 2;
        if x.bus.read8(table(e)) == 1 {
            let s = x.bus.read8(table(e) + 1);
            x.ds_wb(di, 0x80 + e as u8);
            x.ds_wb(di.wrapping_add(1), s);
            di = di.wrapping_add(2);
            x.c.set_l(EDX, s);
            x.steps += 6;
        }
        x.steps += 3;
    }
    x.c.set_l(EAX, 0x80);
    // MOV [DI],0; MOV [0f1f],0e1eh; MOV BP,SP; XOR EDX,EDX
    x.ds_wb(di, 0);
    x.set(EDI, di);
    x.ds_ww(FREE_NEXT, FREE_LIST);
    let mut bp = sp;
    x.c.r[EDX] = 0;
    x.steps += 4;
    for n in 0..0xffu32 {
        // 5be6: CMP [EDX*4+0a22],2; JNZ 5d06
        let a = lin32(ds, slot(n));
        x.steps += 2;
        if x.bus.read8(a) == 2 {
            // MOV DI,[0f1f]; MOV AL,[DI]; OR AL,AL; JZ 5c62
            let next = x.ds_rw(FREE_NEXT);
            x.set(EDI, next);
            let e = x.ds_rb(next);
            x.c.set_l(EAX, e);
            x.steps += 4;
            if e != 0 {
                // ADD [0f1f],2; XOR ECX,ECX; MOV CL,[DI+1]; CMP CL,0ffh; JZ 5c26
                x.ds_ww(FREE_NEXT, next.wrapping_add(2));
                let held = x.ds_rb(next.wrapping_add(1));
                x.c.r[ECX] = held as u32;
                x.steps += 5;
                if held != 0xff {
                    // MOV [ECX*4+0a22],3; MOV CX,[ECX*4+0a24]; MOV [ECX+0f21],0ffh
                    let b = lin32(ds, slot(held as u32));
                    x.bus.write8(b, 3);
                    let key = x.bus.read16(b + 2);
                    x.set(ECX, key);
                    x.bus.write8(lin32(ds, COLOUR_MAP as u32 + x.c.r[ECX]), 0xff);
                    x.steps += 3;
                }
                // 5c26: MOV [EDX*4+0a22],6; MOV [EDX*4+0a23],AL; MOV [BP],AL;
                // MOV AX,[EDX*4+0a24]; SHL AH,2; ADD AH,3; MOV [BP+1],AH; MOV AH,AL;
                // AND AX,0ff0h; SHR AL,2; SHL AH,2; ADD AX,303h; MOV [BP+2],AX; ADD BP,4; JMP
                x.bus.write8(a, 6);
                x.bus.write8(a + 1, e);
                x.bus.write8(lin(ss, bp), e);
                let key = x.bus.read16(a + 2);
                let r = ((key >> 8) as u8).wrapping_shl(2).wrapping_add(3);
                x.bus.write8(lin(ss, bp.wrapping_add(1)), r);
                let gb = key as u8 as u16;
                let ax = ((gb << 8 | gb) & 0x0ff0) as u16;
                let ax = ((((ax >> 8) as u8).wrapping_shl(2) as u16) << 8 | ((ax as u8) >> 2) as u16).wrapping_add(0x303);
                x.bus.write16(lin(ss, bp.wrapping_add(2)), ax);
                x.set(EAX, ax);
                bp = bp.wrapping_add(4);
                x.steps += 15;
            } else {
                // 5c62: XOR ECX,ECX; MOV CX,[EDX*4+0a24]; XOR EAX,EAX; then the search
                let mut c = x.bus.read16(a + 2);
                x.c.r[EAX] = 0;
                x.steps += 3;
                let mut found;
                let mut guard = 0u32;
                // 5c70: the red, 5c92: the green, 5cb6: the blue, each stepped down while it
                // can be, a colour taken if it is known and not new.
                loop {
                    guard += 1;
                    if guard > 100_000 {
                        return Err(Decline("no known colour below a new one (the original loops for ever)"));
                    }
                    let probe = |x: &mut Ctx, c: u16| -> Option<u8> {
                        let e = x.bus.read8(lin32(ds, COLOUR_MAP as u32 + c as u32));
                        x.steps += 3;
                        if e == 0xff {
                            return None;
                        }
                        x.c.set_l(EAX, e);
                        x.steps += 3;
                        Some(e)
                    };
                    // OR CH,CH; JZ 5c92; DEC CH; CMP [ECX+0f21],0ffh; JZ 5c92; MOV AL,..;
                    // CMP [EAX*4+0a22],2; JNZ 5cd9
                    x.steps += 2;
                    if c >> 8 != 0 {
                        c = c.wrapping_sub(0x100);
                        if let Some(e) = probe(x, c) {
                            if x.bus.read8(lin32(ds, slot(e as u32))) != 2 {
                                found = e;
                                break;
                            }
                        }
                    }
                    // 5c92: TEST CL,0f0h; JZ 5cb6; SUB CL,10h; …
                    x.steps += 2;
                    if c & 0xf0 != 0 {
                        c = c.wrapping_sub(0x10);
                        if let Some(e) = probe(x, c) {
                            if x.bus.read8(lin32(ds, slot(e as u32))) != 2 {
                                found = e;
                                break;
                            }
                        }
                    }
                    // 5cb6: TEST CL,0fh; JZ 5c70; DEC CL; …; JZ 5c70
                    x.steps += 2;
                    if c & 0xf != 0 {
                        c = c.wrapping_sub(1);
                        if let Some(e) = probe(x, c) {
                            if x.bus.read8(lin32(ds, slot(e as u32))) != 2 {
                                found = e;
                                break;
                            }
                        }
                    }
                }
                x.c.r[ECX] = c as u32;
                // 5cd9: CMP [EAX*4+0a22],4; JB; MOV [..],6; MOV AL,[EAX*4+0a23];
                // MOV [EDX*4+0a23],AL; MOV [EDX*4+0a22],1
                let b = lin32(ds, slot(found as u32));
                x.steps += 2;
                if x.bus.read8(b) >= 4 {
                    x.bus.write8(b, 6);
                    x.steps += 1;
                }
                let e = x.bus.read8(b + 1);
                x.c.set_l(EAX, e);
                x.bus.write8(a + 1, e);
                x.bus.write8(a, 1);
                x.steps += 3;
                found = e;
                let _ = found;
            }
        }
        // 5d06: INC DL; CMP DL,0ffh; JB 5be6
        x.steps += 3;
    }
    x.c.set_l(EDX, 0xff);
    // MOV CX,BP; SUB CX,SP; SHR CX,2; JZ 5d45
    let n = bp.wrapping_sub(sp) >> 2;
    x.set(ECX, n);
    x.set(EBP, bp);
    x.steps += 4;
    if n != 0 {
        // MOV BP,SP; MOV DX,3dah; wait for the retrace to end, then to start; CLI;
        // MOV DX,3c8h; per entry: MOV AX,[BP]; OUT DX,AL; INC DL; MOV AL,AH; OUT DX,AL;
        // MOV AX,[BP+2]; OUT DX,AL; MOV AL,AH; OUT DX,AL; ADD BP,4; DEC DL; DEC CL; JNZ; STI
        x.set(EDX, 0x3da);
        x.steps += 2;
        wait_retrace(x, false);
        wait_retrace(x, true);
        x.c.flags &= !IF;
        x.steps += 2;
        let mut p = sp;
        let mut cl = n as u8;
        loop {
            let w0 = x.bus.read16(lin(ss, p));
            x.steps += 1;
            out(x, 0x3c8, w0 as u8);
            x.steps += 2;
            out(x, 0x3c9, (w0 >> 8) as u8);
            let w1 = x.bus.read16(lin(ss, p.wrapping_add(2)));
            x.steps += 1;
            out(x, 0x3c9, w1 as u8);
            x.steps += 1;
            out(x, 0x3c9, (w1 >> 8) as u8);
            x.set(EAX, (w1 & 0xff00) | (w1 >> 8));
            p = p.wrapping_add(4);
            cl = cl.wrapping_sub(1);
            x.steps += 4;
            if cl == 0 {
                break;
            }
        }
        x.set(EDX, 0x3c8);
        x.c.set_l(ECX, 0);
        x.set(EBP, p);
        x.c.flags |= IF;
        x.steps += 1;
    }
    // ADD SP,200h; RETF
    x.set(ESP, sp.wrapping_add(0x200));
    x.steps += 1;
    x.retf();
    Ok(())
}

/// Upload `words` words from `seg:si` + `from` to the DAC data port (`MOV AX,[SI+n]; OUT DX,AL;
/// MOV AL,AH; OUT DX,AL` each).
fn out_words(x: &mut Ctx, seg: u16, si: u16, from: u16, words: u16) {
    for i in 0..words {
        let w = x.bus.read16(lin(seg, si.wrapping_add(from + 2 * i)));
        x.steps += 1;
        out(x, 0x3c9, w as u8);
        x.steps += 1;
        out(x, 0x3c9, (w >> 8) as u8);
        x.set(EAX, (w & 0xff00) | (w >> 8));
    }
}

/// `STI; MOV DX,3dah; (wait for the retrace); CLI; MOV DX,3c9h` between the chunks of the
/// full uploads.
fn next_chunk(x: &mut Ctx) {
    x.c.flags |= IF;
    x.set(EDX, 0x3da);
    x.steps += 2;
    wait_retrace(x, true);
    x.c.flags &= !IF;
    x.set(EDX, 0x3c9);
    x.steps += 2;
}

/// The start of a DAC upload: `MOV DX,3dah`, wait for the retrace to end and start again,
/// `CLI`, then the first entry to the index port (3C8h) and `INC DX`.
fn start_upload(x: &mut Ctx, first: u8, load: u64) {
    x.set(EDX, 0x3da);
    x.steps += 1;
    wait_retrace(x, false);
    wait_retrace(x, true);
    x.c.flags &= !IF;
    x.c.set_l(EAX, first);
    x.steps += 1 + load + 1;
    out(x, 0x3c8, first);
    x.set(EDX, 0x3c9);
    x.steps += 1;
}

/// Port of `EL2MCGA3:0811` (slot `DS:024c`): set palette `AL` (`DS:891a`): reset the colour
/// allocator (`CALLF [023c]` with `AL` = 1), then [`palette_after_reset`].
pub fn set_palette(x: &mut Ctx) -> Result<(), Decline> {
    // MOV [891a],AL; XOR AH,AH; PUSH AX; MOV AL,1; CALLF [023c]
    let p = x.c.l(EAX);
    x.ds_wb(0x891a, p);
    x.push(p as u16);
    x.set(EAX, 1);
    x.steps += 4;
    x.call_slot(0x023c, 0x081d);
    Ok(())
}

/// Port of `EL2MCGA3:081d`: upload the 256 colours of palette `[SP]` (pointer at font offset
/// 5 + 2·n) in four chunks of 64 colours, each after a vertical retrace.
pub fn palette_after_reset(x: &mut Ctx) -> Result<(), Decline> {
    // POP AX; SHL AX,1; MOV SI,AX; PUSH DS; MOV AX,05deh; MOV DS,AX; MOV SI,[SI+5]
    let n = x.pop();
    let ds = x.c.ds;
    x.push(ds);
    let font = x.c.cs.wrapping_add(FONT_PARAS);
    let si = x.bus.read16(lin(font, (n << 1).wrapping_add(5)));
    x.set(ESI, si);
    x.set(EAX, font);
    x.steps += 7;
    start_upload(x, 0, 1);
    out_words(x, font, si, 0, 96);
    for from in [0xc0, 0x180, 0x240] {
        next_chunk(x);
        out_words(x, font, si, from, 96);
    }
    // STI; POP DS; RETF
    x.c.flags |= IF;
    x.c.ds = x.pop();
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:142a` (slot `DS:0248`): reset the colour allocator, then
/// [`high_after_reset`].
pub fn load_high_colours(x: &mut Ctx) -> Result<(), Decline> {
    // PUSHAD; MOV AL,1; CALLF [023c]
    pushad(x);
    x.c.set_l(EAX, 1);
    x.steps += 2;
    x.call_slot(0x023c, 0x1432);
    Ok(())
}

/// `PUSHAD`: the eight registers, `ESP` as it was before.
fn pushad(x: &mut Ctx) {
    let sp = x.c.r[ESP];
    for r in [EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI] {
        let v = if r == ESP { sp } else { x.c.r[r] };
        x.push32(v);
    }
}

/// `POPAD`: the registers back, `ESP` skipped.
fn popad(x: &mut Ctx) {
    for r in [EDI, ESI, EBP, ESP, EBX, EDX, ECX, EAX] {
        let v = x.pop32();
        if r != ESP {
            x.c.r[r] = v;
        }
    }
}

/// Port of `EL2MCGA3:1432`: upload colours 80h–FFh from `AX:SI` (384 bytes) in two chunks.
pub fn high_after_reset(x: &mut Ctx) -> Result<(), Decline> {
    // POPAD; PUSH DS; MOV DS,AX
    popad(x);
    let ds = x.c.ds;
    x.push(ds);
    let (seg, si) = (x.ax(), x.si());
    x.steps += 3;
    start_upload(x, 0x80, 1);
    out_words(x, seg, si, 0, 96);
    next_chunk(x);
    out_words(x, seg, si, 0xc0, 96);
    // STI; POP DS; RETF
    x.c.flags |= IF;
    x.c.ds = x.pop();
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:34e7` (slot `DS:0250`): reset the colour allocator, then
/// [`colours_after_reset`].
pub fn load_colours(x: &mut Ctx) -> Result<(), Decline> {
    // PUSHAD; PUSH ES; MOV AL,1; CALLF [023c]
    pushad(x);
    let es = x.c.es;
    x.push(es);
    x.c.set_l(EAX, 1);
    x.steps += 3;
    x.call_slot(0x023c, 0x34f0);
    Ok(())
}

/// Port of `EL2MCGA3:34f0`: upload `CL` colours (0 = 256) from `AX:SI`, 3 bytes each, starting
/// at entry `BL`, after a vertical retrace.
pub fn colours_after_reset(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // POP ES; POPAD; PUSH DS; MOV DS,AX
    x.c.es = x.pop();
    popad(x);
    let ds = x.c.ds;
    x.push(ds);
    let seg = x.ax();
    let mut si = x.si();
    x.steps += 4;
    let first = x.c.l(EBX);
    start_upload(x, first, 1);
    let mut cl = x.c.l(ECX);
    loop {
        // LODSW; OUT DX,AL; MOV AL,AH; OUT DX,AL; LODSB; OUT DX,AL; DEC CL; JNZ
        let w = x.bus.read16(lin(seg, si));
        let b = x.bus.read8(lin(seg, si.wrapping_add(2)));
        si = si.wrapping_add(3);
        x.steps += 1;
        out(x, 0x3c9, w as u8);
        x.steps += 1;
        out(x, 0x3c9, (w >> 8) as u8);
        x.steps += 1;
        out(x, 0x3c9, b);
        x.set(EAX, (w & 0xff00) | b as u16);
        cl = cl.wrapping_sub(1);
        x.steps += 2;
        if cl == 0 {
            break;
        }
    }
    x.c.set_l(ECX, 0);
    x.set(ESI, si);
    // STI; POP DS; RETF
    x.c.flags |= IF;
    x.c.ds = x.pop();
    x.steps += 2;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:5d4a` (slot `DS:0244`): nothing (`RETF`).
pub fn nothing(x: &mut Ctx) -> Result<(), Decline> {
    x.retf();
    Ok(())
}
