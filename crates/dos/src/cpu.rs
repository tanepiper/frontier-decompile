//! An Intel 80386 interpreter for real mode, enough for `FRONTIER.EXE` and its overlays.
//!
//! The DOS build is 16-bit real-mode code that uses 32-bit registers and arithmetic heavily
//! through the `66h` operand-size prefix (docs/FINDINGS.md §2), so both prefixes (`66h`, `67h`),
//! `FS`/`GS`, the `0Fh` two-byte opcodes and SIB addressing are implemented. Protected mode is
//! not: a byte scan finds no `mov cr0`/`lgdt` in the program or overlays. There is no x87: the
//! main program only probes for one during set-up, which the reimplementation skips.
//!
//! Software interrupts whose vector points at the runtime's own BIOS stubs reach the host
//! through the private opcode `0F FF nn` ([`Event::Hle`]), so programs can hook and chain
//! vectors exactly as on real hardware.

use crate::mem::{lin, Memory};

pub const CF: u32 = 0x0001;
pub const PF: u32 = 0x0004;
pub const AF: u32 = 0x0010;
pub const ZF: u32 = 0x0040;
pub const SF: u32 = 0x0080;
pub const TF: u32 = 0x0100;
pub const IF: u32 = 0x0200;
pub const DF: u32 = 0x0400;
pub const OF: u32 = 0x0800;

pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

pub const ES: usize = 0;
pub const CS: usize = 1;
pub const SS: usize = 2;
pub const DS: usize = 3;
pub const FS: usize = 4;
pub const GS: usize = 5;

/// Port I/O, supplied by the machine.
pub trait Io {
    fn inb(&mut self, port: u16) -> u8;
    fn outb(&mut self, port: u16, v: u8);
    fn inw(&mut self, port: u16) -> u16 {
        self.inb(port) as u16 | (self.inb(port.wrapping_add(1)) as u16) << 8
    }
    fn outw(&mut self, port: u16, v: u16) {
        self.outb(port, v as u8);
        self.outb(port.wrapping_add(1), (v >> 8) as u8);
    }
}

/// Why [`Cpu::step`] stopped and needs the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `0F FF nn`: host service `nn` (a BIOS/DOS stub).
    Hle(u8),
    /// `HLT` executed.
    Halt,
    /// An opcode this interpreter does not implement, at `cs:ip`.
    Unimplemented { cs: u16, ip: u16, op: u16 },
    /// About to execute the instruction at this linear address, which has a breakpoint (a
    /// native port hooked in at an original routine's entry). Nothing was executed.
    Breakpoint(u32),
}

#[derive(Clone, Copy)]
struct Ea {
    reg: Option<usize>,
    seg: usize,
    off: u32,
}

pub struct Cpu {
    pub r: [u32; 8],
    pub s: [u16; 6],
    pub ip: u32,
    pub flags: u32,
    pub mem: Memory,
    /// Instructions executed so far.
    pub icount: u64,
    // Per-instruction decode state.
    seg_ovr: Option<usize>,
    os32: bool,
    as32: bool,
    rep: u8,
    start_ip: u32,
    /// Interrupts are inhibited for one instruction after `mov ss`/`pop ss`/`sti`.
    pub inhibit_irq: bool,
    pub halted: bool,
    /// One bit per linear address: stop with [`Event::Breakpoint`] before executing there.
    breakpoints: Vec<u64>,
    /// Number of set breakpoints (the check is skipped while 0).
    breakpoint_count: usize,
    /// Skip the breakpoint check once (to execute the original at a hooked address).
    pub skip_breakpoint: bool,
    /// Last few `cs:ip` values executed (debugging aid).
    pub trace: [(u16, u16); 64],
    trace_pos: usize,
}

#[inline(always)]
fn mask(sz: u8) -> u32 {
    match sz {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    }
}

#[inline(always)]
fn sign(sz: u8) -> u32 {
    match sz {
        1 => 0x80,
        2 => 0x8000,
        _ => 0x8000_0000,
    }
}

#[inline(always)]
fn sext(v: u32, sz: u8) -> i64 {
    match sz {
        1 => v as u8 as i8 as i64,
        2 => v as u16 as i16 as i64,
        _ => v as i32 as i64,
    }
}

#[inline(always)]
fn parity(v: u32) -> bool {
    (v as u8).count_ones() & 1 == 0
}

impl Cpu {
    pub fn new(mem: Memory) -> Self {
        Cpu {
            r: [0; 8],
            s: [0; 6],
            ip: 0,
            flags: 0x0002 | IF,
            mem,
            icount: 0,
            seg_ovr: None,
            os32: false,
            as32: false,
            rep: 0,
            start_ip: 0,
            inhibit_irq: false,
            halted: false,
            breakpoints: vec![0; 0x10_0000 / 64],
            breakpoint_count: 0,
            skip_breakpoint: false,
            trace: [(0, 0); 64],
            trace_pos: 0,
        }
    }

    // ---- registers -------------------------------------------------------------------------

    #[inline(always)]
    pub fn get_reg(&self, i: usize, sz: u8) -> u32 {
        match sz {
            1 => {
                if i < 4 {
                    self.r[i] & 0xFF
                } else {
                    (self.r[i - 4] >> 8) & 0xFF
                }
            }
            2 => self.r[i] & 0xFFFF,
            _ => self.r[i],
        }
    }

    #[inline(always)]
    pub fn set_reg(&mut self, i: usize, sz: u8, v: u32) {
        match sz {
            1 => {
                if i < 4 {
                    self.r[i] = (self.r[i] & !0xFF) | (v & 0xFF);
                } else {
                    self.r[i - 4] = (self.r[i - 4] & !0xFF00) | ((v & 0xFF) << 8);
                }
            }
            2 => self.r[i] = (self.r[i] & !0xFFFF) | (v & 0xFFFF),
            _ => self.r[i] = v,
        }
    }

    pub fn ax(&self) -> u16 {
        self.r[EAX] as u16
    }
    pub fn bx(&self) -> u16 {
        self.r[EBX] as u16
    }
    pub fn cx(&self) -> u16 {
        self.r[ECX] as u16
    }
    pub fn dx(&self) -> u16 {
        self.r[EDX] as u16
    }
    pub fn si(&self) -> u16 {
        self.r[ESI] as u16
    }
    pub fn di(&self) -> u16 {
        self.r[EDI] as u16
    }
    pub fn ah(&self) -> u8 {
        (self.r[EAX] >> 8) as u8
    }
    pub fn al(&self) -> u8 {
        self.r[EAX] as u8
    }
    pub fn set_ax(&mut self, v: u16) {
        self.set_reg(EAX, 2, v as u32)
    }
    pub fn set_bx(&mut self, v: u16) {
        self.set_reg(EBX, 2, v as u32)
    }
    pub fn set_cx(&mut self, v: u16) {
        self.set_reg(ECX, 2, v as u32)
    }
    pub fn set_dx(&mut self, v: u16) {
        self.set_reg(EDX, 2, v as u32)
    }
    pub fn set_si(&mut self, v: u16) {
        self.set_reg(ESI, 2, v as u32)
    }
    pub fn set_di(&mut self, v: u16) {
        self.set_reg(EDI, 2, v as u32)
    }
    pub fn set_al(&mut self, v: u8) {
        self.set_reg(EAX, 1, v as u32)
    }
    pub fn set_ah(&mut self, v: u8) {
        self.set_reg(4, 1, v as u32)
    }

    #[inline(always)]
    pub fn flag(&self, f: u32) -> bool {
        self.flags & f != 0
    }

    #[inline(always)]
    pub fn set_flag(&mut self, f: u32, on: bool) {
        if on {
            self.flags |= f
        } else {
            self.flags &= !f
        }
    }

    // ---- memory helpers --------------------------------------------------------------------

    #[inline(always)]
    fn fetch8(&mut self) -> u8 {
        let v = self.mem.rb(lin(self.s[CS], self.ip));
        self.ip = (self.ip + 1) & 0xFFFF;
        v
    }

    #[inline(always)]
    fn fetch16(&mut self) -> u16 {
        let v = self.mem.rw(lin(self.s[CS], self.ip));
        self.ip = (self.ip + 2) & 0xFFFF;
        v
    }

    #[inline(always)]
    fn fetch32(&mut self) -> u32 {
        let v = self.mem.rd(lin(self.s[CS], self.ip));
        self.ip = (self.ip + 4) & 0xFFFF;
        v
    }

    #[inline(always)]
    fn fetch_sz(&mut self, sz: u8) -> u32 {
        match sz {
            1 => self.fetch8() as u32,
            2 => self.fetch16() as u32,
            _ => self.fetch32(),
        }
    }

    #[inline(always)]
    pub fn read(&self, seg: usize, off: u32, sz: u8) -> u32 {
        let a = lin(self.s[seg], off);
        match sz {
            1 => self.mem.rb(a) as u32,
            2 => self.mem.rw(a) as u32,
            _ => self.mem.rd(a),
        }
    }

    #[inline(always)]
    pub fn write(&mut self, seg: usize, off: u32, sz: u8, v: u32) {
        let a = lin(self.s[seg], off);
        match sz {
            1 => self.mem.wb(a, v as u8),
            2 => self.mem.ww(a, v as u16),
            _ => self.mem.wd(a, v),
        }
    }

    #[inline(always)]
    fn sp_mask(&self) -> u32 {
        0xFFFF
    }

    pub fn push(&mut self, sz: u8, v: u32) {
        let sp = self.r[ESP].wrapping_sub(sz as u32) & self.sp_mask();
        self.r[ESP] = (self.r[ESP] & !self.sp_mask()) | sp;
        self.write(SS, sp, sz, v);
    }

    pub fn pop(&mut self, sz: u8) -> u32 {
        let sp = self.r[ESP] & self.sp_mask();
        let v = self.read(SS, sp, sz);
        self.r[ESP] = (self.r[ESP] & !self.sp_mask()) | (sp.wrapping_add(sz as u32) & self.sp_mask());
        v
    }

    pub fn push16(&mut self, v: u16) {
        self.push(2, v as u32)
    }
    pub fn pop16(&mut self) -> u16 {
        self.pop(2) as u16
    }

    /// Deliver interrupt `n` through the real-mode IVT.
    pub fn interrupt(&mut self, n: u8) {
        let fl = self.flags;
        self.push16(fl as u16);
        let cs = self.s[CS];
        self.push16(cs);
        let ip = self.ip as u16;
        self.push16(ip);
        self.flags &= !(IF | TF);
        self.ip = self.mem.rw(n as u32 * 4) as u32;
        self.s[CS] = self.mem.rw(n as u32 * 4 + 2);
        self.halted = false;
    }

    /// Far call to `seg:off` with a return address of `ret_seg:ret_off` (host-initiated calls).
    pub fn far_call(&mut self, seg: u16, off: u16, ret_seg: u16, ret_off: u16) {
        self.push16(ret_seg);
        self.push16(ret_off);
        self.s[CS] = seg;
        self.ip = off as u32;
    }

    // ---- effective addresses ---------------------------------------------------------------

    fn modrm(&mut self) -> (usize, Ea) {
        let m = self.fetch8();
        let md = m >> 6;
        let reg = ((m >> 3) & 7) as usize;
        let rm = (m & 7) as usize;
        if md == 3 {
            return (reg, Ea { reg: Some(rm), seg: DS, off: 0 });
        }
        let (seg, off) = if !self.as32 {
            let (base, dseg) = match rm {
                0 => (self.r[EBX].wrapping_add(self.r[ESI]), DS),
                1 => (self.r[EBX].wrapping_add(self.r[EDI]), DS),
                2 => (self.r[EBP].wrapping_add(self.r[ESI]), SS),
                3 => (self.r[EBP].wrapping_add(self.r[EDI]), SS),
                4 => (self.r[ESI], DS),
                5 => (self.r[EDI], DS),
                6 => {
                    if md == 0 {
                        (self.fetch16() as u32, DS)
                    } else {
                        (self.r[EBP], SS)
                    }
                }
                _ => (self.r[EBX], DS),
            };
            let disp = match md {
                1 => self.fetch8() as i8 as i32 as u32,
                2 => self.fetch16() as u32,
                _ => 0,
            };
            (dseg, base.wrapping_add(disp) & 0xFFFF)
        } else {
            let mut dseg = DS;
            let base = if rm == 4 {
                let sib = self.fetch8();
                let scale = sib >> 6;
                let idx = ((sib >> 3) & 7) as usize;
                let b = (sib & 7) as usize;
                let index = if idx == 4 { 0 } else { self.r[idx] << scale };
                let bv = if b == 5 && md == 0 {
                    self.fetch32()
                } else {
                    if b == ESP || b == EBP {
                        dseg = SS;
                    }
                    self.r[b]
                };
                bv.wrapping_add(index)
            } else if rm == 5 && md == 0 {
                self.fetch32()
            } else {
                if rm == EBP {
                    dseg = SS;
                }
                self.r[rm]
            };
            let disp = match md {
                1 => self.fetch8() as i8 as i32 as u32,
                2 => self.fetch32(),
                _ => 0,
            };
            (dseg, base.wrapping_add(disp))
        };
        let seg = self.seg_ovr.unwrap_or(seg);
        (reg, Ea { reg: None, seg, off })
    }

    #[inline(always)]
    fn rd_ea(&self, ea: &Ea, sz: u8) -> u32 {
        match ea.reg {
            Some(r) => self.get_reg(r, sz),
            None => self.read(ea.seg, ea.off, sz),
        }
    }

    #[inline(always)]
    fn wr_ea(&mut self, ea: &Ea, sz: u8, v: u32) {
        match ea.reg {
            Some(r) => self.set_reg(r, sz, v),
            None => self.write(ea.seg, ea.off, sz, v),
        }
    }

    // ---- flags -----------------------------------------------------------------------------

    #[inline(always)]
    fn szp(&mut self, r: u32, sz: u8) {
        let r = r & mask(sz);
        let mut f = self.flags & !(ZF | SF | PF);
        if r == 0 {
            f |= ZF;
        }
        if r & sign(sz) != 0 {
            f |= SF;
        }
        if parity(r) {
            f |= PF;
        }
        self.flags = f;
    }

    fn add(&mut self, a: u32, b: u32, c: u32, sz: u8) -> u32 {
        let m = mask(sz);
        let full = (a & m) as u64 + (b & m) as u64 + c as u64;
        let r = full as u32 & m;
        self.set_flag(CF, full > m as u64);
        self.set_flag(OF, ((a ^ r) & (b ^ r) & sign(sz)) != 0);
        self.set_flag(AF, ((a ^ b ^ r) & 0x10) != 0);
        self.szp(r, sz);
        r
    }

    fn sub(&mut self, a: u32, b: u32, c: u32, sz: u8) -> u32 {
        let m = mask(sz);
        let (a, b) = (a & m, b & m);
        let r = a.wrapping_sub(b).wrapping_sub(c) & m;
        self.set_flag(CF, (b as u64 + c as u64) > a as u64);
        self.set_flag(OF, ((a ^ b) & (a ^ r) & sign(sz)) != 0);
        self.set_flag(AF, ((a ^ b ^ r) & 0x10) != 0);
        self.szp(r, sz);
        r
    }

    fn logic(&mut self, r: u32, sz: u8) -> u32 {
        let r = r & mask(sz);
        self.flags &= !(CF | OF | AF);
        self.szp(r, sz);
        r
    }

    fn alu(&mut self, op: usize, a: u32, b: u32, sz: u8) -> u32 {
        match op {
            0 => self.add(a, b, 0, sz),
            1 => self.logic(a | b, sz),
            2 => {
                let c = self.flags & CF;
                self.add(a, b, c, sz)
            }
            3 => {
                let c = self.flags & CF;
                self.sub(a, b, c, sz)
            }
            4 => self.logic(a & b, sz),
            5 => self.sub(a, b, 0, sz),
            6 => self.logic(a ^ b, sz),
            _ => {
                self.sub(a, b, 0, sz);
                a & mask(sz)
            }
        }
    }

    fn inc(&mut self, a: u32, sz: u8) -> u32 {
        let c = self.flags & CF;
        let r = self.add(a, 1, 0, sz);
        self.flags = (self.flags & !CF) | c;
        r
    }

    fn dec(&mut self, a: u32, sz: u8) -> u32 {
        let c = self.flags & CF;
        let r = self.sub(a, 1, 0, sz);
        self.flags = (self.flags & !CF) | c;
        r
    }

    /// Shift/rotate group (`C0`, `C1`, `D0`–`D3`). `op` is the ModRM reg field.
    fn shift(&mut self, op: usize, a: u32, cnt: u32, sz: u8) -> u32 {
        let cnt = cnt & 0x1F;
        if cnt == 0 {
            return a;
        }
        let bits = sz as u32 * 8;
        let m = mask(sz);
        let a = a & m;
        let sb = sign(sz);
        match op {
            0 => {
                // ROL
                let c = cnt % bits;
                let r = if c == 0 { a } else { ((a << c) | (a >> (bits - c))) & m };
                self.set_flag(CF, r & 1 != 0);
                self.set_flag(OF, ((r & sb != 0) as u32 ^ (r & 1)) != 0);
                r
            }
            1 => {
                // ROR
                let c = cnt % bits;
                let r = if c == 0 { a } else { ((a >> c) | (a << (bits - c))) & m };
                self.set_flag(CF, r & sb != 0);
                self.set_flag(OF, ((r ^ (r << 1)) & sb) != 0);
                r
            }
            2 => {
                // RCL
                let c = cnt % (bits + 1);
                let mut r = a as u64;
                let mut cf = self.flag(CF) as u64;
                for _ in 0..c {
                    let out = (r >> (bits - 1)) & 1;
                    r = ((r << 1) | cf) & m as u64;
                    cf = out;
                }
                let r = r as u32;
                self.set_flag(CF, cf != 0);
                self.set_flag(OF, ((r & sb != 0) as u64 ^ cf) != 0);
                r
            }
            3 => {
                // RCR
                let c = cnt % (bits + 1);
                let mut r = a as u64;
                let mut cf = self.flag(CF) as u64;
                let of = ((a & sb != 0) as u64) ^ cf;
                for _ in 0..c {
                    let out = r & 1;
                    r = (r >> 1) | (cf << (bits - 1));
                    cf = out;
                }
                self.set_flag(CF, cf != 0);
                self.set_flag(OF, of != 0);
                r as u32
            }
            4 | 6 => {
                // SHL/SAL
                let r = if cnt >= bits { 0 } else { (a << cnt) & m };
                let cf = if cnt > bits { false } else { (a >> (bits - cnt)) & 1 != 0 };
                self.set_flag(CF, cf);
                self.set_flag(OF, ((r & sb != 0) as u32 ^ cf as u32) != 0);
                self.szp(r, sz);
                r
            }
            5 => {
                // SHR
                let r = if cnt >= bits { 0 } else { a >> cnt };
                let cf = if cnt > bits { false } else { (a >> (cnt - 1)) & 1 != 0 };
                self.set_flag(CF, cf);
                self.set_flag(OF, a & sb != 0);
                self.szp(r, sz);
                r
            }
            _ => {
                // SAR
                let s = sext(a, sz);
                let c = cnt.min(bits - 1) as i64;
                let r = (s >> c) as u32 & m;
                let cf = if cnt >= bits { s < 0 } else { (s >> (cnt - 1)) & 1 != 0 };
                self.set_flag(CF, cf);
                self.set_flag(OF, false);
                self.szp(r, sz);
                r
            }
        }
    }

    fn cond(&self, c: u8) -> bool {
        let f = self.flags;
        let r = match c >> 1 {
            0 => f & OF != 0,
            1 => f & CF != 0,
            2 => f & ZF != 0,
            3 => f & (CF | ZF) != 0,
            4 => f & SF != 0,
            5 => f & PF != 0,
            6 => (f & SF != 0) != (f & OF != 0),
            _ => (f & ZF != 0) || ((f & SF != 0) != (f & OF != 0)),
        };
        r != (c & 1 != 0)
    }

    fn set_flags_word(&mut self, v: u32, sz: u8) {
        let keep = if sz == 4 { 0xFFFC_0000 & self.flags } else { self.flags & !0xFFFF };
        let v = if sz == 4 { v & 0x0003_7FD5 } else { v & 0x7FD5 };
        self.flags = keep | v | 0x0002;
    }

    // ---- control transfer --------------------------------------------------------------------

    fn jump_rel(&mut self, d: u32) {
        self.ip = self.ip.wrapping_add(d) & if self.os32 { 0xFFFF_FFFF } else { 0xFFFF };
    }

    fn divide_error(&mut self) {
        self.ip = self.start_ip;
        self.interrupt(0);
    }

    fn rep_count(&self) -> u32 {
        if self.as32 {
            self.r[ECX]
        } else {
            self.r[ECX] & 0xFFFF
        }
    }

    fn set_rep_count(&mut self, v: u32) {
        if self.as32 {
            self.r[ECX] = v
        } else {
            self.set_reg(ECX, 2, v)
        }
    }

    fn idx(&self, r: usize) -> u32 {
        if self.as32 {
            self.r[r]
        } else {
            self.r[r] & 0xFFFF
        }
    }

    fn adv(&mut self, r: usize, sz: u8) {
        let d = if self.flag(DF) { (sz as u32).wrapping_neg() } else { sz as u32 };
        if self.as32 {
            self.r[r] = self.r[r].wrapping_add(d);
        } else {
            let v = self.r[r].wrapping_add(d) & 0xFFFF;
            self.set_reg(r, 2, v);
        }
    }

    /// String instructions (`A4`–`AF`, `6C`–`6F`) with REP handling.
    fn string_op<I: Io>(&mut self, op: u8, io: &mut I) {
        let sz: u8 = if op & 1 == 0 { 1 } else if self.os32 { 4 } else { 2 };
        let src_seg = self.seg_ovr.unwrap_or(DS);
        let base = op & !1;
        if self.rep == 0 {
            self.string_once(base, sz, src_seg, io);
            return;
        }
        // Bounded so a long REP does not starve interrupts forever; the remaining count is
        // resumed by re-executing the instruction (as an interrupted REP does on hardware).
        let mut budget = 0x10000u32;
        while self.rep_count() != 0 {
            if budget == 0 {
                self.ip = self.start_ip;
                return;
            }
            budget -= 1;
            self.string_once(base, sz, src_seg, io);
            let c = self.rep_count().wrapping_sub(1);
            self.set_rep_count(c);
            if base == 0xA6 || base == 0xAE {
                let z = self.flag(ZF);
                if (self.rep == 0xF3 && !z) || (self.rep == 0xF2 && z) {
                    break;
                }
            }
        }
    }

    fn string_once<I: Io>(&mut self, base: u8, sz: u8, src_seg: usize, io: &mut I) {
        match base {
            0xA4 => {
                let v = self.read(src_seg, self.idx(ESI), sz);
                self.write(ES, self.idx(EDI), sz, v);
                self.adv(ESI, sz);
                self.adv(EDI, sz);
            }
            0xA6 => {
                let a = self.read(src_seg, self.idx(ESI), sz);
                let b = self.read(ES, self.idx(EDI), sz);
                self.sub(a, b, 0, sz);
                self.adv(ESI, sz);
                self.adv(EDI, sz);
            }
            0xAA => {
                let v = self.get_reg(EAX, sz);
                self.write(ES, self.idx(EDI), sz, v);
                self.adv(EDI, sz);
            }
            0xAC => {
                let v = self.read(src_seg, self.idx(ESI), sz);
                self.set_reg(EAX, sz, v);
                self.adv(ESI, sz);
            }
            0xAE => {
                let a = self.get_reg(EAX, sz);
                let b = self.read(ES, self.idx(EDI), sz);
                self.sub(a, b, 0, sz);
                self.adv(EDI, sz);
            }
            0x6C => {
                let p = self.dx();
                let v = match sz {
                    1 => io.inb(p) as u32,
                    _ => io.inw(p) as u32,
                };
                self.write(ES, self.idx(EDI), sz, v);
                self.adv(EDI, sz);
            }
            _ => {
                // 0x6E OUTS
                let p = self.dx();
                let v = self.read(src_seg, self.idx(ESI), sz);
                match sz {
                    1 => io.outb(p, v as u8),
                    _ => io.outw(p, v as u16),
                }
                self.adv(ESI, sz);
            }
        }
    }

    fn port_in<I: Io>(&mut self, io: &mut I, port: u16, sz: u8) {
        let v = match sz {
            1 => io.inb(port) as u32,
            2 => io.inw(port) as u32,
            _ => io.inw(port) as u32 | (io.inw(port.wrapping_add(2)) as u32) << 16,
        };
        self.set_reg(EAX, sz, v);
    }

    fn port_out<I: Io>(&mut self, io: &mut I, port: u16, sz: u8) {
        let v = self.get_reg(EAX, sz);
        match sz {
            1 => io.outb(port, v as u8),
            2 => io.outw(port, v as u16),
            _ => {
                io.outw(port, v as u16);
                io.outw(port.wrapping_add(2), (v >> 16) as u16);
            }
        }
    }

    // ---- multiply / divide -------------------------------------------------------------------

    fn mul_group(&mut self, op: usize, v: u32, sz: u8) -> bool {
        let m = mask(sz) as u64;
        let bits = sz as u32 * 8;
        match op {
            4 => {
                let a = self.get_reg(EAX, sz) as u64;
                let r = a * (v as u64 & m);
                let hi = (r >> bits) & m;
                self.store_wide(r, sz);
                self.set_flag(CF, hi != 0);
                self.set_flag(OF, hi != 0);
                self.szp(r as u32, sz);
            }
            5 => {
                let a = sext(self.get_reg(EAX, sz), sz);
                let r = a * sext(v, sz);
                self.store_wide(r as u64, sz);
                let fits = sext(r as u32 & m as u32, sz) == r;
                self.set_flag(CF, !fits);
                self.set_flag(OF, !fits);
                self.szp(r as u32, sz);
            }
            6 => {
                let d = v as u64 & m;
                if d == 0 {
                    self.divide_error();
                    return false;
                }
                let n = self.load_wide(sz);
                let q = n / d;
                if q > m {
                    self.divide_error();
                    return false;
                }
                self.store_div(q as u32, (n % d) as u32, sz);
            }
            _ => {
                let d = sext(v, sz);
                if d == 0 {
                    self.divide_error();
                    return false;
                }
                let n = match sz {
                    1 => self.r[EAX] as u16 as i16 as i64,
                    2 => ((self.dx() as u32) << 16 | self.ax() as u32) as i32 as i64,
                    _ => ((self.r[EDX] as u64) << 32 | self.r[EAX] as u64) as i64,
                };
                if n == i64::MIN && d == -1 {
                    self.divide_error();
                    return false;
                }
                let q = n.wrapping_div(d);
                let r = n.wrapping_rem(d);
                if sext(q as u32 & m as u32, sz) != q {
                    self.divide_error();
                    return false;
                }
                self.store_div(q as u32, r as u32, sz);
            }
        }
        true
    }

    fn store_wide(&mut self, r: u64, sz: u8) {
        match sz {
            1 => self.set_reg(EAX, 2, r as u32 & 0xFFFF),
            2 => {
                self.set_reg(EAX, 2, r as u32);
                self.set_reg(EDX, 2, (r >> 16) as u32);
            }
            _ => {
                self.r[EAX] = r as u32;
                self.r[EDX] = (r >> 32) as u32;
            }
        }
    }

    fn load_wide(&self, sz: u8) -> u64 {
        match sz {
            1 => self.ax() as u64,
            2 => (self.dx() as u64) << 16 | self.ax() as u64,
            _ => (self.r[EDX] as u64) << 32 | self.r[EAX] as u64,
        }
    }

    fn store_div(&mut self, q: u32, r: u32, sz: u8) {
        match sz {
            1 => {
                self.set_al(q as u8);
                self.set_ah(r as u8);
            }
            2 => {
                self.set_ax(q as u16);
                self.set_dx(r as u16);
            }
            _ => {
                self.r[EAX] = q;
                self.r[EDX] = r;
            }
        }
    }

    fn imul3(&mut self, a: u32, b: u32, sz: u8) -> u32 {
        let r = sext(a, sz) * sext(b, sz);
        let res = r as u32 & mask(sz);
        let fits = sext(res, sz) == r;
        self.set_flag(CF, !fits);
        self.set_flag(OF, !fits);
        self.szp(res, sz);
        res
    }

    fn load_far_ptr(&mut self, seg_reg: usize) {
        let (reg, ea) = self.modrm();
        let sz = if self.os32 { 4 } else { 2 };
        let off = self.read(ea.seg, ea.off, sz);
        let seg = self.read(ea.seg, ea.off.wrapping_add(sz as u32), 2) as u16;
        self.set_reg(reg, sz, off);
        self.s[seg_reg] = seg;
    }

    // ---- the interpreter -----------------------------------------------------------------------

    /// Execute one instruction (with its prefixes).
    pub fn step<I: Io>(&mut self, io: &mut I) -> Result<(), Event> {
        if self.breakpoint_count != 0 {
            let l = lin(self.s[CS], self.ip) & 0xF_FFFF;
            if self.breakpoints[(l >> 6) as usize] & (1 << (l & 63)) != 0 {
                if !std::mem::take(&mut self.skip_breakpoint) {
                    return Err(Event::Breakpoint(l));
                }
            }
        }
        self.seg_ovr = None;
        self.os32 = false;
        self.as32 = false;
        self.rep = 0;
        self.start_ip = self.ip;
        self.inhibit_irq = false;
        self.icount += 1;
        self.trace[self.trace_pos] = (self.s[CS], self.ip as u16);
        self.trace_pos = (self.trace_pos + 1) & 63;
        let mut op;
        loop {
            op = self.fetch8();
            match op {
                0x26 => self.seg_ovr = Some(ES),
                0x2E => self.seg_ovr = Some(CS),
                0x36 => self.seg_ovr = Some(SS),
                0x3E => self.seg_ovr = Some(DS),
                0x64 => self.seg_ovr = Some(FS),
                0x65 => self.seg_ovr = Some(GS),
                0x66 => self.os32 = true,
                0x67 => self.as32 = true,
                0xF0 => {}
                0xF2 | 0xF3 => self.rep = op,
                _ => break,
            }
        }
        let osz: u8 = if self.os32 { 4 } else { 2 };
        match op {
            // ALU r/m, reg and immediates
            0x00..=0x3F if op & 7 < 6 => {
                let aop = (op >> 3) as usize;
                match op & 7 {
                    0 | 1 => {
                        let sz = if op & 1 == 0 { 1 } else { osz };
                        let (reg, ea) = self.modrm();
                        let a = self.rd_ea(&ea, sz);
                        let b = self.get_reg(reg, sz);
                        let r = self.alu(aop, a, b, sz);
                        if aop != 7 {
                            self.wr_ea(&ea, sz, r);
                        }
                    }
                    2 | 3 => {
                        let sz = if op & 1 == 0 { 1 } else { osz };
                        let (reg, ea) = self.modrm();
                        let a = self.get_reg(reg, sz);
                        let b = self.rd_ea(&ea, sz);
                        let r = self.alu(aop, a, b, sz);
                        if aop != 7 {
                            self.set_reg(reg, sz, r);
                        }
                    }
                    4 => {
                        let b = self.fetch8() as u32;
                        let a = self.get_reg(EAX, 1);
                        let r = self.alu(aop, a, b, 1);
                        if aop != 7 {
                            self.set_reg(EAX, 1, r);
                        }
                    }
                    _ => {
                        let b = self.fetch_sz(osz);
                        let a = self.get_reg(EAX, osz);
                        let r = self.alu(aop, a, b, osz);
                        if aop != 7 {
                            self.set_reg(EAX, osz, r);
                        }
                    }
                }
            }
            0x06 | 0x0E | 0x16 | 0x1E => {
                let v = self.s[(op >> 3) as usize] as u32;
                self.push(osz, v);
            }
            0x07 | 0x17 | 0x1F => {
                let v = self.pop(osz) as u16;
                self.s[(op >> 3) as usize] = v;
                if op == 0x17 {
                    self.inhibit_irq = true;
                }
            }
            0x27 => {
                // DAA
                let al = self.al() as u32;
                let cf = self.flag(CF);
                let mut v = al;
                let mut ncf = false;
                if (al & 0xF) > 9 || self.flag(AF) {
                    v += 6;
                    ncf = cf || v > 0xFF;
                    self.set_flag(AF, true);
                } else {
                    self.set_flag(AF, false);
                }
                if al > 0x99 || cf {
                    v += 0x60;
                    ncf = true;
                }
                self.set_al(v as u8);
                self.set_flag(CF, ncf);
                self.szp(v & 0xFF, 1);
            }
            0x2F => {
                // DAS
                let al = self.al() as u32;
                let cf = self.flag(CF);
                let mut v = al;
                let mut ncf = false;
                if (al & 0xF) > 9 || self.flag(AF) {
                    v = v.wrapping_sub(6);
                    ncf = cf || al < 6;
                    self.set_flag(AF, true);
                } else {
                    self.set_flag(AF, false);
                }
                if al > 0x99 || cf {
                    v = v.wrapping_sub(0x60);
                    ncf = true;
                }
                self.set_al(v as u8);
                self.set_flag(CF, ncf);
                self.szp(v & 0xFF, 1);
            }
            0x37 | 0x3F => {
                // AAA / AAS
                if (self.al() & 0xF) > 9 || self.flag(AF) {
                    let ax = self.ax();
                    let nax = if op == 0x37 { ax.wrapping_add(0x106) } else { ax.wrapping_sub(6).wrapping_sub(0x100) };
                    self.set_ax(nax);
                    self.set_flag(AF, true);
                    self.set_flag(CF, true);
                } else {
                    self.set_flag(AF, false);
                    self.set_flag(CF, false);
                }
                let al = self.al() & 0xF;
                self.set_al(al);
            }
            0x40..=0x47 => {
                let r = (op & 7) as usize;
                let v = self.get_reg(r, osz);
                let n = self.inc(v, osz);
                self.set_reg(r, osz, n);
            }
            0x48..=0x4F => {
                let r = (op & 7) as usize;
                let v = self.get_reg(r, osz);
                let n = self.dec(v, osz);
                self.set_reg(r, osz, n);
            }
            0x50..=0x57 => {
                let v = self.get_reg((op & 7) as usize, osz);
                self.push(osz, v);
            }
            0x58..=0x5F => {
                let v = self.pop(osz);
                self.set_reg((op & 7) as usize, osz, v);
            }
            0x60 => {
                let sp = self.get_reg(ESP, osz);
                for i in 0..8 {
                    let v = if i == ESP { sp } else { self.get_reg(i, osz) };
                    self.push(osz, v);
                }
            }
            0x61 => {
                for i in (0..8).rev() {
                    let v = self.pop(osz);
                    if i != ESP {
                        self.set_reg(i, osz, v);
                    }
                }
            }
            0x62 => {
                // BOUND
                let (reg, ea) = self.modrm();
                let v = sext(self.get_reg(reg, osz), osz);
                let lo = sext(self.read(ea.seg, ea.off, osz), osz);
                let hi = sext(self.read(ea.seg, ea.off.wrapping_add(osz as u32), osz), osz);
                if v < lo || v > hi {
                    self.ip = self.start_ip;
                    self.interrupt(5);
                }
            }
            0x68 => {
                let v = self.fetch_sz(osz);
                self.push(osz, v);
            }
            0x6A => {
                let v = self.fetch8() as i8 as i32 as u32;
                self.push(osz, v);
            }
            0x69 | 0x6B => {
                let (reg, ea) = self.modrm();
                let a = self.rd_ea(&ea, osz);
                let b = if op == 0x69 { self.fetch_sz(osz) } else { self.fetch8() as i8 as i32 as u32 };
                let r = self.imul3(a, b, osz);
                self.set_reg(reg, osz, r);
            }
            0x6C..=0x6F => self.string_op(op, io),
            0x70..=0x7F => {
                let d = self.fetch8() as i8 as i32 as u32;
                if self.cond(op & 0xF) {
                    self.jump_rel(d);
                }
            }
            0x80..=0x83 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                let a = self.rd_ea(&ea, sz);
                let b = if op == 0x83 { self.fetch8() as i8 as i32 as u32 } else { self.fetch_sz(sz) };
                let r = self.alu(reg, a, b, sz);
                if reg != 7 {
                    self.wr_ea(&ea, sz, r);
                }
            }
            0x84 | 0x85 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                let a = self.rd_ea(&ea, sz);
                let b = self.get_reg(reg, sz);
                self.logic(a & b, sz);
            }
            0x86 | 0x87 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                let a = self.rd_ea(&ea, sz);
                let b = self.get_reg(reg, sz);
                self.wr_ea(&ea, sz, b);
                self.set_reg(reg, sz, a);
            }
            0x88..=0x8B => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                if op & 2 == 0 {
                    let v = self.get_reg(reg, sz);
                    self.wr_ea(&ea, sz, v);
                } else {
                    let v = self.rd_ea(&ea, sz);
                    self.set_reg(reg, sz, v);
                }
            }
            0x8C => {
                let (reg, ea) = self.modrm();
                let v = self.s[reg % 6] as u32;
                let sz = if ea.reg.is_some() { osz } else { 2 };
                self.wr_ea(&ea, sz, v);
            }
            0x8D => {
                let (reg, ea) = self.modrm();
                self.set_reg(reg, osz, ea.off);
            }
            0x8E => {
                let (reg, ea) = self.modrm();
                let v = self.rd_ea(&ea, 2) as u16;
                self.s[reg % 6] = v;
                if reg == SS {
                    self.inhibit_irq = true;
                }
            }
            0x8F => {
                let v = self.pop(osz);
                let (_, ea) = self.modrm();
                self.wr_ea(&ea, osz, v);
            }
            0x90 => {}
            0x91..=0x97 => {
                let r = (op & 7) as usize;
                let a = self.get_reg(EAX, osz);
                let b = self.get_reg(r, osz);
                self.set_reg(EAX, osz, b);
                self.set_reg(r, osz, a);
            }
            0x98 => {
                if self.os32 {
                    self.r[EAX] = self.r[EAX] as u16 as i16 as i32 as u32;
                } else {
                    let v = self.al() as i8 as i16 as u16;
                    self.set_ax(v);
                }
            }
            0x99 => {
                if self.os32 {
                    self.r[EDX] = if self.r[EAX] & 0x8000_0000 != 0 { 0xFFFF_FFFF } else { 0 };
                } else {
                    let v = if self.ax() & 0x8000 != 0 { 0xFFFF } else { 0 };
                    self.set_dx(v);
                }
            }
            0x9A => {
                let off = self.fetch_sz(osz);
                let seg = self.fetch16();
                let cs = self.s[CS] as u32;
                self.push(osz, cs);
                let ip = self.ip;
                self.push(osz, ip);
                self.s[CS] = seg;
                self.ip = off;
            }
            0x9B => {}
            0x9C => {
                let f = self.flags & if self.os32 { 0x0003_7FD7 } else { 0xFFFF };
                self.push(osz, f);
            }
            0x9D => {
                let v = self.pop(osz);
                self.set_flags_word(v, osz);
            }
            0x9E => {
                let ah = self.ah() as u32;
                self.flags = (self.flags & !0xD5) | (ah & 0xD5) | 2;
            }
            0x9F => {
                let f = (self.flags & 0xD7) as u8 | 2;
                self.set_ah(f);
            }
            0xA0..=0xA3 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let off = if self.as32 { self.fetch32() } else { self.fetch16() as u32 };
                let seg = self.seg_ovr.unwrap_or(DS);
                if op & 2 == 0 {
                    let v = self.read(seg, off, sz);
                    self.set_reg(EAX, sz, v);
                } else {
                    let v = self.get_reg(EAX, sz);
                    self.write(seg, off, sz, v);
                }
            }
            0xA4..=0xA7 | 0xAA..=0xAF => self.string_op(op, io),
            0xA8 => {
                let b = self.fetch8() as u32;
                let a = self.get_reg(EAX, 1);
                self.logic(a & b, 1);
            }
            0xA9 => {
                let b = self.fetch_sz(osz);
                let a = self.get_reg(EAX, osz);
                self.logic(a & b, osz);
            }
            0xB0..=0xB7 => {
                let v = self.fetch8() as u32;
                self.set_reg((op & 7) as usize, 1, v);
            }
            0xB8..=0xBF => {
                let v = self.fetch_sz(osz);
                self.set_reg((op & 7) as usize, osz, v);
            }
            0xC0 | 0xC1 | 0xD0..=0xD3 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                let a = self.rd_ea(&ea, sz);
                let cnt = match op {
                    0xC0 | 0xC1 => self.fetch8() as u32,
                    0xD0 | 0xD1 => 1,
                    _ => self.r[ECX] & 0xFF,
                };
                let r = self.shift(reg, a, cnt, sz);
                self.wr_ea(&ea, sz, r);
            }
            0xC2 | 0xC3 => {
                let n = if op == 0xC2 { self.fetch16() as u32 } else { 0 };
                self.ip = self.pop(osz);
                let sp = (self.r[ESP].wrapping_add(n)) & 0xFFFF;
                self.set_reg(ESP, 2, sp);
            }
            0xC4 => self.load_far_ptr(ES),
            0xC5 => self.load_far_ptr(DS),
            0xC6 | 0xC7 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (_, ea) = self.modrm();
                let v = self.fetch_sz(sz);
                self.wr_ea(&ea, sz, v);
            }
            0xC8 => {
                let size = self.fetch16() as u32;
                let level = (self.fetch8() & 31) as u32;
                let bp = self.get_reg(EBP, osz);
                self.push(osz, bp);
                let frame = self.get_reg(ESP, osz);
                let mut bpv = bp;
                for _ in 1..level {
                    bpv = bpv.wrapping_sub(osz as u32);
                    let v = self.read(SS, bpv & 0xFFFF, osz);
                    self.push(osz, v);
                }
                if level > 0 {
                    self.push(osz, frame);
                }
                self.set_reg(EBP, osz, frame);
                let sp = self.r[ESP].wrapping_sub(size) & 0xFFFF;
                self.set_reg(ESP, 2, sp);
            }
            0xC9 => {
                let bp = self.r[EBP] & 0xFFFF;
                self.set_reg(ESP, 2, bp);
                let v = self.pop(osz);
                self.set_reg(EBP, osz, v);
            }
            0xCA | 0xCB => {
                let n = if op == 0xCA { self.fetch16() as u32 } else { 0 };
                self.ip = self.pop(osz);
                self.s[CS] = self.pop(osz) as u16;
                let sp = (self.r[ESP].wrapping_add(n)) & 0xFFFF;
                self.set_reg(ESP, 2, sp);
            }
            0xCC => self.interrupt(3),
            0xCD => {
                let n = self.fetch8();
                self.interrupt(n);
            }
            0xCE => {
                if self.flag(OF) {
                    self.interrupt(4);
                }
            }
            0xCF => {
                self.ip = self.pop(osz);
                self.s[CS] = self.pop(osz) as u16;
                let f = self.pop(osz);
                self.set_flags_word(f, osz);
            }
            0xD4 => {
                let b = self.fetch8();
                if b == 0 {
                    self.divide_error();
                } else {
                    let al = self.al();
                    self.set_ah(al / b);
                    self.set_al(al % b);
                    self.szp(self.al() as u32, 1);
                }
            }
            0xD5 => {
                let b = self.fetch8();
                let v = self.al().wrapping_add(self.ah().wrapping_mul(b));
                self.set_ax(v as u16);
                self.szp(v as u32, 1);
            }
            0xD6 => {
                let v = if self.flag(CF) { 0xFF } else { 0 };
                self.set_al(v);
            }
            0xD7 => {
                let seg = self.seg_ovr.unwrap_or(DS);
                let off = self.idx(EBX).wrapping_add(self.al() as u32) & if self.as32 { !0 } else { 0xFFFF };
                let v = self.read(seg, off, 1);
                self.set_al(v as u8);
            }
            0xD8..=0xDF => {
                // No x87 present: decode the operand and do nothing (as with no coprocessor
                // and EM clear the bus would simply not respond).
                let _ = self.modrm();
            }
            0xE0..=0xE3 => {
                let d = self.fetch8() as i8 as i32 as u32;
                let take = if op == 0xE3 {
                    self.rep_count() == 0
                } else {
                    let c = self.rep_count().wrapping_sub(1);
                    self.set_rep_count(c);
                    let c = c & if self.as32 { !0 } else { 0xFFFF };
                    match op {
                        0xE0 => c != 0 && !self.flag(ZF),
                        0xE1 => c != 0 && self.flag(ZF),
                        _ => c != 0,
                    }
                };
                if take {
                    self.jump_rel(d);
                }
            }
            0xE4 | 0xE5 => {
                let p = self.fetch8() as u16;
                self.port_in(io, p, if op == 0xE4 { 1 } else { osz });
            }
            0xE6 | 0xE7 => {
                let p = self.fetch8() as u16;
                self.port_out(io, p, if op == 0xE6 { 1 } else { osz });
            }
            0xE8 => {
                let d = if self.os32 { self.fetch32() } else { self.fetch16() as i16 as i32 as u32 };
                let ip = self.ip;
                self.push(osz, ip);
                self.jump_rel(d);
            }
            0xE9 => {
                let d = if self.os32 { self.fetch32() } else { self.fetch16() as i16 as i32 as u32 };
                self.jump_rel(d);
            }
            0xEA => {
                let off = self.fetch_sz(osz);
                let seg = self.fetch16();
                self.s[CS] = seg;
                self.ip = off;
            }
            0xEB => {
                let d = self.fetch8() as i8 as i32 as u32;
                self.jump_rel(d);
            }
            0xEC | 0xED => {
                let p = self.dx();
                self.port_in(io, p, if op == 0xEC { 1 } else { osz });
            }
            0xEE | 0xEF => {
                let p = self.dx();
                self.port_out(io, p, if op == 0xEE { 1 } else { osz });
            }
            0xF4 => {
                self.halted = true;
                return Err(Event::Halt);
            }
            0xF5 => self.flags ^= CF,
            0xF6 | 0xF7 => {
                let sz = if op & 1 == 0 { 1 } else { osz };
                let (reg, ea) = self.modrm();
                let v = self.rd_ea(&ea, sz);
                match reg {
                    0 | 1 => {
                        let b = self.fetch_sz(sz);
                        self.logic(v & b, sz);
                    }
                    2 => self.wr_ea(&ea, sz, !v),
                    3 => {
                        let r = self.sub(0, v, 0, sz);
                        self.set_flag(CF, v & mask(sz) != 0);
                        self.wr_ea(&ea, sz, r);
                    }
                    _ => {
                        self.mul_group(reg, v, sz);
                    }
                }
            }
            0xF8 => self.flags &= !CF,
            0xF9 => self.flags |= CF,
            0xFA => self.flags &= !IF,
            0xFB => {
                self.flags |= IF;
                self.inhibit_irq = true;
            }
            0xFC => self.flags &= !DF,
            0xFD => self.flags |= DF,
            0xFE => {
                let (reg, ea) = self.modrm();
                let v = self.rd_ea(&ea, 1);
                let r = match reg {
                    0 => self.inc(v, 1),
                    1 => self.dec(v, 1),
                    _ => return Err(self.unimpl(0xFE00 | reg as u16)),
                };
                self.wr_ea(&ea, 1, r);
            }
            0xFF => {
                let (reg, ea) = self.modrm();
                match reg {
                    0 | 1 => {
                        let v = self.rd_ea(&ea, osz);
                        let r = if reg == 0 { self.inc(v, osz) } else { self.dec(v, osz) };
                        self.wr_ea(&ea, osz, r);
                    }
                    2 => {
                        let t = self.rd_ea(&ea, osz);
                        let ip = self.ip;
                        self.push(osz, ip);
                        self.ip = t;
                    }
                    3 => {
                        let off = self.read(ea.seg, ea.off, osz);
                        let seg = self.read(ea.seg, ea.off.wrapping_add(osz as u32), 2) as u16;
                        let cs = self.s[CS] as u32;
                        self.push(osz, cs);
                        let ip = self.ip;
                        self.push(osz, ip);
                        self.s[CS] = seg;
                        self.ip = off;
                    }
                    4 => self.ip = self.rd_ea(&ea, osz),
                    5 => {
                        let off = self.read(ea.seg, ea.off, osz);
                        let seg = self.read(ea.seg, ea.off.wrapping_add(osz as u32), 2) as u16;
                        self.s[CS] = seg;
                        self.ip = off;
                    }
                    6 => {
                        let v = self.rd_ea(&ea, osz);
                        self.push(osz, v);
                    }
                    _ => return Err(self.unimpl(0xFF07)),
                }
            }
            0x0F => return self.step_0f(),
            _ => return Err(self.unimpl(op as u16)),
        }
        Ok(())
    }

    fn unimpl(&mut self, op: u16) -> Event {
        let ev = Event::Unimplemented { cs: self.s[CS], ip: self.start_ip as u16, op };
        self.ip = self.start_ip;
        ev
    }

    fn step_0f(&mut self) -> Result<(), Event> {
        let osz: u8 = if self.os32 { 4 } else { 2 };
        let op = self.fetch8();
        match op {
            0xFF => {
                let n = self.fetch8();
                return Err(Event::Hle(n));
            }
            0x80..=0x8F => {
                let d = if self.os32 { self.fetch32() } else { self.fetch16() as i16 as i32 as u32 };
                if self.cond(op & 0xF) {
                    self.jump_rel(d);
                }
            }
            0x90..=0x9F => {
                let (_, ea) = self.modrm();
                let v = self.cond(op & 0xF) as u32;
                self.wr_ea(&ea, 1, v);
            }
            0xA0 | 0xA8 => {
                let v = self.s[if op == 0xA0 { FS } else { GS }] as u32;
                self.push(osz, v);
            }
            0xA1 | 0xA9 => {
                let v = self.pop(osz) as u16;
                self.s[if op == 0xA1 { FS } else { GS }] = v;
            }
            0xA3 | 0xAB | 0xB3 | 0xBB => {
                let (reg, ea) = self.modrm();
                let bit = self.get_reg(reg, osz);
                let kind = (op >> 3) & 3; // 0 bt, 1 bts, 2 btr, 3 btc
                self.bit_op(kind, ea, bit, true, osz);
            }
            0xBA => {
                let (reg, ea) = self.modrm();
                let bit = self.fetch8() as u32;
                if reg < 4 {
                    return Err(self.unimpl(0x0FBA));
                }
                self.bit_op((reg - 4) as u8, ea, bit, false, osz);
            }
            0xA4 | 0xA5 | 0xAC | 0xAD => {
                let (reg, ea) = self.modrm();
                let cnt = if op & 1 == 0 { self.fetch8() as u32 } else { self.r[ECX] & 0xFF } & 31;
                if cnt != 0 {
                    let bits = osz as u32 * 8;
                    let a = self.rd_ea(&ea, osz) as u64;
                    let b = self.get_reg(reg, osz) as u64;
                    let m = mask(osz);
                    let r = if op < 0xAC {
                        // SHLD
                        let wide = (a << bits) | b;
                        let r = ((wide << cnt) >> bits) as u32 & m;
                        self.set_flag(CF, (a >> (bits - cnt)) & 1 != 0);
                        r
                    } else {
                        let wide = (b << bits) | a;
                        let r = (wide >> cnt) as u32 & m;
                        self.set_flag(CF, (a >> (cnt - 1)) & 1 != 0);
                        r
                    };
                    self.set_flag(OF, ((r ^ a as u32) & sign(osz)) != 0);
                    self.szp(r, osz);
                    self.wr_ea(&ea, osz, r);
                }
            }
            0xAF => {
                let (reg, ea) = self.modrm();
                let a = self.get_reg(reg, osz);
                let b = self.rd_ea(&ea, osz);
                let r = self.imul3(a, b, osz);
                self.set_reg(reg, osz, r);
            }
            0xB2 => self.load_far_ptr(SS),
            0xB4 => self.load_far_ptr(FS),
            0xB5 => self.load_far_ptr(GS),
            0xB6 | 0xB7 | 0xBE | 0xBF => {
                let (reg, ea) = self.modrm();
                let ssz = if op & 1 == 0 { 1 } else { 2 };
                let v = self.rd_ea(&ea, ssz);
                let v = if op >= 0xBE { sext(v, ssz) as u32 } else { v };
                self.set_reg(reg, osz, v);
            }
            0xBC | 0xBD => {
                let (reg, ea) = self.modrm();
                let v = self.rd_ea(&ea, osz);
                if v == 0 {
                    self.flags |= ZF;
                } else {
                    self.flags &= !ZF;
                    let i = if op == 0xBC { v.trailing_zeros() } else { 31 - v.leading_zeros() };
                    self.set_reg(reg, osz, i);
                }
            }
            0xC8..=0xCF => {
                let r = (op & 7) as usize;
                self.r[r] = self.r[r].swap_bytes();
            }
            0x01 => {
                // SMSW only (MSW = 0: real mode, no coprocessor emulation).
                let (reg, ea) = self.modrm();
                if reg == 4 {
                    self.wr_ea(&ea, 2, 0);
                } else {
                    return Err(self.unimpl(0x0F01));
                }
            }
            _ => return Err(self.unimpl(0x0F00 | op as u16)),
        }
        Ok(())
    }

    fn bit_op(&mut self, kind: u8, ea: Ea, bit: u32, reg_offset: bool, osz: u8) {
        let bits = osz as u32 * 8;
        let (val, ea2) = match ea.reg {
            Some(_) => (self.rd_ea(&ea, osz), ea),
            None => {
                // A register bit offset may address beyond the operand (signed).
                let mut e = ea;
                if reg_offset {
                    let sb = sext(bit, osz);
                    let byte_off = (sb.div_euclid(bits as i64)) * (osz as i64);
                    e.off = e.off.wrapping_add(byte_off as u32) & if self.as32 { !0 } else { 0xFFFF };
                }
                (self.read(e.seg, e.off, osz), e)
            }
        };
        let b = bit % bits;
        let m = 1u32 << b;
        self.set_flag(CF, val & m != 0);
        let nv = match kind {
            1 => val | m,
            2 => val & !m,
            3 => val ^ m,
            _ => return,
        };
        self.wr_ea(&ea2, osz, nv);
    }

    /// Set or clear the breakpoint at linear address `l`.
    pub fn set_breakpoint(&mut self, l: u32, on: bool) {
        let l = l & 0xF_FFFF;
        let (w, b) = ((l >> 6) as usize, 1u64 << (l & 63));
        let was = self.breakpoints[w] & b != 0;
        if on && !was {
            self.breakpoints[w] |= b;
            self.breakpoint_count += 1;
        } else if !on && was {
            self.breakpoints[w] &= !b;
            self.breakpoint_count -= 1;
        }
    }

    /// Return from a far call made by the original code (`RETF`).
    pub fn retf(&mut self) {
        self.ip = self.pop16() as u32;
        self.s[CS] = self.pop16();
    }

    /// The most recent `cs:ip` values, oldest first.
    pub fn recent(&self) -> Vec<(u16, u16)> {
        (0..64).map(|i| self.trace[(self.trace_pos + i) & 63]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoIo;
    impl Io for NoIo {
        fn inb(&mut self, _: u16) -> u8 {
            0xFF
        }
        fn outb(&mut self, _: u16, _: u8) {}
    }

    /// Run `code` at 0100:0000 until it reaches the end.
    fn run(code: &[u8], setup: impl FnOnce(&mut Cpu)) -> Cpu {
        let mut c = Cpu::new(Memory::new(0));
        c.mem.write_bytes(0x1000, code);
        c.s = [0, 0x100, 0x200, 0x300, 0, 0];
        c.r[ESP] = 0xFFFE;
        setup(&mut c);
        while (c.ip as usize) < code.len() {
            c.step(&mut NoIo).unwrap();
        }
        c
    }

    #[test]
    fn add_sets_carry_overflow() {
        // add al, 0x80 with al = 0x80
        let c = run(&[0x04, 0x80], |c| c.r[EAX] = 0x80);
        assert_eq!(c.al(), 0);
        assert!(c.flag(CF) && c.flag(OF) && c.flag(ZF));
    }

    #[test]
    fn operand_size_prefix_uses_32_bits() {
        // mov eax, 0x12345678; add eax, 0x0000_0001 (66 05 imm32)
        let c = run(&[0x66, 0xB8, 0x78, 0x56, 0x34, 0x12, 0x66, 0x05, 1, 0, 0, 0], |_| {});
        assert_eq!(c.r[EAX], 0x1234_5679);
    }

    #[test]
    fn signed_divide_and_shifts() {
        // mov ax,-7 ; cwd ; mov bx,2 ; idiv bx ; sar dx,1? -> check quotient/remainder
        let c = run(&[0xB8, 0xF9, 0xFF, 0x99, 0xBB, 2, 0, 0xF7, 0xFB], |_| {});
        assert_eq!(c.ax() as i16, -3);
        assert_eq!(c.dx() as i16, -1);
        // shld eax, ebx, 4 with eax = 0x1000_0001, ebx = 0xF000_0000
        let c = run(&[0x66, 0x0F, 0xA4, 0xD8, 4], |c| {
            c.r[EAX] = 0x1000_0001;
            c.r[EBX] = 0xF000_0000;
        });
        assert_eq!(c.r[EAX], 0x0000_001F);
        assert!(c.flag(CF));
    }

    #[test]
    fn rep_movsb_copies_forward() {
        let c = run(&[0xF3, 0xA4], |c| {
            c.mem.write_bytes(0x3000, b"hello");
            c.s[ES] = 0x400;
            c.r[ECX] = 5;
        });
        let mut out = [0u8; 5];
        c.mem.read_bytes(0x4000, &mut out);
        assert_eq!(&out, b"hello");
        assert_eq!(c.cx(), 0);
    }
}
