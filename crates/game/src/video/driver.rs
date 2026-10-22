//! The page flip and the driver's start-up (docs/VIDEO.md §2, §4). Both are mostly calls: to
//! the main program's mouse pointer code, to DOS and the BIOS, and to the driver's own exports.
//! Each port ends at its first call out; the code after each call is its own entry (`flip@..`,
//! `init@..` in [`EXPORTS`]).
//!
//! Each port mirrors the original's control flow block by block; the comment on a block gives
//! its address and the number added to the instruction count is that block's instruction count.

use super::*;

/// The flags byte of the mouse pointer in the stack segment (`SS:0fa06`; bit 1: the pointer is
/// drawn, *inferred*).
pub const POINTER_FLAGS: u16 = 0xfa06;

fn ss_rb(x: &mut Ctx, off: u16) -> u8 {
    x.bus.read8(lin(x.c.ss, off))
}

/// Port of `EL2MCGA3:0796` (slot `DS:025c`): show the frame. First the main program's
/// `DS:0140` (the pointer shape, *inferred*).
pub fn flip(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x0140, 0x079a);
    Ok(())
}

/// Port of `EL2MCGA3:079a`: `DS:09fe` = `DS:09fc`, which is cleared (a frame counter,
/// *inferred*); then the main program's `DS:004c` (the pointer position).
pub fn flip_after_shape(x: &mut Ctx) -> Result<(), Decline> {
    // XOR AX,AX; XCHG [09fc],AX; MOV [09fe],AX; INC [1fa9]; CALLF [004c]
    let v = x.ds_rw(0x09fc);
    x.ds_ww(0x09fc, 0);
    x.ds_ww(0x09fe, v);
    x.set(EAX, v);
    let n = x.ds_rw(BLIT_BUSY).wrapping_add(1);
    x.ds_ww(BLIT_BUSY, n);
    x.steps += 4;
    x.call_slot(0x004c, 0x07ab);
    Ok(())
}

/// Port of `EL2MCGA3:07ab`: if the pointer is shown, store its position (`CX`, `DX`), and save
/// the back buffer under it (`DS:0240`); else go on at [`flip_after_pointer`].
pub fn flip_after_position(x: &mut Ctx) -> Result<(), Decline> {
    // TEST SS:[0fa06],2; JE 7d4
    x.steps += 2;
    if ss_rb(x, POINTER_FLAGS) & 2 == 0 {
        return flip_after_pointer(x);
    }
    // MOV [0a0c],CX; MOV [0a0e],DX; MOV ES,[09e2]; PUSH CX; PUSH DX; XOR SS:[0fa06],40h;
    // CALLF [0240]
    let (cx, dx) = (x.cx(), x.dx());
    x.ds_ww(POINTER_X, cx);
    x.ds_ww(POINTER_Y, dx);
    x.c.es = x.ds_rw(TARGET);
    x.push(cx);
    x.push(dx);
    let a = lin(x.c.ss, POINTER_FLAGS);
    let f = x.bus.read8(a);
    x.bus.write8(a, f ^ 0x40);
    x.steps += 6;
    x.call_slot(0x0240, 0x07cb);
    Ok(())
}

/// Port of `EL2MCGA3:07cb`: draw the pointer through the main program's `DS:0100`.
pub fn flip_after_save(x: &mut Ctx) -> Result<(), Decline> {
    // POP DX; POP CX; MOV DI,0fd59h; CALLF [0100]
    let dx = x.pop();
    let cx = x.pop();
    x.set(EDX, dx);
    x.set(ECX, cx);
    x.set(EDI, 0xfd59);
    x.steps += 3;
    x.call_slot(0x0100, 0x07d4);
    Ok(())
}

/// Port of `EL2MCGA3:07d4`: `CALLF [0244]` (the driver's empty export).
pub fn flip_after_pointer(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x0244, 0x07d8);
    Ok(())
}

/// Port of `EL2MCGA3:07d8`: toggle bit 6 of `DS:891b`, copy the back buffer (`DS:09e6`) to video
/// memory (64,000 bytes), and if the pointer is shown put the back buffer under it back
/// (`DS:0230`, keeping `SS:0`).
pub fn flip_after_hook(x: &mut Ctx) -> Result<(), Decline> {
    x.forward()?;
    // XOR [891b],40h; PUSH DS; MOV DS,[09e6]; XOR SI,SI; MOV DI,SI; MOV BX,0a000h; MOV ES,BX;
    // MOV CX,3e80h; REP MOVSD; POP DS; TEST SS:[0fa06],2; JE 80c
    let f = x.ds_rb(0x891b);
    x.ds_wb(0x891b, f ^ 0x40);
    let ds = x.c.ds;
    x.push(ds);
    let back = x.ds_rw(BACK_BUFFER);
    for i in 0..64000u16 {
        let b = x.bus.read8(lin(back, i));
        x.bus.write8(lin(0xa000, i), b);
    }
    x.c.ds = x.pop();
    x.c.es = 0xa000;
    x.set(EBX, 0xa000);
    x.set(ESI, 64000);
    x.set(EDI, 64000);
    x.set(ECX, 0);
    x.steps += 12;
    if ss_rb(x, POINTER_FLAGS) & 2 == 0 {
        return flip_end(x);
    }
    // PUSH SS:[0]; MOV ES,[09e2]; CALLF [0230]
    let saved = x.bus.read16(lin(x.c.ss, 0));
    x.push(saved);
    x.c.es = x.ds_rw(TARGET);
    x.steps += 2;
    x.call_slot(0x0230, 0x0807);
    Ok(())
}

/// Port of `EL2MCGA3:0807`: `POP SS:[0]`, then the end of the flip.
pub fn flip_after_restore(x: &mut Ctx) -> Result<(), Decline> {
    let v = x.pop();
    x.bus.write16(lin(x.c.ss, 0), v);
    x.steps += 1;
    flip_end(x)
}

/// `EL2MCGA3:080c`: `DEC [1fa9]; RETF`.
fn flip_end(x: &mut Ctx) -> Result<(), Decline> {
    let n = x.ds_rw(BLIT_BUSY).wrapping_sub(1);
    x.ds_ww(BLIT_BUSY, n);
    x.steps += 1;
    x.retf();
    Ok(())
}

/// Main data segment table of row offsets (200 words, y·320) the start-up writes.
pub const ROW_TABLE: u16 = 0x083a;

/// The flags of `ADD r16,b` giving `r` from `a` (CF, PF, AF, ZF, SF, OF), as the runtime sets them.
fn add_flags(a: u16, b: u16) -> u32 {
    let r = a.wrapping_add(b);
    let mut f = szp16(r);
    if (a as u32 + b as u32) > 0xffff {
        f |= CF;
    }
    if (a ^ r) & (b ^ r) & 0x8000 != 0 {
        f |= OF;
    }
    if (a ^ b ^ r) & 0x10 != 0 {
        f |= AF;
    }
    f
}

/// Port of `EL2MCGA3:05b8` (slot `DS:020c`): the driver's start-up. The row table at `DS:083a`,
/// then DOS allocates the bitmap cache (`int 21h` 48h, 0F39h paragraphs).
pub fn init(x: &mut Ctx) -> Result<(), Decline> {
    // MOV SI,83ah; MOV CX,0c8h; XOR AX,AX; per row MOV [SI],AX; ADD SI,2; ADD AX,140h; LOOP
    x.steps += 3;
    let mut ax: u16 = 0;
    for y in 0..200u16 {
        x.ds_ww(ROW_TABLE + 2 * y, ax);
        ax = ax.wrapping_add(WIDTH);
        x.steps += 4;
    }
    x.set(ESI, ROW_TABLE + 400);
    x.set(ECX, 0);
    // MOV AH,48h; MOV BX,0f39h; INT 21h
    x.set(EAX, 0x4800 | (ax & 0xff));
    x.set(EBX, 0x0f39);
    x.steps += 2;
    let flags = (x.c.flags & !(CF | PF | AF | ZF | SF | OF)) | add_flags(ax.wrapping_sub(WIDTH), WIDTH);
    x.int(0x21, 0x05d1, flags);
    Ok(())
}

/// Port of `EL2MCGA3:05d1`: on failure the fatal-error exit (`JMP FAR [002c]`, `AX` = 1900h);
/// else the cache segment (`DS:1fa7` = `AX`) is set up empty and the BIOS sets mode 13h
/// (`int 10h`).
pub fn init_after_alloc(x: &mut Ctx) -> Result<(), Decline> {
    // JAE 5da; MOV AX,1900h; JMP FAR [002c]
    x.steps += 1;
    if x.c.flags & CF != 0 {
        x.set(EAX, 0x1900);
        let (off, seg) = (x.ds_rw(0x2c), x.ds_rw(0x2e));
        x.c.cs = seg;
        x.c.ip = off;
        x.steps += 2;
        return Ok(());
    }
    x.forward()?;
    // MOV [1fa7],AX; MOV ES,AX; the allocator words; MOV DI,0f000h; MOV AX,0ffffh;
    // MOV CX,0e1h; REP STOSW; MOV AX,13h; INT 10h
    let seg = x.ax();
    x.ds_ww(CACHE_SEGMENT, seg);
    x.c.es = seg;
    for (off, v) in [(CACHE_LIMIT, 0xf000), (CACHE_FREE, 0), (RING_HEAD, RING_START), (RING_TAIL, RING_START), (CACHE_COUNT, 0), (CACHE_HIGH, 1)] {
        x.bus.write16(lin(seg, off), v);
    }
    for i in 0..0xe1u16 {
        x.bus.write16(lin(seg, 0xf000 + 2 * i), 0xffff);
    }
    x.set(EDI, 0xf000 + 2 * 0xe1);
    x.set(ECX, 0);
    x.set(EAX, 0x13);
    x.steps += 2 + 6 + 4 + 1;
    let flags = x.c.flags;
    x.int(0x10, 0x0619, flags);
    Ok(())
}

/// Port of `EL2MCGA3:0619`: `DEC [8919]`, then palette 0 (`DS:024c` with `AL` = 0).
pub fn init_after_mode(x: &mut Ctx) -> Result<(), Decline> {
    // DEC [8919]; XOR AL,AL; CALLF [024c]
    let v = x.ds_rb(0x8919).wrapping_sub(1);
    x.ds_wb(0x8919, v);
    x.c.set_l(EAX, 0);
    x.steps += 2;
    x.call_slot(0x024c, 0x0623);
    Ok(())
}

/// Port of `EL2MCGA3:0623`: `CALL 66f` (the back buffer is the EMS page frame segment,
/// `DS:0a1a`, which becomes the target and both pointer descriptors' segment), then draw into
/// it (`DS:0268`).
pub fn init_after_palette(x: &mut Ctx) -> Result<(), Decline> {
    // CALL 66f: MOV AX,[0a1a]; MOV [09e6],AX; MOV [09e2],AX; MOV SS:[0fd5b],AX;
    // MOV SS:[0fd69],AX; RET
    x.push(0x0626);
    let v = x.ds_rw(0x0a1a);
    x.ds_ww(BACK_BUFFER, v);
    x.ds_ww(TARGET, v);
    let ss = x.c.ss;
    x.bus.write16(lin(ss, 0xfd5b), v);
    x.bus.write16(lin(ss, 0xfd69), v);
    x.set(EAX, v);
    let _ = x.pop();
    x.steps += 1 + 6;
    // CALLF [0268]
    x.call_slot(0x0268, 0x062a);
    Ok(())
}

/// Port of `EL2MCGA3:062a`: open `EL2G.GRF` (the name at the driver's paragraph 05d4h + 1Bh)
/// through the main program's `DS:00c0`.
pub fn init_after_target(x: &mut Ctx) -> Result<(), Decline> {
    // MOV AX,05d4h; MOV ES,AX; MOV DX,1bh; CALLF [00c0]
    let seg = x.c.cs.wrapping_add(0x5d4);
    x.set(EAX, seg);
    x.c.es = seg;
    x.set(EDX, 0x1b);
    x.steps += 3;
    x.call_slot(0x00c0, 0x0636);
    Ok(())
}

/// Port of `EL2MCGA3:0636`: read the bitmap directory into `DS:5813` through `DS:0094`.
pub fn init_after_open(x: &mut Ctx) -> Result<(), Decline> {
    // MOV DX,DS; MOV AX,5813h; CALLF [0094]
    let ds = x.c.ds;
    x.set(EDX, ds);
    x.set(EAX, BITMAP_DIRECTORY);
    x.steps += 2;
    x.call_slot(0x0094, 0x063f);
    Ok(())
}

/// Port of `EL2MCGA3:063f`: pointer bitmap 0 (`DS:0254` with `BX` = 0).
pub fn init_after_read(x: &mut Ctx) -> Result<(), Decline> {
    // OR AX,AX; MOV BX,0; CALLF [0254]
    x.set(EBX, 0);
    x.steps += 2;
    x.call_slot(0x0254, 0x0648);
    Ok(())
}

/// Port of `EL2MCGA3:0648`: show the pointer (`DS:0024`).
pub fn init_after_pointer(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x0024, 0x064c);
    Ok(())
}

/// `XOR AX,AX; MOV BX,13fh; MOV CL,AL; MOV DL,0c7h; XOR BP,BP; CALLF [01ec]`: clear the screen
/// to colour 0.
fn clear_screen(x: &mut Ctx, ret: u16) {
    x.set(EAX, 0);
    x.set(EBX, 0x13f);
    x.c.set_l(ECX, 0);
    x.c.set_l(EDX, 0xc7);
    x.set(EBP, 0);
    x.steps += 5;
    x.call_slot(0x01ec, ret);
}

/// Port of `EL2MCGA3:064c`: clear the back buffer (`DS:01ec`).
pub fn init_after_show(x: &mut Ctx) -> Result<(), Decline> {
    clear_screen(x, 0x065b);
    Ok(())
}

/// Port of `EL2MCGA3:065b`: show it (`DS:025c`).
pub fn init_after_clear(x: &mut Ctx) -> Result<(), Decline> {
    x.call_slot(0x025c, 0x065f);
    Ok(())
}

/// Port of `EL2MCGA3:065f`: clear the back buffer again (`DS:01ec`).
pub fn init_after_flip(x: &mut Ctx) -> Result<(), Decline> {
    clear_screen(x, 0x066e);
    Ok(())
}
