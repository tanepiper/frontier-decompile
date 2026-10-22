//! Generated names (docs/NAMES.md): the name generator in `EL2M09` and the main program's
//! text expander it copies words with.
//!
//! The port is register- and memory-exact and returns the original's instruction count. It
//! reads the word tables from the overlay in memory; nothing of the game is embedded here.

use crate::math3::{lin, Bus, Cpu, Port, DECLINE, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use crate::rng::{mix, MIX_STEPS};

/// Offset of the word table in `EL2M09` (word offsets relative to the table, then the text).
const WORDS: u16 = 0x00a3;
/// `EL2M09` tables for kinds 6–14: first word index and word count of each kind's template.
const KIND_BASE: u16 = 0x0f61;
const KIND_COUNT: u16 = 0x0f73;

/// The generator's working registers, its stack and its instruction count.
struct St<'a> {
    ax: u16,
    bx: u16,
    cx: u16,
    dx: u16,
    si: u16,
    di: u16,
    bp: u16,
    sp: u16,
    es: u16,
    ss: u16,
    ds: u16,
    /// `EL2M09`'s code segment.
    cs: u16,
    n: u64,
    bus: &'a mut dyn Bus,
}

/// The original met an `FFh` escape (a nested string through `DS:0014`), which is not ported.
struct Decline;

impl St<'_> {
    fn push(&mut self, v: u16) {
        self.sp = self.sp.wrapping_sub(2);
        self.bus.write16(lin(self.ss, self.sp), v);
    }

    fn pop(&mut self) -> u16 {
        let v = self.bus.read16(lin(self.ss, self.sp));
        self.sp = self.sp.wrapping_add(2);
        v
    }

    /// `CALLF [DS:0128]`: [`mix`] with `AX` as the range.
    fn mix(&mut self) {
        let (ax, bx, cx, dx, si, di) = mix(self.ax, self.bx, self.cx, self.si, self.di);
        (self.ax, self.bx, self.cx, self.dx, self.si, self.di) = (ax, bx, cx, dx, si, di);
        self.n += 1 + MIX_STEPS;
    }

    fn wb(&mut self, off: u16, v: u8) {
        self.bus.write8(lin(self.ss, off), v);
    }

    /// `EL2M09:11c3` (entered by `PUSH CS; CALL`) and the main program's text expander
    /// `1000:e969` (slot `DS:005c`), *observed*: copy word `AX` of the `EL2M09` table to `SS:DI`.
    /// Ordinary bytes are copied up to and including the 0; bytes 80h–FCh are copied too; `FDh`
    /// writes `1Eh` and the next byte, `FEh` writes `1Fh` and the next word; `FFh` (expand another
    /// string) declines.
    fn copy_word(&mut self) -> Result<(), Decline> {
        // 11c3: SHL AX,1; MOV SI,CS; MOV ES,SI; MOV SI,00a3h; CALLF [005c]
        self.ax <<= 1;
        self.es = self.cs;
        self.si = WORDS;
        self.n += 5;
        // e969: MOV BX,AX; ADD SI,ES:[BX+SI]
        self.bx = self.ax;
        let rel = self.bus.read16(lin(self.es, self.bx.wrapping_add(self.si)));
        self.si = self.si.wrapping_add(rel);
        self.n += 2;
        loop {
            let al = self.bus.read8(lin(self.es, self.si));
            self.si = self.si.wrapping_add(1);
            self.ax = (self.ax & 0xff00) | al as u16;
            self.wb(self.di, al);
            self.di = self.di.wrapping_add(1);
            if (al as i8) > 0 {
                self.n += 5;
                continue;
            }
            if al == 0 {
                self.n += 7; // ... JNZ (not taken), RETF
                break;
            }
            self.di = self.di.wrapping_sub(1);
            if al < 0xfd {
                self.di = self.di.wrapping_add(1);
                self.n += 11;
                continue;
            }
            if al == 0xff {
                return Err(Decline);
            }
            if al == 0xfe {
                self.wb(self.di, 0x1f);
                let w = self.bus.read16(lin(self.es, self.si));
                self.si = self.si.wrapping_add(2);
                self.ax = w;
                self.bus.write16(lin(self.ss, self.di.wrapping_add(1)), w);
                self.di = self.di.wrapping_add(3);
            } else {
                self.wb(self.di, 0x1e);
                let b = self.bus.read8(lin(self.es, self.si));
                self.si = self.si.wrapping_add(1);
                self.ax = (self.ax & 0xff00) | b as u16;
                self.wb(self.di.wrapping_add(1), b);
                self.di = self.di.wrapping_add(2);
            }
            self.n += 17;
        }
        self.n += 1; // 11c3's RETF
        Ok(())
    }

    /// `PUSH BX; PUSH CX; PUSH DI; PUSH SI; MOV DI,BP; PUSH CS; CALL 11c3`, the copy, then (with
    /// `space`) `MOV SS:[DI-1],20h`, then `MOV BP,DI; POP SI; POP DI; POP CX; POP BX`.
    fn copy_block(&mut self, space: bool) -> Result<(), Decline> {
        let (bx, cx, di, si) = (self.bx, self.cx, self.di, self.si);
        self.push(bx);
        self.push(cx);
        self.push(di);
        self.push(si);
        self.di = self.bp;
        let cs = self.cs;
        self.push(cs);
        self.push(0); // return offset (dead stack)
        self.n += 7;
        self.copy_word()?;
        self.sp = self.sp.wrapping_add(4); // the RETF
        if space {
            self.wb(self.di.wrapping_sub(1), 0x20);
            self.n += 1;
        }
        self.bp = self.di;
        self.si = self.pop();
        self.di = self.pop();
        self.cx = self.pop();
        self.bx = self.pop();
        self.n += 5;
        Ok(())
    }

    /// `EL2M09:1032`: a last word. `BL` < 19h: word `4Fh + random(10)`, then `s` replaces the
    /// terminator, `on` follows unless `BH` bit 3 is set, and a new terminator. Otherwise
    /// ([`Self::last_word_1037`]) word `59h + random(8Ah)`. Ends with `RETF` (counted).
    fn last_word_1032(&mut self) -> Result<(), Decline> {
        self.n += 2; // CMP BL,19h; JC
        if (self.bx as u8) >= 0x19 {
            return self.last_word_1037();
        }
        self.ax = 0x0a;
        self.n += 1;
        self.mix();
        self.ax = self.ax.wrapping_add(0x4f);
        self.n += 1;
        self.copy_block(false)?;
        self.wb(self.bp.wrapping_sub(1), b's');
        self.n += 3; // MOV [BP-1],'s'; TEST BH,8; JNZ
        if self.bx & 0x0800 == 0 {
            self.bus.write16(lin(self.ss, self.bp), 0x6e6f);
            self.bp = self.bp.wrapping_add(2);
            self.n += 2;
        }
        self.wb(self.bp, 0);
        self.bp = self.bp.wrapping_add(1);
        self.n += 3; // MOV [BP],0; INC BP; RETF
        Ok(())
    }

    fn last_word_1037(&mut self) -> Result<(), Decline> {
        self.ax = 0x8a;
        self.n += 1;
        self.mix();
        self.ax = self.ax.wrapping_add(0x59);
        self.n += 1;
        self.copy_block(false)?;
        self.n += 1; // RETF
        Ok(())
    }

    /// `EL2M09:11a7` (a letter, `'A' + random(26)`) or `11b5` (a digit, `'0' + random(10)`):
    /// `CALL`, `MOV AX,n`, mix, `ADD AL,c`, `MOV [BP],AL`, `INC BP`, `RET`.
    fn char_at_bp(&mut self, range: u16, base: u8) {
        self.ax = range;
        self.n += 2;
        self.mix();
        let al = (self.ax as u8).wrapping_add(base);
        self.ax = (self.ax & 0xff00) | al as u16;
        self.wb(self.bp, al);
        self.bp = self.bp.wrapping_add(1);
        self.n += 4;
    }

    /// `SHL DI,1; RCL SI,1; ADC DI,0` `k` times: rotate `SI:DI` left by `k`.
    fn rol_sidi(&mut self, k: u32) {
        let v = ((self.si as u32) << 16 | self.di as u32).rotate_left(k);
        self.si = (v >> 16) as u16;
        self.di = v as u16;
        self.n += 3 * k as u64;
    }

    /// `EL2M09:0fc0`.
    fn generate(&mut self) -> Result<(), Decline> {
        self.bp = self.di;
        self.n += 3; // MOV BP,DI; CMP AX,6; JNC
        if self.ax < 6 {
            // SI:DI = (BX:CX) + (CX:BX), rotated left 7.
            let v = ((self.bx as u32) << 16 | self.cx as u32).wrapping_add((self.cx as u32) << 16 | self.bx as u32);
            self.si = (v >> 16) as u16;
            self.di = v as u16;
            self.n += 4;
            self.rol_sidi(7);
            let kind = self.ax;
            enum Next {
                W1016,
                W1083,
                W108f,
            }
            let next = if kind == 1 {
                self.n += 2;
                Next::W1016
            } else if kind == 2 {
                self.n += 4;
                Next::W1083
            } else if kind == 4 {
                self.n += 6;
                Next::W108f
            } else {
                self.n += 8; // ... TEST BL,1; JNZ
                if self.bx & 1 != 0 { Next::W1083 } else { Next::W1016 }
            };
            match next {
                Next::W1016 => {
                    self.ax = 0x69;
                    self.n += 1;
                    self.mix();
                }
                Next::W1083 => {
                    self.ax = 0x3c;
                    self.n += 1;
                    self.mix();
                    self.ax = self.ax.wrapping_add(0xe0);
                    self.n += 2; // ADD AX,E0h; JMP 101d
                }
                Next::W108f => {
                    self.mix();
                    self.n += 1; // JMP 1037
                    return self.last_word_1037();
                }
            }
            self.copy_block(true)?;
            return self.last_word_1032();
        }
        // Kinds 6 and up: CX:BX += BX:CX; SI:DI = BX:CX rotated; two more mixes of the seed.
        let (si0, di0) = (self.bx, self.cx);
        let v = ((self.cx as u32) << 16 | self.bx as u32).wrapping_add((si0 as u32) << 16 | di0 as u32);
        self.cx = (v >> 16) as u16;
        self.bx = v as u16;
        self.si = si0;
        self.di = di0;
        self.n += 4;
        self.rol_sidi(7);
        self.add_sidi_to_cxbx();
        self.rol_sidi(4);
        self.add_sidi_to_cxbx();
        self.n += 2; // CMP AX,0Fh; JNC
        if self.ax < 0x0f {
            return self.template();
        }
        self.n += 2; // CMP AX,0Fh; JA
        if self.ax == 0x0f {
            self.char_at_bp(26, b'A');
            self.char_at_bp(26, b'A');
            self.wb(self.bp, b'-');
            self.bp = self.bp.wrapping_add(1);
            self.n += 2;
            for _ in 0..3 {
                self.char_at_bp(10, b'0');
            }
        } else {
            self.n += 2; // CMP AX,10h; JNZ
            if self.ax == 0x10 {
                self.bus.write16(lin(self.ss, self.bp), 0x202b);
                self.n += 1;
                for _ in 0..3 {
                    self.char_at_bp(10, b'0');
                }
                self.char_at_bp(26, b'A');
            } else {
                self.char_at_bp(26, b'A');
                self.char_at_bp(26, b'A');
                self.bus.write16(lin(self.ss, self.bp), 0x2a20);
                self.wb(self.bp.wrapping_add(2), 0x20);
                self.bp = self.bp.wrapping_add(3);
                self.n += 3;
                self.char_at_bp(10, b'0');
            }
        }
        self.wb(self.bp, 0);
        self.bp = self.bp.wrapping_add(1);
        self.n += 3; // MOV [BP],0; INC BP; RETF
        Ok(())
    }

    fn add_sidi_to_cxbx(&mut self) {
        let v = ((self.cx as u32) << 16 | self.bx as u32).wrapping_add((self.si as u32) << 16 | self.di as u32);
        self.cx = (v >> 16) as u16;
        self.bx = v as u16;
        self.n += 2;
    }

    /// Kinds 6–14 (`EL2M09:10f7`): a sub-name ([`Self::last_word_1032`]) into a 1Eh-byte stack
    /// buffer, published in `[DS:07e4]`, then a template word `EL2M09:0f61[kind] +
    /// random(EL2M09:0f73[kind])` copied to the output.
    fn template(&mut self) -> Result<(), Decline> {
        let ds = self.ds;
        let saved = self.bus.read32(lin(ds, 0x07e4));
        self.sp = self.sp.wrapping_sub(4);
        self.bus.write32(lin(self.ss, self.sp), saved);
        self.sp = self.sp.wrapping_sub(0x1e);
        let (ax, bp) = (self.ax, self.bp);
        self.push(ax);
        self.push(bp);
        self.bp = self.sp.wrapping_add(4);
        let cs = self.cs;
        self.push(cs);
        self.push(0x110b);
        self.n += 7; // PUSH [7e4]; SUB SP; PUSH AX; PUSH BP; MOV BP,SP; ADD BP,4; CALLF 1032
        self.last_word_1032()?;
        self.sp = self.sp.wrapping_add(4);
        self.bp = self.pop();
        self.ax = self.pop();
        self.bus.write16(lin(ds, 0x07e4), self.sp);
        self.bus.write16(lin(ds, 0x07e6), self.ss);
        self.ax <<= 1;
        let saved_7c8 = self.bus.read16(lin(ds, 0x07c8));
        self.push(saved_7c8);
        let bx = self.bx;
        self.push(bx);
        self.bx = self.ax;
        let base = self.bus.read16(lin(self.cs, KIND_BASE.wrapping_add(self.bx)));
        self.bus.write16(lin(ds, 0x07c8), base);
        self.ax = self.bus.read16(lin(self.cs, KIND_COUNT.wrapping_add(self.bx)));
        self.bx = self.pop();
        self.n += 12; // POP BP; POP AX; MOV [7e4]; MOV [7e6]; SHL; PUSH [7c8]; PUSH BX; MOV BX,AX; MOV AX,CS:; MOV [7c8]; MOV AX,CS:; POP BX
        self.mix();
        self.ax = self.ax.wrapping_add(self.bus.read16(lin(ds, 0x07c8)));
        let v = self.pop();
        self.bus.write16(lin(ds, 0x07c8), v);
        self.n += 2; // ADD AX,[7c8]; POP [7c8]
        self.copy_block(false)?;
        self.sp = self.sp.wrapping_add(0x1e);
        let v = self.bus.read32(lin(self.ss, self.sp));
        self.sp = self.sp.wrapping_add(4);
        self.bus.write32(lin(ds, 0x07e4), v);
        self.n += 3; // ADD SP,1Eh; POP [7e4]; RETF
        Ok(())
    }
}

/// Port of `EL2M09:0f92` (module 9, handler `+20`), *observed*: write a generated name of kind
/// `AX` from the seed `CX:BX` to `SS:DI`, terminated by 0. `BP` returns just past the name (or
/// past its last word). `[DS:07e0]`–`07ef` are saved and restored around the work. Declines when
/// a copied word contains an `FFh` escape (a nested string expansion).
pub fn name_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let ds = c.ds;
    let mut saved = [0u32; 4];
    for (i, s) in saved.iter_mut().enumerate() {
        *s = bus.read32(lin(ds, 0x07e0 + 4 * i as u16));
    }
    let mut sp = c.x(ESP);
    for s in saved {
        sp = sp.wrapping_sub(4);
        bus.write32(lin(c.ss, sp), s);
    }
    // CALLF 1000:0fc0: return address on the stack.
    sp = sp.wrapping_sub(4);
    bus.write16(lin(c.ss, sp.wrapping_add(2)), c.cs);
    bus.write16(lin(c.ss, sp), 0x0fab);
    let mut st = St {
        ax: c.x(EAX),
        bx: c.x(EBX),
        cx: c.x(ECX),
        dx: c.x(EDX),
        si: c.x(ESI),
        di: c.x(EDI),
        bp: c.x(EBP),
        sp,
        es: c.es,
        ss: c.ss,
        ds,
        cs: c.cs,
        n: 5,
        bus,
    };
    if st.generate().is_err() {
        return DECLINE;
    }
    let n = st.n + 5; // four POP dword, RETF
    let (ax, bx, cx, dx, si, di, bp, es) = (st.ax, st.bx, st.cx, st.dx, st.si, st.di, st.bp, st.es);
    for (i, s) in saved.iter().enumerate() {
        bus.write32(lin(ds, 0x07e0 + 4 * i as u16), *s);
    }
    c.set_x(EAX, ax);
    c.set_x(EBX, bx);
    c.set_x(ECX, cx);
    c.set_x(EDX, dx);
    c.set_x(ESI, si);
    c.set_x(EDI, di);
    c.set_x(EBP, bp);
    c.es = es;
    c.retf(bus);
    n
}

/// Every ported routine of `EL2M09`: (offset, vector slot, name, port).
pub const PORTS_M09: &[(u16, u16, &str, Port)] = &[(0x0f92, 0, "name", name_port)];
