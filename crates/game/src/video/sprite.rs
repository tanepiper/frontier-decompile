//! Bitmaps from `EL2G.GRF`: the cache in conventional memory, filled from EMS, and the blitter
//! (docs/VIDEO.md §5).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.
//! The cache's miss path calls the EMS driver (`int 67h`) three times; the port ends at each
//! `INT`, and the code after it is its own entry ([`cache_after_save`], [`cache_after_map`],
//! [`cache_after_restore`]), as is the code after each call of the cache ([`blit_after_lookup`]).

use super::*;

/// Main data segment variables of the cache (names *inferred* from use).
/// Segment of the cache, allocated at start-up (`EL2MCGA3:05ca`, 0F39h paragraphs).
pub const CACHE_SEGMENT: u16 = 0x1fa7;
/// Per bitmap: its EMS offset and logical page (dwords from `DS:5813`).
pub const BITMAP_DIRECTORY: u16 = 0x5813;
/// The EMS handle and the page frame segment.
pub const EMS_HANDLE: u16 = 0x0a18;
pub const EMS_FRAME: u16 = 0x0a1a;
/// Depth counters (incremented around the cache and the blitter).
pub const CACHE_BUSY: u16 = 0x0a20;
pub const BLIT_BUSY: u16 = 0x1fa9;

/// Offsets in the cache segment: the slot of each bitmap (FFFFh = not cached) from F000h, the
/// ring of cached bitmap numbers F1C2h–F383h, and the allocator's words.
pub const CACHE_SLOTS: u32 = 0xf000;
pub const RING_START: u16 = 0xf1c2;
pub const RING_END: u16 = 0xf384;
pub const CACHE_LIMIT: u16 = 0xf384;
pub const CACHE_FREE: u16 = 0xf386;
pub const RING_HEAD: u16 = 0xf388;
pub const RING_TAIL: u16 = 0xf38a;
pub const CACHE_COUNT: u16 = 0xf38c;
pub const CACHE_HIGH: u16 = 0xf38e;

fn es_rw(x: &mut Ctx, off: u16) -> u16 {
    x.bus.read16(lin(x.c.es, off))
}
fn es_ww(x: &mut Ctx, off: u16, v: u16) {
    x.bus.write16(lin(x.c.es, off), v)
}
fn cs_rw(x: &mut Ctx, off: u16) -> u16 {
    x.bus.read16(lin(x.c.cs, off))
}
fn cs_ww(x: &mut Ctx, off: u16, v: u16) {
    x.bus.write16(lin(x.c.cs, off), v)
}

/// How the cache lookup at `EL2MCGA3:0173` ended.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// The bitmap is cached: `ES:SI` is its data, and the `RETF` was taken.
    Hit,
    /// Not cached: the port stopped at the `int 67h` that saves the EMS mapping.
    Miss,
}

/// The cache lookup `EL2MCGA3:0173` (reached by far call): bitmap `BX` in the cache segment
/// (`ES` = `DS:1fa7`), `SI` its offset if cached. Otherwise the registers are saved, the
/// bitmap's EMS page is mapped (`int 67h` 47h, then 44h) and it is copied into the cache
/// ([`cache_after_map`]).
pub fn cache_lookup(x: &mut Ctx) -> Lookup {
    // AND EBX,0ffffh; MOV ES,[1fa7]; MOV SI,ES:[EBX*2+0f000h]; CMP SI,-1; JE 18d; RETF
    x.c.r[EBX] &= 0xffff;
    x.c.es = x.ds_rw(CACHE_SEGMENT);
    let id = x.c.r[EBX];
    let si = x.bus.read16(lin32(x.c.es, CACHE_SLOTS + 2 * id));
    x.set(ESI, si);
    x.steps += 5;
    if si != 0xffff {
        x.retf();
        return Lookup::Hit;
    }
    // 18d: PUSH ECX; PUSH EDX; PUSH EDI; PUSH EBX; MOV DI,[EBX*4+5813h]; MOV BX,[EBX*4+5815h];
    // INC [0a20]; PUSH DI; PUSH ES; PUSH BX; MOV AH,47h; MOV DX,[0a18]; INT 67h
    for r in [ECX, EDX, EDI, EBX] {
        let v = x.c.r[r];
        x.push32(v);
    }
    let ds = x.c.ds;
    let di = x.bus.read16(lin32(ds, BITMAP_DIRECTORY as u32 + 4 * id));
    let page = x.bus.read16(lin32(ds, BITMAP_DIRECTORY as u32 + 2 + 4 * id));
    x.set(EDI, di);
    x.set(EBX, page);
    let busy = x.ds_rw(CACHE_BUSY);
    let n = busy.wrapping_add(1);
    x.ds_ww(CACHE_BUSY, n);
    let es = x.c.es;
    x.push(di);
    x.push(es);
    x.push(page);
    x.c.set_h(EAX, 0x47);
    let handle = x.ds_rw(EMS_HANDLE);
    x.set(EDX, handle);
    x.steps += 12;
    // The flags of CMP SI,-1 (equal) as INC [0a20] leaves them.
    let flags = (x.c.flags & !(CF | PF | AF | ZF | SF | OF))
        | szp16(n)
        | if n == 0x8000 { OF } else { 0 }
        | if (busy ^ 1 ^ n) & 0x10 != 0 { AF } else { 0 };
    x.int(0x67, 0x1b4, flags);
    Lookup::Miss
}

/// Port of the cache's code after its first `int 67h` (`EL2MCGA3:01b4`): `POP BX; MOV AX,4400h;
/// MOV DX,[0a18]; INT 67h` (map logical page `BX` into physical page 0).
pub fn cache_after_save(x: &mut Ctx) -> Result<(), Decline> {
    let bx = x.pop();
    x.set(EBX, bx);
    x.set(EAX, 0x4400);
    let handle = x.ds_rw(EMS_HANDLE);
    x.set(EDX, handle);
    x.steps += 3;
    let flags = x.c.flags;
    x.int(0x67, 0x1be, flags);
    Ok(())
}

/// Port of the cache's code after mapping the page (`EL2MCGA3:01be`): the bitmap (a size word
/// and its bytes) is copied from the page frame into the cache at the free offset, evicting
/// the oldest cached bitmaps (a ring of their numbers) until it fits below F000h; its number
/// joins the ring. Ends at the `int 67h` that restores the mapping.
pub fn cache_after_map(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // POP ES; POP SI; PUSH DS; MOV DS,[0a1a]; LODSW; ADD AX,2
    x.c.es = x.pop();
    let mut si = x.pop();
    let ds = x.c.ds;
    x.push(ds);
    let frame = x.ds_rw(EMS_FRAME);
    x.c.ds = frame;
    let size = x.bus.read16(lin(frame, si)).wrapping_add(2);
    si = si.wrapping_add(2);
    x.set(EAX, size);
    x.steps += 6;
    let di;
    loop {
        // 1c9: MOV EBX,[ESP+2]; CMP ES:[0f38c],0e1h; JAE 25c; MOV DI,ES:[0f386]; ADD DI,AX;
        // JB 25c; CMP DI,ES:[0f384]; JA 25c; CMP DI,0f000h; JA 25c
        let ebx = x.bus.read32(lin32(x.c.ss, x.c.r[ESP].wrapping_add(2)));
        x.c.r[EBX] = ebx;
        x.steps += 3;
        let fits = es_rw(x, CACHE_COUNT) < 0xe1 && {
            let free = es_rw(x, CACHE_FREE);
            let (end, carry) = free.overflowing_add(size);
            x.set(EDI, end);
            x.steps += 3;
            !carry && {
                x.steps += 2;
                end <= es_rw(x, CACHE_LIMIT) && {
                    x.steps += 2;
                    end <= 0xf000
                }
            }
        };
        if fits {
            // 1f0: XCHG ES:[0f386],DI; INC ES:[0f38c]; MOV BP,ES:[0f388]; MOV ES:[BP],BX;
            // ADD BP,2; CMP BP,0f384h; JB; MOV BP,0f1c2h; MOV ES:[0f388],BP
            let end = x.di();
            let start = es_rw(x, CACHE_FREE);
            es_ww(x, CACHE_FREE, end);
            x.set(EDI, start);
            let c = es_rw(x, CACHE_COUNT).wrapping_add(1);
            es_ww(x, CACHE_COUNT, c);
            let mut bp = es_rw(x, RING_HEAD);
            es_ww(x, bp, ebx as u16);
            bp = bp.wrapping_add(2);
            x.steps += 7;
            if bp >= RING_END {
                bp = RING_START;
                x.steps += 1;
            }
            es_ww(x, RING_HEAD, bp);
            x.set(EBP, bp);
            // 214: MOV CX,AX; ADD CX,DI; CMP CX,ES:[0f38e]; JB; MOV ES:[0f38e],CX
            let cx = size.wrapping_add(start);
            let high = es_rw(x, CACHE_HIGH);
            let _ = high;
            x.steps += 5;
            if cx >= high {
                es_ww(x, CACHE_HIGH, cx);
                x.steps += 1;
            }
            di = start;
            break;
        }
        // 25c: evict the oldest: PUSH AX; PUSH SI; MOV SI,ES:[0f38a]; MOVZX EBX,ES:[SI];
        // ADD SI,2; CMP SI,0f384h; JB; MOV SI,0f1c2h; MOV ES:[0f38a],SI; MOV SI,0ffffh;
        // XCHG ES:[EBX*2+0f000h],SI; SUB SI,2; DEC ES:[0f38c]; MOV AX,ES:[SI]; ADD AX,SI;
        // CMP AX,ES:[0f38e]; JNE 2a9
        x.steps += 1;
        let push_si = si;
        x.push(size);
        x.push(push_si);
        let mut tail = es_rw(x, RING_TAIL);
        let victim = es_rw(x, tail) as u32;
        x.c.r[EBX] = victim;
        tail = tail.wrapping_add(2);
        x.steps += 6;
        if tail >= RING_END {
            tail = RING_START;
            x.steps += 1;
        }
        es_ww(x, RING_TAIL, tail);
        let slot = lin32(x.c.es, CACHE_SLOTS + 2 * victim);
        let old = x.bus.read16(slot);
        x.bus.write16(slot, 0xffff);
        let at = old.wrapping_sub(2);
        let c = es_rw(x, CACHE_COUNT).wrapping_sub(1);
        es_ww(x, CACHE_COUNT, c);
        let len = es_rw(x, at);
        let end = len.wrapping_add(at);
        x.steps += 9;
        if end == es_rw(x, CACHE_HIGH) {
            // MOV ES:[0f384],0f000h; MOV ES:[0f38e],1; JMP 2c9
            es_ww(x, CACHE_LIMIT, 0xf000);
            es_ww(x, CACHE_HIGH, 1);
            x.steps += 3;
        } else if at == 0 {
            // 2a9: CMP SI,0; JNE 2c1; MOV ES:[0f386],0; MOV AX,ES:[SI]; ADD AX,0;
            // MOV ES:[0f384],AX; JMP 2c9
            es_ww(x, CACHE_FREE, 0);
            let v = es_rw(x, 0);
            es_ww(x, CACHE_LIMIT, v);
            x.steps += 7;
        } else {
            // 2c1: MOV AX,ES:[SI]; ADD ES:[0f384],AX
            let v = es_rw(x, at);
            let l = es_rw(x, CACHE_LIMIT).wrapping_add(v);
            es_ww(x, CACHE_LIMIT, l);
            x.steps += 4;
        }
        // 2c9: POP SI; POP AX; JMP 1c9
        si = x.pop();
        let _ = x.pop();
        x.steps += 3;
    }
    // 224: STOSW; SUB AX,2; MOV CX,AX; PUSH DI; SHR CX,1; JAE; MOVSB; SHR CX,1; JAE; MOVSW;
    // REP MOVSD; POP DI; POP DS
    let es = x.c.es;
    x.bus.write16(lin(es, di), size);
    let data = di.wrapping_add(2);
    let n = size.wrapping_sub(2);
    for i in 0..n {
        let b = x.bus.read8(lin(frame, si.wrapping_add(i)));
        x.bus.write8(lin(es, data.wrapping_add(i)), b);
    }
    let c1 = n >> 1;
    x.steps += 5 + 3 + (n & 1) as u64 + ((c1 & 1) as u64) + 1 + 2;
    x.set(EAX, n);
    x.set(ECX, 0);
    x.set(ESI, si.wrapping_add(n));
    x.set(EDI, data);
    x.c.ds = x.pop();
    // PUSH DI; PUSH ES; MOV AH,48h; MOV DX,[0a18]; INT 67h
    x.push(data);
    x.push(es);
    x.c.set_h(EAX, 0x48);
    let handle = x.ds_rw(EMS_HANDLE);
    x.set(EDX, handle);
    x.steps += 4;
    // The flags of SHR CX,1 (twice), with AF from SUB AX,2.
    let cmp_af = (size ^ 2 ^ n) & 0x10 != 0;
    let r = c1 >> 1;
    let flags = (x.c.flags & !(CF | PF | AF | ZF | SF | OF))
        | szp16(r)
        | if c1 & 1 != 0 { CF } else { 0 }
        | if c1 & 0x8000 != 0 { OF } else { 0 }
        | if cmp_af { AF } else { 0 };
    x.int(0x67, 0x244, flags);
    Ok(())
}

/// Port of the cache's end (`EL2MCGA3:0244`): `POP ES; POP SI` (the bitmap's offset),
/// `DEC [0a20]`, the registers restored, the offset stored as the bitmap's slot; `RETF`.
pub fn cache_after_restore(x: &mut Ctx) -> Result<(), Decline> {
    x.c.es = x.pop();
    let si = x.pop();
    x.set(ESI, si);
    let n = x.ds_rw(CACHE_BUSY).wrapping_sub(1);
    x.ds_ww(CACHE_BUSY, n);
    for r in [EBX, EDI, EDX, ECX] {
        let v = x.pop32();
        x.c.r[r] = v;
    }
    let slot = lin32(x.c.es, CACHE_SLOTS + 2 * x.c.r[EBX]);
    x.bus.write16(slot, si);
    x.steps += 8;
    x.retf();
    Ok(())
}

/// Port of `EL2MCGA3:0173` as an entry of its own (the textured fills reach it by `PUSH CS;
/// CALL`): [`cache_lookup`].
pub fn cache(x: &mut Ctx) -> Result<(), Decline> {
    cache_lookup(x);
    Ok(())
}

/// Port of `EL2MCGA3:02ce` (slot `DS:021c`): [`blit`] of bitmap `SI` through the descriptor at
/// `SS:0fd59h` (the mouse pointer's, *inferred*).
pub fn blit_pointer(x: &mut Ctx) -> Result<(), Decline> {
    // MOV BX,SI; MOV DI,0fd59h
    x.set(EBX, x.si());
    x.set(EDI, 0xfd59);
    x.steps += 2;
    blit(x)
}

/// Port of `EL2MCGA3:02d3` (slot `DS:0220`): draw bitmap `BX` at (`CX`, `DX`) through the target
/// descriptor at `SS:DI`; with `DI` = 0 only look the bitmap up (`ES:SI` → `BP`).
pub fn blit(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // INC [1fa9]; CALLF 0173
    let n = x.ds_rw(BLIT_BUSY).wrapping_add(1);
    x.ds_ww(BLIT_BUSY, n);
    x.steps += 1;
    let cs = x.c.cs;
    x.far_call(cs, 0x0173, 0x02dc);
    if cache_lookup(x) == Lookup::Miss {
        return Ok(());
    }
    blit_after_lookup(x)
}

/// Port of the blitter after the cache lookup (`EL2MCGA3:02dc`). The bitmap's 8-byte header:
/// flags (bit 0 transparent, colour 0 not drawn; bit 1 compressed), height, width, x and y
/// offsets. The descriptor at `SS:DI`: the target's far pointer, then the clip rectangle's
/// left x, top y, width, bottom y, and the gap after each row. The bitmap is clipped against
/// it and copied row by row ([`copy_rows`]); a compressed one is first unpacked into a buffer
/// on the stack: literal runs, and back references of 1–16 bytes up to 2 KB back.
pub fn blit_after_lookup(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // XOR AX,AX; OR DI,DI; JNE 2e9; MOV BP,SI; DEC [1fa9]; RETF
    x.set(EAX, 0);
    x.steps += 3;
    if x.di() == 0 {
        x.set(EBP, x.si());
        let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
        x.ds_ww(BLIT_BUSY, n);
        x.steps += 2;
        x.retf();
        return Ok(());
    }
    // 2e9: the header into CS:10e–115; SI past it into CS:120; CS:11e = 0; MOV BP,DI;
    // PUSH DS; MOV AX,ES; MOV DS,AX; LES DI,[BP]; ADD DX,CS:[114]; MOVZX AX,CS:[10f];
    // MOV CS:[118],AX; SUB DX,[BP+6]; JNS 346
    let (es, mut si) = (x.c.es, x.si());
    let h0 = x.bus.read32(lin(es, si));
    let h1 = x.bus.read32(lin(es, si.wrapping_add(4)));
    let cs = x.c.cs;
    x.bus.write32(lin(cs, 0x10e), h0);
    x.bus.write32(lin(cs, 0x112), h1);
    x.c.r[EBX] = h1;
    si = si.wrapping_add(8);
    cs_ww(x, 0x120, si);
    cs_ww(x, 0x11e, 0);
    let bp = x.di();
    x.set(EBP, bp);
    let ds = x.c.ds;
    x.push(ds);
    x.c.ds = es;
    let ss = x.c.ss;
    let d = |x: &mut Ctx, off: u16| x.bus.read16(lin(ss, bp.wrapping_add(off)));
    let (mut di, tseg) = (d(x, 0), d(x, 2));
    x.c.es = tseg;
    let flags = h0 as u8;
    let width = (h0 >> 16) as u16;
    let mut dx = x.dx().wrapping_add((h1 >> 16) as u16);
    let mut rows = (h0 >> 8) as u8 as u16;
    cs_ww(x, 0x118, rows);
    x.set(EAX, rows);
    let top = d(x, 6);
    let off = dx.wrapping_sub(top);
    dx = off;
    x.steps += 17;
    let finish = |x: &mut Ctx, si: u16, di: u16, dx: u16| {
        // 3dd: POP DS; DEC [1fa9]; RETF
        x.set(ESI, si);
        x.set(EDI, di);
        x.set(EDX, dx);
        x.c.ds = x.pop();
        let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
        x.ds_ww(BLIT_BUSY, n);
        x.steps += 2;
        x.retf();
    };
    if (off as i16) < 0 {
        // ADD CS:[118],DX; JS 3dd; JE 3dd; NEG DX; MOV AX,CS:[110]; MUL DX; ADD SI,AX;
        // XOR DX,DX; JMP 354
        rows = rows.wrapping_add(off);
        cs_ww(x, 0x118, rows);
        x.steps += 2;
        if (rows as i16) < 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
        x.steps += 1;
        if rows == 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
        let skip = off.wrapping_neg();
        let p = width as u32 * skip as u32;
        si = si.wrapping_add(p as u16);
        x.set(EAX, p as u16);
        dx = 0;
        x.steps += 6;
    } else {
        // 346: MOV BX,DX; MOV AX,[BP+8]; ADD AX,[BP+0ch]; MUL DX; ADD DI,AX; MOV DX,BX
        let stride = d(x, 8).wrapping_add(d(x, 0xc));
        let p = stride as u32 * off as u32;
        di = di.wrapping_add(p as u16);
        x.set(EAX, p as u16);
        x.set(EBX, off);
        x.steps += 6;
    }
    // 354: ADD DX,CS:[118]; SUB DX,[BP+0ah]; JLE 365; SUB CS:[118],DX; JLE 3dd
    let bottom = d(x, 0xa);
    let end = dx.wrapping_add(rows);
    let over = end.wrapping_sub(bottom);
    dx = over;
    x.steps += 3;
    if (end as i16 as i32) - (bottom as i16 as i32) > 0 {
        let left = rows as i16 as i32 - over as i16 as i32;
        rows = rows.wrapping_sub(over);
        cs_ww(x, 0x118, rows);
        x.steps += 2;
        if left <= 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
    }
    // 365: MOV AX,CS:[110]; MOV CS:[116],AX; NEG AX; ADD AX,[BP+8]; ADD AX,[BP+0ch];
    // MOV CS:[11a],AX; MOV CS:[11c],0; ADD CX,CS:[112]; SUB CX,[BP+4]; JNS 3a1
    let mut cols = width;
    cs_ww(x, 0x116, cols);
    let mut gap_dst = width.wrapping_neg().wrapping_add(d(x, 8)).wrapping_add(d(x, 0xc));
    cs_ww(x, 0x11a, gap_dst);
    let mut gap_src: u16 = 0;
    cs_ww(x, 0x11c, 0);
    x.set(EAX, gap_dst);
    let mut cx = x.cx().wrapping_add(h1 as u16).wrapping_sub(d(x, 4));
    x.steps += 10;
    if (cx as i16) < 0 {
        // ADD CS:[116],CX; JS 3dd; JE 3dd; SUB CS:[11a],CX; SUB CS:[11c],CX; SUB SI,CX;
        // XOR CX,CX
        cols = cols.wrapping_add(cx);
        cs_ww(x, 0x116, cols);
        x.set(ECX, cx);
        x.steps += 2;
        if (cols as i16) < 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
        x.steps += 1;
        if cols == 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
        gap_dst = gap_dst.wrapping_sub(cx);
        cs_ww(x, 0x11a, gap_dst);
        gap_src = gap_src.wrapping_sub(cx);
        cs_ww(x, 0x11c, gap_src);
        si = si.wrapping_sub(cx);
        cx = 0;
        x.steps += 4;
    }
    // 3a1: ADD DI,CX; ADD CX,CS:[116]; SUB CX,[BP+8]; JLE 3be; SUB CS:[116],CX; JLE 3dd;
    // ADD CS:[11a],CX; ADD CS:[11c],CX
    di = di.wrapping_add(cx);
    let right = cx.wrapping_add(cols);
    let clip_w = d(x, 8);
    let over = right.wrapping_sub(clip_w);
    x.set(ECX, over);
    x.steps += 4;
    if (right as i16 as i32) - (clip_w as i16 as i32) > 0 {
        let left = cols as i16 as i32 - over as i16 as i32;
        cols = cols.wrapping_sub(over);
        cs_ww(x, 0x116, cols);
        x.steps += 2;
        if left <= 0 {
            finish(x, si, di, dx);
            return Ok(());
        }
        gap_dst = gap_dst.wrapping_add(over);
        cs_ww(x, 0x11a, gap_dst);
        gap_src = gap_src.wrapping_add(over);
        cs_ww(x, 0x11c, gap_src);
        x.steps += 2;
    }
    // 3be: TEST CS:[10e],2; JNE 3e3
    x.steps += 2;
    if flags & 2 == 0 {
        // MOV AH,CS:[118]; MOV DX,CS:[11c]; MOV BP,CS:[11a]; MOV BX,CS:[11e]; CALL 476
        let src = x.c.ds;
        x.steps += 5;
        let (s2, d2) = copy_rows(x, src, si, di, rows as u8, cols, gap_src, gap_dst, flags);
        x.set(EBP, gap_dst);
        x.set(EBX, cols);
        x.c.set_h(EAX, 0);
        finish(x, s2, d2, gap_src);
        return Ok(());
    }
    // 3e3: MOV CS:[122],SI; MOVZX AX,CS:[10f]; MUL CS:[110]; MOV CX,SP; MOV DX,AX; INC DX;
    // AND DL,0feh; SUB SP,DX; MOV BP,SP; PUSH CX; PUSH BP; MOV SI,CS:[120]; MOV CX,AX
    cs_ww(x, 0x122, si);
    let height = (h0 >> 8) as u8 as u32;
    let total = (height * width as u32) as u16;
    let old_sp = x.sp();
    let room = total.wrapping_add(1) & !1;
    let buf = old_sp.wrapping_sub(room);
    x.set(ESP, buf);
    x.push(old_sp);
    x.push(buf);
    let mut src = cs_rw(x, 0x120);
    x.set(ECX, total);
    x.set(EBP, buf);
    x.steps += 13;
    // 408: unpack `total` bytes into SS:buf.
    let mut out = buf;
    let mut left = total;
    let ds = x.c.ds;
    let mut bx = 0u16;
    loop {
        // JCXZ 447
        x.steps += 1;
        if left == 0 {
            break;
        }
        if x.steps > 50_000_000 {
            return Err(Decline("bitmap data does not end"));
        }
        // LODSB; OR AL,AL; JS 421
        let op = x.bus.read8(lin(ds, src));
        src = src.wrapping_add(1);
        x.steps += 3;
        if (op as i8) >= 0 {
            // SUB CL,AL; SBB CH,0; MOV AH,AL; then per byte LODSB; MOV [BP],AL; INC BP;
            // DEC AH; JNE; JMP 408
            left = left.wrapping_sub(op as u16);
            x.steps += 3;
            let mut n = op;
            loop {
                let b = x.bus.read8(lin(ds, src));
                src = src.wrapping_add(1);
                x.bus.write8(lin(ss, out), b);
                out = out.wrapping_add(1);
                n = n.wrapping_sub(1);
                x.steps += 5;
                if n == 0 {
                    break;
                }
            }
            x.steps += 1;
        } else {
            // MOV AH,AL; SAR AH,3; NEG AH; SUB CL,AH; SBB CH,0; MOV BX,BP; AND AL,7;
            // SUB BH,AL; LODSB; SUB BL,AL; SBB BH,0
            let mut n = ((op as i8) >> 3).wrapping_neg() as u8;
            left = left.wrapping_sub(n as u16);
            let lo = x.bus.read8(lin(ds, src));
            src = src.wrapping_add(1);
            bx = out.wrapping_sub(((op & 7) as u16) << 8).wrapping_sub(lo as u16);
            x.steps += 11;
            loop {
                // MOV AL,SS:[BX]; MOV [BP],AL; INC BX; INC BP; DEC AH; JNE; JMP 408
                let b = x.bus.read8(lin(ss, bx));
                x.bus.write8(lin(ss, out), b);
                bx = bx.wrapping_add(1);
                out = out.wrapping_add(1);
                n = n.wrapping_sub(1);
                x.steps += 6;
                if n == 0 {
                    break;
                }
            }
            x.steps += 1;
        }
    }
    // 447: POP SI; ADD SI,CS:[122]; SUB SI,CS:[120]; MOV AX,SS; MOV DS,AX; MOV AH,CS:[118];
    // MOV DX,CS:[11c]; MOV BP,CS:[11a]; MOV BX,CS:[11e]; CALL 476
    let popped = x.pop();
    let start = popped.wrapping_add(cs_rw(x, 0x122)).wrapping_sub(cs_rw(x, 0x120));
    x.c.ds = ss;
    x.steps += 10;
    let (s2, d2) = copy_rows(x, ss, start, di, rows as u8, cols, gap_src, gap_dst, flags);
    let _ = bx;
    // POP AX; MOV SP,AX; POP DS; DEC [1fa9]; RETF
    let sp = x.pop();
    x.set(ESP, sp);
    x.set(EAX, sp);
    x.set(EBP, gap_dst);
    x.set(EBX, cols);
    x.steps += 2;
    finish(x, s2, d2, gap_src);
    Ok(())
}

/// The row copier `EL2MCGA3:0476` (a near routine, with its `RET`): `rows` rows (`AH`, 0 =
/// 256) of `cols` bytes from `src:si` to `ES:di`, the gaps added after each row; transparent
/// bitmaps (`CS:10e` bit 0) skip colour 0. Returns `SI`, `DI` after it; leaves `CX` = 0 and
/// `AL` the last byte read when transparent.
#[allow(clippy::too_many_arguments)]
fn copy_rows(x: &mut Ctx, src: u16, mut si: u16, mut di: u16, rows: u8, cols: u16, gap_src: u16, gap_dst: u16, flags: u8) -> (u16, u16) {
    let es = x.c.es;
    let mut rows = rows;
    // TEST CS:[10e],1; JNE 490; MOV BX,CS:[116]
    x.steps += 3;
    loop {
        if flags & 1 == 0 {
            // MOV CX,BX; REP MOVSB; ADD DI,BP; ADD SI,DX; DEC AH; JNE
            for _ in 0..cols {
                let b = x.bus.read8(lin(src, si));
                x.bus.write8(lin(es, di), b);
                si = si.wrapping_add(1);
                di = di.wrapping_add(1);
            }
            x.steps += 6;
        } else {
            // MOV CX,BX; per byte MOV AL,[SI]; INC SI; OR AL,AL; JE; MOV ES:[DI],AL; INC DI;
            // DEC CX; JE/JNE; then ADD DI,BP; ADD SI,DX; DEC AH; JNE
            x.steps += 1;
            let mut n = cols;
            loop {
                let b = x.bus.read8(lin(src, si));
                si = si.wrapping_add(1);
                x.c.set_l(EAX, b);
                x.steps += 7;
                if b != 0 {
                    x.bus.write8(lin(es, di), b);
                    x.steps += 1;
                }
                di = di.wrapping_add(1);
                n = n.wrapping_sub(1);
                if n == 0 {
                    break;
                }
            }
            x.steps += 4;
        }
        di = di.wrapping_add(gap_dst);
        si = si.wrapping_add(gap_src);
        rows = rows.wrapping_sub(1);
        if rows == 0 {
            break;
        }
    }
    x.set(ECX, 0);
    x.steps += 1; // RET
    (si, di)
}
