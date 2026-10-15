//! Port of `EL2ADLIB.OVL`, the game's AdLib / Sound Blaster music and effects driver
//! (docs/AUDIO.md).
//!
//! The driver is a byte-code sequencer for nine OPL2 voices, ticked at 202.27 Hz from IRQ 0 and
//! stepping its voices on 327 of every 910 ticks. Its music, instruments and tables are data
//! inside the overlay, read from the user's file at run time; nothing of it is in this crate.
//!
//! The port is 1:1 at the level of the original's state: every variable lives where the
//! original keeps it (the overlay's own segment, plus a few words of the main data segment), so
//! the original and the port can take turns on the same memory and be compared byte for byte.
//! Registers that survive between routines are modelled, the instruction count of every path is
//! charged (so the game's timing is the original's), and every OPL register write is reported
//! with the instruction count at which the original executes its `OUT`. Parity with the original
//! is proven by `crates/hooks/tests/adlib_parity.rs`.

/// Memory and ports as the driver sees them.
pub trait Host {
    fn read8(&mut self, a: u32) -> u8;
    fn write8(&mut self, a: u32, v: u8);
    /// `OUT port, v`, executed after `steps` instructions of the current call.
    fn out8(&mut self, steps: u64, port: u16, v: u8);
}

pub fn lin(seg: u16, off: u16) -> u32 {
    ((seg as u32) << 4) + off as u32
}

/// The 16-bit registers the driver uses. Exports either preserve them (`PUSHAD`/`POPAD`) or
/// leave what the original leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
}

/// The 11 exports: (overlay offset, main vector slot, name, routine).
pub const EXPORTS: &[(u16, u16, &str, fn(&mut Driver))] = &[
    (0x0152, 0x032c, "engine", |d| d.engine()),
    (0x01b6, 0x0330, "stop_music_unless_jukebox", |d| d.stop_music_unless_jukebox()),
    (0x01c0, 0x0334, "init", |d| d.init()),
    (0x01d2, 0x0338, "play_song", |d| d.play_song()),
    (0x022a, 0x033c, "shutdown", |d| d.shutdown()),
    (0x0233, 0x0340, "effect", |d| d.effect()),
    (0x0280, 0x0344, "effect_at_range", |d| d.effect_at_range()),
    (0x025b, 0x0348, "effect_with_volume", |d| d.effect_with_volume()),
    (0x029e, 0x034c, "tick", |d| d.tick()),
    (0x033b, 0x0350, "silence", |d| d.silence_all()),
    (0x036f, 0x0354, "stop_music", |d| d.stop_music()),
];

// Offsets in the driver's segment.
const BUSY: u16 = 0x0092;
const DIVIDER: u16 = 0x0093;
const EFFECT_MAP: u16 = 0x0095;
const EFFECT_TIMERS: [(u16, u16); 5] = [(0x00c3, 0x22), (0x00bd, 0x19), (0x00bf, 0x0e), (0x00c1, 0x0c), (0x00c5, 0x1e)];
const RANGE_VOLUME: u16 = 0x00c7;
const PITCH_TABLES: u16 = 0x0382;
const LAST_DURATION: u16 = 0x0562;
const VOICE: u16 = 0x0565;
const ACTIVE: u16 = 0x0566;
const RHYTHM_KEYS: u16 = 0x0567;
const TEMPO: u16 = 0x0568;
const SEED: u16 = 0x056b;
const DEPTH_BITS: u16 = 0x056e;
const OPERATOR: u16 = 0x056f;
const COMMANDS: u16 = 0x0588;
const QUEUE_WRITE: u16 = 0x0598;
const QUEUE_READ: u16 = 0x059a;
const QUEUE: u16 = 0x059c;
const VOICE_RECORDS: u16 = 0x084a;
const OPERATOR_OF_VOICE: u16 = 0x085e;
const F_NUMBERS: u16 = 0x0867;
const COMMAND_TABLE: u16 = 0x1871;
const INSTRUMENTS: u16 = 0x1873;
const OPCODES: u16 = 0x17dd;

// Per-voice callbacks (near pointers stored in the voice record).
const CB_NONE: u16 = 0x169c;
const CB_VIBRATO: u16 = 0x169d;
const CB_LFO: u16 = 0x1708;
const CB_SLIDE: u16 = 0x173c;

// Main data segment.
const DS_EFFECTS_ON: u16 = 0x4636;
const DS_EFFECT_14_ON: u16 = 0x463a;
const DS_MUSIC_ON: u16 = 0x463b;
const DS_JUKEBOX: u16 = 0x463f;
const DS_SONG_ENABLED: u16 = 0x4640;
const DS_SONG: u16 = 0x53a3;
const DS_RANGE_ACTIVE: u16 = 0x5709;
const DS_SOUND: u16 = 0x8920;
const DS_SEED: u16 = 0x1fd9;

/// Instructions in `EL2ADLIB:1472` (one register write with its two delay loops), counted
/// from its first instruction to its `RET`.
const WRITE_STEPS: u64 = 81;

/// Where the byte-code interpreter continues.
#[derive(Clone, Copy)]
enum Next {
    Callbacks,
    NextVoice,
}

pub struct Driver<'a> {
    h: &'a mut dyn Host,
    cs: u16,
    ds: u16,
    pub r: Regs,
    /// Instructions the original executes for this call, including its `RETF`.
    pub steps: u64,
    /// Byte-code ops executed in this call (bit n = op n), for coverage.
    pub ops: u128,
}

impl<'a> Driver<'a> {
    pub fn new(h: &'a mut dyn Host, cs: u16, ds: u16, r: Regs) -> Self {
        Driver { h, cs, ds, r, steps: 0, ops: 0 }
    }

    // ---- memory -------------------------------------------------------------------------

    fn cb(&mut self, off: u16) -> u8 {
        self.h.read8(lin(self.cs, off))
    }
    fn cw(&mut self, off: u16) -> u16 {
        self.cb(off) as u16 | (self.cb(off.wrapping_add(1)) as u16) << 8
    }
    fn scb(&mut self, off: u16, v: u8) {
        self.h.write8(lin(self.cs, off), v)
    }
    fn scw(&mut self, off: u16, v: u16) {
        self.scb(off, v as u8);
        self.scb(off.wrapping_add(1), (v >> 8) as u8);
    }
    fn db(&mut self, off: u16) -> u8 {
        self.h.read8(lin(self.ds, off))
    }
    fn dw(&mut self, off: u16) -> u16 {
        self.db(off) as u16 | (self.db(off.wrapping_add(1)) as u16) << 8
    }
    fn sdb(&mut self, off: u16, v: u8) {
        self.h.write8(lin(self.ds, off), v)
    }
    fn sdw(&mut self, off: u16, v: u16) {
        self.sdb(off, v as u8);
        self.sdb(off.wrapping_add(1), (v >> 8) as u8);
    }
    /// A byte of the current voice record.
    fn vb(&mut self, f: u16) -> u8 {
        let a = self.r.di.wrapping_add(f);
        self.cb(a)
    }
    fn vw(&mut self, f: u16) -> u16 {
        let a = self.r.di.wrapping_add(f);
        self.cw(a)
    }
    fn svb(&mut self, f: u16, v: u8) {
        let a = self.r.di.wrapping_add(f);
        self.scb(a, v)
    }
    fn svw(&mut self, f: u16, v: u16) {
        let a = self.r.di.wrapping_add(f);
        self.scw(a, v)
    }
    fn lodsb(&mut self) -> u8 {
        let v = self.cb(self.r.si);
        self.r.si = self.r.si.wrapping_add(1);
        v
    }
    fn lodsw(&mut self) -> u16 {
        let v = self.cw(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        v
    }

    // ---- byte registers -----------------------------------------------------------------

    fn al(&self) -> u8 {
        self.r.ax as u8
    }
    fn ah(&self) -> u8 {
        (self.r.ax >> 8) as u8
    }
    fn set_al(&mut self, v: u8) {
        self.r.ax = (self.r.ax & 0xFF00) | v as u16;
    }
    fn set_ah(&mut self, v: u8) {
        self.r.ax = (self.r.ax & 0x00FF) | (v as u16) << 8;
    }
    fn bl(&self) -> u8 {
        self.r.bx as u8
    }
    fn bh(&self) -> u8 {
        (self.r.bx >> 8) as u8
    }
    fn set_bl(&mut self, v: u8) {
        self.r.bx = (self.r.bx & 0xFF00) | v as u16;
    }
    fn set_bh(&mut self, v: u8) {
        self.r.bx = (self.r.bx & 0x00FF) | (v as u16) << 8;
    }
    fn cl(&self) -> u8 {
        self.r.cx as u8
    }
    fn ch(&self) -> u8 {
        (self.r.cx >> 8) as u8
    }
    fn set_cl(&mut self, v: u8) {
        self.r.cx = (self.r.cx & 0xFF00) | v as u16;
    }
    fn set_ch(&mut self, v: u8) {
        self.r.cx = (self.r.cx & 0x00FF) | (v as u16) << 8;
    }

    fn voice(&mut self) -> u8 {
        self.cb(VOICE)
    }

    // ---- low-level routines -------------------------------------------------------------

    /// Port of `EL2ADLIB:1472`: OPL register `AH` ← `AL` through `388h`/`389h`, with the
    /// original's 4- and 23-read delays. `AX` is preserved, `DX` = `389h`. The caller's `CALL`
    /// is counted by the caller.
    fn write_reg(&mut self) {
        let (reg, val) = (self.ah(), self.al());
        self.h.out8(self.steps + 2, 0x388, reg);
        self.h.out8(self.steps + 23, 0x389, val);
        self.r.dx = 0x389;
        self.steps += WRITE_STEPS;
    }

    /// `CALL 1472` with `AH`, `AL`.
    fn call_write(&mut self, reg: u8, val: u8) {
        self.set_ah(reg);
        self.set_al(val);
        self.steps += 1;
        self.write_reg();
    }

    /// Port of `EL2ADLIB:089f`: append command `AL` to the 16-entry queue. `BX` = new index.
    fn queue(&mut self) {
        let bx = self.cw(QUEUE_WRITE);
        let al = self.al();
        self.scb(QUEUE.wrapping_add(bx), al);
        let bx = bx.wrapping_add(1) & 0xF;
        self.scw(QUEUE_WRITE, bx);
        self.r.bx = bx;
        self.steps += 6;
    }

    /// Port of `EL2ADLIB:17cb`: the driver's 16-bit pseudo-random step (`+9248h`, rotate right
    /// 3). Result in `AX`.
    fn random(&mut self) {
        let ax = self.cw(SEED).wrapping_add(0x9248).rotate_right(3);
        self.scw(SEED, ax);
        self.r.ax = ax;
        self.steps += 7;
    }

    /// Port of `EL2ADLIB:159b`: clear a voice record (`DI+3` … `DI+42h`), then priority 0,
    /// tempo accumulator `FFh`, both callbacks off, second key-off point 1.
    fn clear_voice(&mut self) {
        for i in 0..0x40 {
            self.svb(3 + i, 0);
        }
        self.svb(0x25, 0xFF);
        self.svb(0x08, 0);
        self.svw(0x29, CB_NONE);
        self.svw(0x2b, CB_NONE);
        self.svb(0x35, 1);
        self.steps += 23;
    }

    /// Port of `EL2ADLIB:0920`: reset the chip and every voice.
    fn reset(&mut self) {
        self.scb(ACTIVE, 0);
        self.scw(SEED, 0x1234);
        self.steps += 2;
        self.steps += 1;
        self.call_write(0x01, 0x20);
        self.steps += 1;
        self.call_write(0x08, 0x00);
        self.steps += 1;
        self.call_write(0xBD, 0x00);
        self.r.cx = 10;
        self.set_bh(0);
        self.steps += 2;
        // LOOP goes back to `MOV BL,CL`, after the `MOV BH,0`.
        loop {
            self.set_bl(self.cl().wrapping_sub(1));
            self.steps += 4;
            let bl = self.bl();
            if bl != 9 {
                let op = self.cb(OPERATOR_OF_VOICE.wrapping_add(bl as u16));
                self.steps += 3;
                self.call_write(op.wrapping_add(0x40), 0x3F);
                let op = self.cb(OPERATOR_OF_VOICE.wrapping_add(bl as u16));
                self.steps += 3;
                self.call_write(op.wrapping_add(0x43), 0x3F);
            }
            self.set_bl(bl.wrapping_shl(1));
            self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
            self.steps += 3;
            self.clear_voice();
            self.steps += 1;
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:087f`: point at the command table and reset.
    fn init_state(&mut self) {
        self.r.bx = self.cw(COMMAND_TABLE);
        self.scw(COMMANDS, self.r.bx);
        self.steps += 3;
        self.reset();
        self.scw(QUEUE_READ, 0);
        self.scw(QUEUE_WRITE, 0);
        self.steps += 3;
    }

    /// Port of `EL2ADLIB:0346`: queue command 0 (silence), clear the effect timers and arm the
    /// engine timer at 1.
    fn clear_effects(&mut self) {
        self.r.ax = 0;
        self.steps += 2;
        self.queue();
        for (t, _) in EFFECT_TIMERS {
            self.scw(t, if t == 0x00c3 { 1 } else { 0 });
        }
        self.steps += 6;
    }

    /// Port of `EL2ADLIB:00d6`: start effect `AX`. Effects `18h`, `0Dh`, `0Bh` and `1Dh` arm a
    /// timer that stops them; if the timer is already running it is only re-armed (the effect
    /// keeps playing).
    fn start_effect(&mut self) {
        for (code, timer, ticks) in [(0x18u16, 0x00bdu16, 400u16), (0x0d, 0x00bf, 600), (0x0b, 0x00c1, 600), (0x1d, 0x00c5, 400)] {
            self.steps += 2;
            if self.r.ax == code {
                self.steps += 2;
                if self.cw(timer) != 0 {
                    self.scw(timer, ticks);
                    self.steps += 2;
                    return;
                }
                self.scw(timer, ticks);
                self.steps += 2;
            }
        }
        self.steps += 1;
        self.queue();
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:1494`: the bar counter (`57fh`–`582h`) that op `1Dh` waits on.
    fn tempo(&mut self) {
        let (s, c) = self.cb(0x0581).overflowing_add(self.cb(TEMPO));
        self.set_al(s);
        self.scb(0x0581, s);
        self.steps += 4;
        if !c {
            self.steps += 1;
            return;
        }
        let v = self.cb(0x0580).wrapping_sub(1);
        self.scb(0x0580, v);
        self.steps += 2;
        if v != 0 {
            self.steps += 1;
            return;
        }
        let al = self.cb(0x057f);
        self.set_al(al);
        self.scb(0x0580, al);
        let b = self.cb(0x0582).wrapping_add(1);
        self.scb(0x0582, b);
        self.steps += 4;
    }

    /// Port of `EL2ADLIB:164e`: silence voice `CL` (all operators to fastest release, key off).
    /// In rhythm mode voices 6–8 are left alone. The final write keys the voice on again at
    /// F-number 0, as the original does.
    fn silence_voice(&mut self) {
        self.steps += 2;
        if self.cb(RHYTHM_KEYS) != 0 {
            self.steps += 2;
            if self.cl() as i8 >= 6 {
                self.steps += 1;
                return;
            }
        }
        let v = self.cl();
        self.r.bx = OPERATOR_OF_VOICE.wrapping_add(v as u16);
        self.set_ch(v);
        let op = self.cb(self.r.bx);
        self.set_cl(op);
        self.steps += 5;
        for base in [0x60u8, 0x63, 0x80, 0x83] {
            self.steps += 3;
            self.call_write(base.wrapping_add(op), 0xFF);
        }
        self.steps += 3;
        self.call_write(0xB0u8.wrapping_add(v), 0);
        self.steps += 1;
        self.call_write(0xB0u8.wrapping_add(v), 0x20);
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:1404`: key off the current voice (not voice 9, and not the rhythm
    /// voices 6–8 while rhythm keys are in use).
    fn key_off(&mut self) {
        self.steps += 2;
        if self.cb(RHYTHM_KEYS) != 0 {
            self.steps += 2;
            if self.voice() as i8 >= 6 {
                self.steps += 1;
                return;
            }
        }
        self.steps += 2;
        let v = self.voice();
        if v == 9 {
            self.steps += 1;
            return;
        }
        let al = self.vb(0x28) & 0xDF;
        self.svb(0x28, al);
        self.steps += 5;
        self.call_write(0xB0u8.wrapping_add(v), al);
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:13cc`: key on the current voice and restart its vibrato.
    fn key_on(&mut self) {
        let saved = self.r.ax;
        let v = self.voice();
        let al = self.vb(0x28) | 0x20;
        self.svb(0x28, al);
        self.steps += 6;
        self.call_write(0xB0u8.wrapping_add(v), al);
        let cl = 9u8.wrapping_sub(self.vb(0x19));
        self.set_cl(cl);
        let ax = (self.vw(0x27) & 0x3FF) >> (cl & 31);
        self.svw(0x17, ax & 0xFF);
        let c = self.vb(0x1c);
        self.set_cl(c);
        self.svb(0x1f, c);
        self.r.ax = saved;
        self.steps += 13;
    }

    /// Port of `EL2ADLIB:1432`: set the voice's count-down to duration `AH`, plus a random
    /// part (`DI+36h` mask) or with a key-off point at `AH/8 × DI+2Dh` (staccato).
    fn duration(&mut self) {
        let ah = self.ah();
        self.scb(LAST_DURATION, ah);
        self.set_bl(ah);
        self.steps += 4;
        let mask = self.vb(0x36);
        if mask != 0 {
            self.steps += 1;
            self.random();
            let al = self.al() & mask;
            self.set_al(al);
            let bl = self.bl().wrapping_add(al);
            self.set_bl(bl);
            self.svb(0x05, bl);
            self.steps += 4;
            return;
        }
        self.steps += 2;
        let mult = self.vb(0x2d);
        if mult != 0 {
            self.set_bl(ah);
            let step = ah >> 3;
            self.set_ah(step);
            self.set_cl(mult);
            self.set_ch(0);
            let mut al = 0u8;
            self.steps += 7;
            for _ in 0..mult {
                al = al.wrapping_add(step);
                self.steps += 2;
            }
            self.set_al(al);
            self.r.cx = 0;
            self.svb(0x21, al);
            self.steps += 1;
        }
        let bl = self.bl();
        self.svb(0x05, bl);
        self.steps += 2;
    }

    /// The pitch part shared by `EL2ADLIB:1304` and op `39h` (`0e87`): note `AL` (octave in the
    /// high nibble, semitone in the low) plus the voice's transpositions → F-number and block,
    /// then the pitch-table offset at `DI+42h`. `zero_skips`: `1304` skips the table when the
    /// offset is 0, op `39h` does not. Leaves `CX` = (B0h value, A0h value).
    fn pitch(&mut self, note: u8, zero_skips: bool) {
        let mut ch = (note & 0xF0).wrapping_add(self.vb(0x07));
        let mut al = (note & 0x0F).wrapping_add(self.vb(0x12));
        self.steps += 9;
        if al as i8 >= 12 {
            al = al.wrapping_sub(12);
            ch = ch.wrapping_add(0x10);
            self.steps += 2;
        } else {
            self.steps += 2;
            if (al as i8) < 0 {
                al = al.wrapping_add(12);
                ch = ch.wrapping_sub(0x10);
                self.steps += 3;
            }
        }
        self.set_bl(al.wrapping_shl(1));
        self.set_bh(0);
        let mut ax = self.cw(F_NUMBERS.wrapping_add(self.r.bx));
        let dl = self.vb(0x24);
        self.r.dx = dl as u16;
        ax = ax.wrapping_add(dl as u16);
        ch = ((ch >> 2) & 0x1C) | (ax >> 8) as u8;
        self.r.ax = ax;
        self.set_ch(ch);
        self.set_cl(ax as u8);
        self.steps += 13;
        let off = self.vb(0x42);
        if zero_skips {
            self.steps += 1;
            if off == 0 {
                return;
            }
        }
        self.steps += 1;
        let low = self.vb(0x41) & 0x0F;
        if off as i8 >= 0 {
            self.r.bx = (low as u16 + 2) << 1;
            let table = self.cw(PITCH_TABLES.wrapping_add(self.r.bx));
            self.r.bx = off as u16;
            let a = self.cb(table.wrapping_add(off as u16));
            self.r.ax = a as u16;
            self.r.cx = self.r.cx.wrapping_add(a as u16);
            self.steps += 15;
        } else {
            self.r.bx = (low as u16) << 1;
            let table = self.cw(PITCH_TABLES.wrapping_add(self.r.bx));
            let idx = (off ^ 0xFF).wrapping_add(1);
            self.r.bx = idx as u16;
            let a = self.cb(table.wrapping_add(idx as u16));
            self.r.ax = a as u16;
            self.r.cx = self.r.cx.wrapping_sub(a as u16);
            self.steps += 14;
            if !zero_skips {
                self.steps += 1;
            }
        }
    }

    /// The tail shared by `1304` and op `39h`: store and write `A0h`/`B0h` (keeping the key bit).
    fn write_pitch(&mut self) {
        let v = self.voice();
        let ch = self.ch() | (self.vb(0x28) & 0x20);
        self.set_ch(ch);
        self.svb(0x28, ch);
        let cl = self.cl();
        self.svb(0x27, cl);
        self.steps += 9;
        self.call_write(0xA0u8.wrapping_add(v), cl);
        self.steps += 3;
        self.call_write(0xB0u8.wrapping_add(v), ch);
    }

    /// Port of `EL2ADLIB:1304`: set the current voice's pitch to note `AL`. `AX` is preserved.
    fn note(&mut self) {
        let saved = self.r.ax;
        let note = self.al();
        self.svb(0x41, note);
        self.steps += 2;
        self.pitch(note, true);
        self.write_pitch();
        self.r.ax = saved;
        self.steps += 2;
    }

    /// Port of `EL2ADLIB:1514`: carrier level = instrument level + volumes − scaling, clamped to
    /// 0–3Fh, with the KSL bits. `CL` and `BX` are left as the original leaves them.
    fn carrier_level(&mut self) -> u8 {
        let raw = self.vb(0x2f);
        let mut al = (raw & 0x3F).wrapping_add(self.vb(0x20)).wrapping_add(self.vb(0x00)).wrapping_add(self.vb(0x30));
        self.steps += 7;
        al = self.scale(al, 0x33);
        self.finish_level(al, raw)
    }

    /// Port of `EL2ADLIB:1554`: modulator level; volumes apply only in additive connection.
    fn modulator_level(&mut self) -> u8 {
        let raw = self.vb(0x2e);
        let mut al = raw & 0x3F;
        self.steps += 4;
        if self.vb(0x31) != 0 {
            al = al.wrapping_add(self.vb(0x20)).wrapping_add(self.vb(0x00)).wrapping_add(self.vb(0x30));
            self.steps += 3;
        }
        self.steps += 2;
        al = self.scale(al, 0x32);
        self.finish_level(al, raw)
    }

    /// Level scaling: subtract the high byte of `DI+34h` << (shift field + 1). The `CMP`/`JZ`
    /// before it are counted by the callers; the 6 instructions here only when taken.
    fn scale(&mut self, al: u8, field: u16) -> u8 {
        let s = self.vb(field);
        if s == 0 {
            return al;
        }
        let cl = s.wrapping_add(1);
        self.set_cl(cl);
        self.set_bh(0);
        let b = self.vb(0x34);
        self.set_bl(b);
        self.r.bx = self.r.bx << (cl & 31);
        self.steps += 6;
        al.wrapping_sub(self.bh())
    }

    fn finish_level(&mut self, mut al: u8, raw: u8) -> u8 {
        self.steps += 2;
        if al > 0x3F {
            self.steps += 2;
            if (al as i8) < 0 {
                al = 0;
                self.steps += 1;
            } else {
                al = 0x3F;
                self.steps += 2;
            }
        }
        let bl = raw & 0xC0;
        self.set_bl(bl);
        al |= bl;
        self.set_al(al);
        self.steps += 4;
        al
    }

    /// Port of `EL2ADLIB:14da`: write the current voice's carrier level (and the modulator's,
    /// in additive connection).
    fn set_levels(&mut self) {
        self.steps += 1;
        let al = self.carrier_level();
        let v = self.voice();
        self.r.bx = OPERATOR_OF_VOICE.wrapping_add(v as u16);
        let op = self.cb(self.r.bx);
        self.set_cl(op);
        self.steps += 6;
        self.call_write(op.wrapping_add(0x43), al);
        self.steps += 2;
        if self.vb(0x31) == 0 {
            self.steps += 1;
            return;
        }
        self.steps += 1;
        let al = self.modulator_level();
        self.r.bx = OPERATOR_OF_VOICE.wrapping_add(v as u16);
        let op = self.cb(self.r.bx);
        self.set_cl(op);
        self.steps += 6;
        self.call_write(op.wrapping_add(0x40), al);
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:15d3`: load the 11-byte instrument at `SI` into operator offset `CL`
    /// of the current voice. As in the original, a level scaling shift clobbers `CL` for the
    /// registers after it.
    fn load_instrument(&mut self) {
        for base in [0x20u8, 0x23] {
            let b = self.lodsb();
            self.steps += 3;
            self.call_write(base.wrapping_add(self.cl()), b);
        }
        let b = self.lodsb();
        let v = self.voice();
        self.steps += 3;
        self.call_write(0xC0u8.wrapping_add(v), b);
        self.set_al(b & 1);
        self.svb(0x31, b & 1);
        self.steps += 2;
        for base in [0xE0u8, 0xE3] {
            let b = self.lodsb();
            self.steps += 3;
            self.call_write(base.wrapping_add(self.cl()), b);
        }
        let reg = 0x40u8.wrapping_add(self.cl());
        let b = self.lodsb();
        self.svb(0x2e, b);
        self.steps += 5;
        let al = self.modulator_level();
        self.steps += 1;
        self.set_ah(reg);
        self.set_al(al);
        self.write_reg();
        let reg = 0x43u8.wrapping_add(self.cl());
        let b = self.lodsb();
        self.svb(0x2f, b);
        self.steps += 5;
        let al = self.carrier_level();
        self.steps += 1;
        self.set_ah(reg);
        self.set_al(al);
        self.write_reg();
        for base in [0x60u8, 0x63, 0x80, 0x83] {
            let b = self.lodsb();
            self.steps += 3;
            self.call_write(base.wrapping_add(self.cl()), b);
        }
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:1483`: clamp `AX` (signed) to 0–3Fh in `AL`.
    fn clamp(&mut self) {
        self.steps += 2;
        if (self.r.ax as i16) < 0 {
            self.set_al(0);
            self.steps += 2;
            return;
        }
        self.steps += 2;
        if (self.r.ax as i16) > 0x3F {
            self.set_al(0x3F);
            self.steps += 2;
            return;
        }
        self.steps += 1;
    }

    /// Start command `cmd` on its voice if its priority allows: the shared part of the queue
    /// drain (`08b3`) and op `02h` (`0a89`). Returns whether it started. `CL` = its voice.
    fn start_command(&mut self, cmd: u8) -> bool {
        self.r.bx = (cmd as u16) << 1;
        self.r.bx = self.r.bx.wrapping_add(self.cw(COMMANDS));
        self.r.si = self.cw(self.r.bx);
        self.r.bx = self.cw(self.r.si);
        let prio = self.bh();
        self.set_al(prio);
        self.set_cl(self.bl());
        self.set_bh(0);
        self.r.bx <<= 1;
        self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
        self.steps += 12;
        if (prio as i8) < (self.vb(0x08) as i8) {
            return false;
        }
        self.steps += 1;
        self.clear_voice();
        self.svb(0x08, prio);
        self.r.si = self.r.si.wrapping_add(2);
        self.svw(0x03, self.r.si);
        self.svb(0x25, 0xFF);
        self.svb(0x26, 0xFF);
        self.svb(0x05, 1);
        self.scb(ACTIVE, 1);
        self.steps += 8;
        true
    }

    /// Port of `EL2ADLIB:08b3`: start every queued command. Voice 9 (the spare) is not
    /// silenced first.
    fn drain_queue(&mut self) {
        loop {
            self.r.bx = self.cw(QUEUE_READ);
            self.steps += 3;
            if self.r.bx == self.cw(QUEUE_WRITE) {
                self.steps += 2;
                return;
            }
            let cmd = self.cb(QUEUE.wrapping_add(self.r.bx));
            self.set_bl(cmd);
            self.steps += 1;
            if self.start_command(cmd) {
                self.steps += 2;
                if self.bl() != 0x12 {
                    self.steps += 1;
                    self.silence_voice();
                }
            }
            let r = (self.cw(QUEUE_READ).wrapping_add(1)) & 0xF;
            self.scw(QUEUE_READ, r);
            self.steps += 3;
        }
    }

    // ---- per-voice callbacks ------------------------------------------------------------

    fn callback(&mut self, target: u16) {
        self.steps += 1;
        match target {
            CB_NONE => self.steps += 1,
            CB_VIBRATO => self.vibrato(),
            CB_LFO => self.register_lfo(),
            CB_SLIDE => self.slide(),
            other => panic!("EL2ADLIB: voice callback at unknown address {other:04x}"),
        }
    }

    /// Port of `EL2ADLIB:169d`: vibrato. After a delay (`DI+1Fh`), at a rate (`DI+1Dh`), adds
    /// the depth (`DI+17h`) to the F-number, reversing direction every `DI+1Bh` steps.
    fn vibrato(&mut self) {
        self.steps += 2;
        let delay = self.vb(0x1f);
        if delay != 0 {
            self.svb(0x1f, delay - 1);
            self.steps += 2;
            return;
        }
        let (s, c) = self.vb(0x1e).overflowing_add(self.vb(0x1d));
        self.set_cl(s);
        self.svb(0x1e, s);
        self.steps += 4;
        if !c {
            self.steps += 1;
            return;
        }
        self.r.bx = self.vw(0x17);
        let n = self.vb(0x1a).wrapping_sub(1);
        self.svb(0x1a, n);
        self.steps += 3;
        if n == 0 {
            self.r.bx = (self.r.bx ^ 0xFFFF).wrapping_add(1);
            self.svw(0x17, self.r.bx);
            let al = self.vb(0x1b);
            self.set_al(al);
            self.svb(0x1a, al);
            self.steps += 5;
        }
        let ax = (self.vw(0x27) & 0x3FF).wrapping_add(self.r.bx);
        self.r.ax = ax;
        self.svb(0x27, ax as u8);
        let b0 = (self.vb(0x28) & 0xFC) | (ax >> 8) as u8;
        self.svb(0x28, b0);
        let v = self.voice();
        let lo = self.vb(0x27);
        self.steps += 11;
        self.call_write(0xA0u8.wrapping_add(v), lo);
        self.steps += 3;
        self.call_write(0xB0u8.wrapping_add(v), b0);
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:1708`: a register LFO. At a rate, steps an index down through a table
    /// (`DI+3Eh`) and writes the entry to operator register `DI+3Dh`.
    fn register_lfo(&mut self) {
        let (s, c) = self.vb(0x3a).overflowing_add(self.vb(0x39));
        self.set_al(s);
        self.svb(0x3a, s);
        self.steps += 4;
        if !c {
            self.steps += 1;
            return;
        }
        let n = self.vb(0x3c).wrapping_sub(1);
        self.svb(0x3c, n);
        self.steps += 2;
        if (n as i8) < 0 {
            let r = self.vb(0x3b);
            self.svb(0x3c, r);
            self.steps += 2;
        }
        let reg = self.vb(0x3d).wrapping_add(self.cb(OPERATOR));
        let idx = self.vb(0x3c);
        self.r.bx = self.vw(0x3e);
        let val = self.cb(self.r.bx.wrapping_add(idx as u16));
        self.steps += 5;
        self.call_write(reg, val);
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:173c`: pitch slide by `DI+15h` per step, carrying into the block at the
    /// ends of the F-number range.
    fn slide(&mut self) {
        let (s, c) = self.vb(0x14).overflowing_add(self.vb(0x13));
        self.set_cl(s);
        self.svb(0x14, s);
        self.steps += 4;
        if !c {
            self.steps += 1;
            return;
        }
        let b0 = self.vb(0x28);
        self.r.bx = self.vb(0x27) as u16 | ((b0 & 3) as u16) << 8;
        let mut dl = b0 & 0x1C;
        let dh = b0 & 0x20;
        self.r.cx = self.vw(0x15);
        self.steps += 10;
        if self.r.cx as i16 >= 0 {
            self.r.bx = self.r.bx.wrapping_add(self.r.cx);
            self.steps += 3;
            if self.r.bx as i16 >= 0x2DE {
                // The original's PUSH/POP pair around the `AND BX,3FFh` test unbalances the
                // stack when the result is a multiple of 400h; slides never get there.
                self.r.bx >>= 1;
                dl = dl.wrapping_add(4) & 0x1C;
                self.steps += 8;
            }
        } else {
            self.r.bx = self.r.bx.wrapping_add(self.r.cx);
            self.steps += 3;
            if self.r.bx as i16 <= 0x184 {
                self.r.bx <<= 1;
                dl = dl.wrapping_sub(4) & 0x1C;
                self.steps += 7;
            }
        }
        self.r.dx = (dh as u16) << 8 | dl as u16;
        self.r.bx &= 0x3FF;
        self.r.cx = self.r.dx;
        let v = self.voice();
        let lo = self.bl();
        self.steps += 5;
        self.call_write(0xA0u8.wrapping_add(v), lo);
        self.svb(0x27, lo);
        let hi = self.bh() | dl | dh;
        self.set_bh(hi);
        self.steps += 6;
        self.call_write(0xB0u8.wrapping_add(v), hi);
        self.svb(0x28, hi);
        self.steps += 2;
    }

    // ---- the sequencer ------------------------------------------------------------------

    /// Port of `EL2ADLIB:0974`: step voices 9 down to 0.
    fn step_voices(&mut self) {
        self.steps += 2;
        if self.cb(ACTIVE) == 0 {
            self.steps += 1;
            return;
        }
        self.scb(VOICE, 9);
        self.steps += 1;
        loop {
            let next = self.step_voice();
            if matches!(next, Next::Callbacks) {
                let a = self.vw(0x29);
                self.callback(a);
                let b = self.vw(0x2b);
                self.callback(b);
            }
            let v = self.voice().wrapping_sub(1);
            self.scb(VOICE, v);
            self.steps += 2;
            if (v as i8) < 0 {
                self.steps += 1;
                return;
            }
            self.steps += 1;
        }
    }

    /// One voice of `0974`, from `0983` to the point where it reaches the callbacks (`0a65`)
    /// or the next voice (`0a6d`).
    fn step_voice(&mut self) -> Next {
        let v = self.voice();
        self.set_bl(v.wrapping_shl(1));
        self.set_bh(0);
        self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
        let count = self.vb(0x05);
        self.set_al(count);
        self.steps += 7;
        if count == 0 {
            self.steps += 1;
            return Next::NextVoice;
        }
        self.r.bx = OPERATOR_OF_VOICE.wrapping_add(v as u16);
        let op = self.cb(self.r.bx);
        self.set_al(op);
        self.scb(OPERATOR, op);
        self.steps += 7;
        if self.vb(0x40) != 0 {
            let t = self.cb(TEMPO);
            self.set_al(t);
            self.svb(0x25, t);
            self.steps += 2;
        }
        let (s, c) = self.vb(0x26).overflowing_add(self.vb(0x25));
        self.set_al(s);
        self.svb(0x26, s);
        self.steps += 4;
        if !c {
            self.steps += 1;
            return Next::Callbacks;
        }
        let count = self.vb(0x05).wrapping_sub(1);
        self.svb(0x05, count);
        self.steps += 2;
        if count != 0 {
            self.set_al(count);
            self.steps += 3;
            if self.al() == self.vb(0x21) {
                self.steps += 1;
                self.key_off();
            }
            self.steps += 2;
            if self.al() == self.vb(0x35) {
                self.steps += 2;
                if self.voice() != 9 {
                    self.steps += 1;
                    self.key_off();
                }
            }
            self.steps += 1;
            return Next::Callbacks;
        }
        self.r.si = self.vw(0x03);
        self.steps += 1;
        loop {
            match self.interpret() {
                Some(next) => return next,
                None => continue,
            }
        }
    }

    /// One byte-code instruction at `SI` (`09f8`). `None` = fetch the next one.
    fn interpret(&mut self) -> Option<Next> {
        self.r.ax = self.lodsw();
        self.steps += 3;
        if self.al() & 0x80 == 0 {
            // A note: pitch AL, key on, duration AH, optional level scaling byte.
            self.steps += 1;
            self.note();
            self.steps += 1;
            self.key_on();
            self.steps += 1;
            self.duration();
            self.steps += 2;
            let scaled = if self.vw(0x32) != 0 {
                self.steps += 1;
                true
            } else {
                self.steps += 2;
                self.vw(0x33) != 0
            };
            if scaled {
                let b = self.lodsb();
                self.set_al(b);
                self.svb(0x34, b);
                self.steps += 3;
                let al = self.carrier_level();
                let op = self.cb(OPERATOR);
                self.r.dx = 0x388;
                self.steps += 4;
                self.set_ah(op.wrapping_add(0x43));
                self.set_al(al);
                self.write_reg();
                self.steps += 1;
                let al = self.modulator_level();
                let op = self.cb(OPERATOR);
                self.r.dx = 0x388;
                self.steps += 4;
                self.set_ah(op.wrapping_add(0x40));
                self.set_al(al);
                self.write_reg();
            }
            return self.after_duration(true);
        }
        self.steps += 1;
        let op = self.al() & 0x7F;
        self.set_al(op);
        self.steps += 3;
        self.ops |= 1 << op;
        if op > 0x70 {
            self.steps += 1;
            return Some(self.end_voice());
        }
        self.set_bl(op.wrapping_shl(1));
        self.set_bh(0);
        let target = self.cw(OPCODES.wrapping_add(self.r.bx));
        self.r.bx = target;
        self.steps += 5;
        self.opcode(target)
    }

    /// `MOV AL,[562]; OR AL,AL; JZ`: if the duration just set is 0 continue with the next
    /// instruction, else stop here until it runs out. After a note (`0a41`) the stream
    /// position is saved only when stopping; the op tails (`0b78`, `0b9e`, `0ce8`, `0d00`) saved
    /// it already and end in a `JMP` either way.
    fn after_duration(&mut self, after_note: bool) -> Option<Next> {
        let d = self.cb(LAST_DURATION);
        self.set_al(d);
        self.steps += 3;
        if after_note {
            if d == 0 {
                return None;
            }
            self.svw(0x03, self.r.si);
            self.steps += 2;
        } else {
            self.steps += 1;
            if d == 0 {
                return None;
            }
        }
        Some(Next::Callbacks)
    }

    /// `0b2f`: end of a voice's stream (also every unused opcode). Voice 2 is the music's
    /// lead: when it ends the song is over, and in jukebox mode another starts.
    fn end_voice(&mut self) -> Next {
        self.svb(0x08, 0);
        self.svw(0x03, 0);
        self.steps += 4;
        if self.voice() != 9 {
            self.steps += 1;
            self.key_off();
        }
        self.steps += 2;
        if self.voice() == 2 {
            self.sdw(DS_SONG, 0xFFFF);
            self.steps += 3;
            if self.db(DS_JUKEBOX) != 0 {
                let saved = self.r.ax;
                self.r.ax = 0xFFFF;
                self.steps += 4;
                self.play_song();
                self.r.ax = saved;
                self.steps += 1;
            }
        }
        self.steps += 1;
        Next::NextVoice
    }

    /// Jump to `AH` (low) and the next byte (high): ops `01h`/`04h` (`0ae9`).
    fn jump(&mut self) {
        let lo = self.ah();
        self.set_cl(lo);
        let hi = self.lodsb();
        self.r.ax = (hi as u16) << 8 | lo as u16;
        self.r.si = self.r.ax;
        self.steps += 6;
    }

    fn opcode(&mut self, target: u16) -> Option<Next> {
        let ah = self.ah();
        match target {
            // 00h: set loop counter.
            0x0a78 => {
                self.svb(0x06, ah);
                self.steps += 2;
            }
            // 01h: loop: count down, jump while non-zero.
            0x0a7f => {
                let n = self.vb(0x06).wrapping_sub(1);
                self.svb(0x06, n);
                self.steps += 2;
                if n != 0 {
                    self.jump();
                } else {
                    self.r.si = self.r.si.wrapping_add(1);
                    self.steps += 2;
                }
            }
            // 02h: start command AH (if its voice's priority allows), keep going here.
            0x0a89 => {
                let (si, di, bx) = (self.r.si, self.r.di, self.r.bx);
                self.set_bl(ah);
                self.steps += 6;
                if ah != 0xFF {
                    self.set_bh(0);
                    if self.start_command(ah) {
                        self.steps += 1;
                        self.silence_voice();
                    }
                } else {
                    self.steps += 1;
                }
                self.r.si = si;
                self.r.di = di;
                self.r.bx = bx;
                self.steps += 4;
            }
            // 03h: second key-off point.
            0x0ae2 => {
                self.svb(0x35, ah);
                self.steps += 2;
            }
            // 04h: jump.
            0x0ae9 => self.jump(),
            // 05h: call (3-byte instruction).
            0x0af6 => {
                self.r.si = self.r.si.wrapping_sub(1);
                let depth = self.vb(0x09);
                let slot = 0x0a + depth.wrapping_shl(1) as u16;
                self.r.bx = depth.wrapping_shl(1) as u16;
                self.r.ax = self.r.si.wrapping_add(2);
                self.svw(slot, self.r.ax);
                self.svb(0x09, depth.wrapping_add(1));
                self.r.ax = self.lodsw();
                self.r.si = self.r.ax;
                self.steps += 12;
            }
            // 06h: return.
            0x0b12 => {
                self.r.si = self.r.si.wrapping_sub(1);
                let depth = self.vb(0x09).wrapping_sub(1);
                self.svb(0x09, depth);
                self.r.bx = depth.wrapping_shl(1) as u16;
                self.r.ax = self.vw(0x0a + self.r.bx);
                self.r.si = self.r.ax;
                self.steps += 8;
            }
            // 07h: octave transpose.
            0x0b28 => {
                self.svb(0x07, ah);
                self.steps += 2;
            }
            // End of stream.
            0x0b2f => return Some(self.end_voice()),
            // 09h: rest for AH (key off).
            0x0b66 => {
                self.steps += 1;
                self.duration();
                self.svw(0x03, self.r.si);
                self.steps += 3;
                if self.voice() != 9 {
                    self.steps += 1;
                    self.key_off();
                }
                return self.after_duration(false);
            }
            // 0Ah: write OPL register AH ← next byte.
            0x0b86 => {
                let v = self.lodsb();
                self.steps += 2;
                self.set_al(v);
                self.write_reg();
                self.steps += 1;
            }
            // 0Bh: legato note: pitch AH, duration next byte, no key-on.
            0x0b8e => {
                self.r.ax = self.r.ax.rotate_left(8);
                self.steps += 2;
                self.note();
                let b = self.lodsb();
                self.set_al(b);
                self.r.ax = self.r.ax.rotate_left(8);
                self.steps += 3;
                self.duration();
                self.svw(0x03, self.r.si);
                self.steps += 1;
                return self.after_duration(false);
            }
            // 0Ch: semitone transpose.
            0x0bac => {
                self.svb(0x12, ah);
                self.steps += 2;
            }
            // 0Dh: register LFO: rate AH, period, register, table.
            0x0bb3 => {
                self.svb(0x3a, ah);
                self.svb(0x39, ah);
                let p = self.lodsb();
                self.svb(0x3b, p);
                self.svb(0x3c, p);
                let reg = self.lodsb();
                self.svb(0x3d, reg);
                self.r.ax = self.lodsw();
                self.svw(0x3e, self.r.ax);
                self.svw(0x2b, CB_LFO);
                self.steps += 11;
            }
            // 0Eh: stop voice AH (no sound change).
            0x0bda => {
                let saved = self.voice();
                self.set_al(saved);
                let di = self.r.di;
                self.set_bl(ah.wrapping_shl(1));
                self.set_bh(0);
                self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
                self.svb(0x05, 0);
                self.svb(0x08, 0);
                self.svw(0x03, 0);
                self.r.di = di;
                self.scb(VOICE, saved);
                self.steps += 14;
            }
            // 10h: load instrument AH.
            0x0c04 => {
                let si = self.r.si;
                let op = self.cb(OPERATOR);
                self.set_cl(op);
                self.r.bx = (ah as u16) << 1;
                self.r.si = self.cw(INSTRUMENTS.wrapping_add(self.r.bx));
                self.steps += 7;
                self.load_instrument();
                self.r.si = si;
                self.steps += 2;
            }
            // 11h: pitch slide: rate AH, signed step (big-endian word).
            0x0c1c => {
                self.svb(0x13, ah);
                let hi = self.lodsb();
                let lo = self.lodsb();
                self.r.ax = (hi as u16) << 8 | lo as u16;
                self.svw(0x15, self.r.ax);
                self.svw(0x29, CB_SLIDE);
                self.svb(0x14, 0xFF);
                self.steps += 8;
            }
            // 12h: slide off.
            0x0c38 => {
                self.r.si = self.r.si.wrapping_sub(1);
                self.svw(0x29, CB_NONE);
                self.svw(0x15, 0);
                self.steps += 4;
            }
            // 13h: detune.
            0x0c48 => {
                self.svb(0x24, ah);
                self.steps += 2;
            }
            // 15h: vibrato: rate AH, depth shift, half period, delay.
            0x0c4f => {
                self.svb(0x1d, ah);
                let d = self.lodsb();
                self.svb(0x19, d);
                let p = self.lodsb();
                self.svb(0x1a, p.wrapping_add(1));
                self.svb(0x1b, p.wrapping_shl(1));
                let delay = self.lodsb();
                self.set_al(delay);
                self.svb(0x1c, delay);
                self.svw(0x29, CB_VIBRATO);
                self.steps += 12;
            }
            // 1Ah: priority.
            0x0c78 => {
                self.svb(0x08, ah);
                self.steps += 2;
            }
            // 1Ch: bar length.
            0x0c7f => {
                let h = ah >> 1;
                self.scb(0x057f, h);
                self.scb(0x0580, h);
                self.scb(0x0581, 0xFF);
                self.set_ah(0);
                self.scb(0x0582, 0);
                self.scb(0x0583, 0);
                self.steps += 8;
            }
            // 1Dh: wait for a bar boundary (AH masks the bar counter).
            0x0ca0 => {
                let w = self.cb(0x0583);
                self.set_al(w);
                self.steps += 3;
                let m = ah & self.cb(0x0582);
                self.set_ah(m);
                if w == 0 {
                    self.steps += 2;
                    if m == 0 {
                        self.scb(0x0583, 1);
                        self.steps += 2;
                    }
                } else {
                    self.steps += 2;
                    if m != 0 {
                        self.scb(0x0583, 0);
                        self.steps += 2;
                        return None;
                    }
                }
                self.r.si = self.r.si.wrapping_sub(2);
                self.svb(0x05, 1);
                self.svw(0x03, self.r.si);
                self.steps += 5;
                return Some(Next::NextVoice);
            }
            // 1Eh: volume.
            0x0cd7 => {
                self.svb(0x20, ah);
                self.steps += 2;
                self.set_levels();
                self.steps += 1;
            }
            // 20h: wait AH (key stays on).
            0x0ce1 => {
                self.steps += 1;
                self.duration();
                self.svw(0x03, self.r.si);
                self.steps += 1;
                return self.after_duration(false);
            }
            // 21h: key on again for AH.
            0x0cf6 => {
                self.steps += 1;
                self.duration();
                self.steps += 1;
                self.key_on();
                self.svw(0x03, self.r.si);
                self.steps += 1;
                return self.after_duration(false);
            }
            // 24h: staccato.
            0x0d0e => {
                self.set_ah(ah & 7);
                self.svb(0x2d, ah & 7);
                self.steps += 3;
            }
            // 26h: global tempo.
            0x0d18 => {
                self.scb(TEMPO, ah);
                self.steps += 2;
            }
            // 27h: LFO off.
            0x0d20 => {
                self.r.si = self.r.si.wrapping_sub(1);
                self.svw(0x2b, CB_NONE);
                self.steps += 3;
            }
            // 29h: voice tempo.
            0x0d2a => {
                self.svb(0x25, ah);
                self.steps += 2;
            }
            // 2Bh: second volume.
            0x0d31 => {
                self.svb(0x30, ah);
                self.steps += 2;
            }
            // 2Ch / 2Dh: set / add the base volume of voice AH.
            0x0d38 | 0x0d5f => {
                let saved = self.voice();
                self.set_al(saved);
                let di = self.r.di;
                let b = self.lodsb();
                self.scb(VOICE, ah);
                self.r.bx = ah.wrapping_shl(1) as u16;
                self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
                let v = if target == 0x0d38 { b } else { b.wrapping_add(self.vb(0x00)) };
                self.set_al(v);
                self.svb(0x00, v);
                self.steps += if target == 0x0d38 { 11 } else { 12 };
                self.set_levels();
                self.r.di = di;
                self.set_al(saved);
                self.scb(VOICE, saved);
                self.steps += 4;
            }
            // 2Eh / 2Fh: tremolo / vibrato depth bit of register BDh.
            0x0d89 | 0x0da9 => {
                let (shift, keep) = if target == 0x0d89 { (7, 0x7F) } else { (6, 0xBF) };
                self.set_cl(shift);
                let bit = (ah & 1) << shift;
                let bl = (self.cb(DEPTH_BITS) & keep) | bit;
                self.set_bl(bl);
                self.scb(DEPTH_BITS, bl);
                self.steps += 10;
                self.set_ah(0xBD);
                self.set_al(bl);
                self.write_reg();
                self.steps += 1;
            }
            // 30h: add to volume.
            0x0dc9 => {
                let v = ah.wrapping_add(self.vb(0x20));
                self.set_ah(v);
                self.svb(0x20, v);
                self.steps += 3;
                self.set_levels();
                self.steps += 1;
            }
            // 33h: stop voice AH and mute it.
            0x0dd7 => {
                let saved = self.voice();
                self.set_al(saved);
                let di = self.r.di;
                self.scb(VOICE, ah);
                self.r.bx = ah.wrapping_shl(1) as u16;
                self.r.di = self.cw(VOICE_RECORDS.wrapping_add(self.r.bx));
                self.svb(0x05, 0);
                self.svb(0x08, 0);
                self.svw(0x03, 0);
                self.svb(0x00, 0);
                self.steps += 14;
                if ah != 9 {
                    self.r.bx = ah as u16;
                    let op = self.cb(OPERATOR_OF_VOICE.wrapping_add(self.r.bx));
                    self.set_cl(op);
                    self.steps += 7;
                    self.set_ah(0xC0u8.wrapping_add(ah));
                    self.set_al(0);
                    self.write_reg();
                    self.steps += 3;
                    self.call_write(0x43u8.wrapping_add(op), 0x3F);
                    self.steps += 3;
                    self.call_write(0x83u8.wrapping_add(op), 0xFF);
                    self.steps += 3;
                    self.call_write(0xB0u8.wrapping_add(ah), 0);
                }
                self.r.di = di;
                self.scb(VOICE, saved);
                self.steps += 4;
            }
            // 35h: random pitch offset within mask (AH, next byte).
            0x0e45 => {
                let b = self.lodsb();
                self.set_al(b);
                self.r.bx = self.r.ax;
                self.steps += 3;
                self.random();
                self.r.ax &= self.r.bx;
                let b0 = self.vb(0x28);
                let cl = b0 & 0x20;
                self.set_cl(cl);
                self.r.bx = ((b0 & 0x1F) as u16) << 8 | self.vb(0x27) as u16;
                self.r.bx = self.r.bx.wrapping_add(self.r.ax);
                self.set_bh(self.bh() | cl);
                let v = self.voice();
                let (lo, hi) = (self.bl(), self.bh());
                self.steps += 12;
                self.set_ah(0xA0u8.wrapping_add(v));
                self.set_al(lo);
                self.write_reg();
                self.steps += 3;
                self.call_write(0xB0u8.wrapping_add(v), hi);
                self.steps += 1;
            }
            // 36h: vibrato / slide off.
            0x0e7d => {
                self.r.si = self.r.si.wrapping_sub(1);
                self.svw(0x29, CB_NONE);
                self.steps += 3;
            }
            // 39h: pitch-table offset AH, re-pitch the last note.
            0x0e87 => {
                self.svb(0x42, ah);
                let note = self.vb(0x41);
                self.set_al(note);
                let saved = self.r.ax;
                self.steps += 3;
                self.pitch(note, false);
                self.write_pitch();
                self.r.ax = saved;
                self.steps += 2;
            }
            // 3Ah: voice tempo from the global tempo.
            0x0f55 => {
                self.r.si = self.r.si.wrapping_sub(1);
                let t = self.cb(TEMPO);
                self.set_al(t);
                self.svb(0x25, t);
                self.steps += 4;
            }
            // 3Bh: no operation.
            0x0f61 => {
                self.r.si = self.r.si.wrapping_sub(1);
                self.steps += 2;
            }
            // 3Ch: random duration mask.
            0x0f65 => {
                self.svb(0x36, ah);
                self.steps += 2;
            }
            // 3Dh: change voice tempo by signed AH, saturating at FFh / 1.
            0x0f6c => {
                let t = self.vb(0x25);
                self.steps += 2;
                let mut v = ah.wrapping_add(t);
                if ah as i8 >= 0 {
                    self.steps += 2;
                    if (ah as u16 + t as u16) > 0xFF {
                        v = 0xFF;
                        self.steps += 2;
                    }
                } else {
                    self.steps += 3;
                    if v >= t {
                        v = 1;
                        self.steps += 1;
                    }
                }
                self.set_ah(v);
                self.svb(0x25, v);
                self.steps += 2;
            }
            // 3Fh–41h: rhythm set-up: instruments for voices 6, 7, 8 and their pitches.
            0x0f8d => self.rhythm_setup(),
            // 42h: rhythm hit AH (bits of BDh): first release the drums being hit, then key
            // them (both writes to BDh, the second without re-selecting the index).
            0x1095 => {
                self.steps += 2;
                self.h.out8(self.steps, 0x388, 0xBD);
                self.steps += 1 + 1 + 17 + 1;
                self.r.dx = 0x389;
                let keys = self.cb(RHYTHM_KEYS);
                let off = (!(ah & 0x1F) & keys) | 0x20;
                self.steps += 6;
                self.h.out8(self.steps, 0x389, off);
                self.steps += 1 + 1 + 55;
                let on = ah | keys;
                self.scb(RHYTHM_KEYS, on);
                let v = on | self.cb(DEPTH_BITS) | 0x20;
                self.r.ax = 0xBD00 | v as u16;
                self.steps += 5;
                self.h.out8(self.steps, 0x389, v);
                self.steps += 1 + 1 + 55;
            }
            // 43h: rhythm off.
            0x10c9 => {
                self.r.si = self.r.si.wrapping_sub(1);
                let al = self.cb(DEPTH_BITS) & 0xC0;
                self.scb(RHYTHM_KEYS, 0);
                self.steps += 5;
                self.call_write(0xBD, al);
                self.steps += 1;
            }
            // 44h–46h: rhythm levels for the drums selected by the bits of AH.
            0x10de | 0x1197 | 0x1250 => self.rhythm_levels(target),
            // 48h: follow the global tempo.
            0x12f0 => {
                self.svb(0x40, ah);
                self.steps += 2;
            }
            // 49h: level scaling shifts (modulator AH, carrier next byte).
            0x12f7 => {
                self.svb(0x32, ah);
                let b = self.lodsb();
                self.svb(0x33, b);
                self.steps += 4;
            }
            other => panic!("EL2ADLIB: opcode handler at unknown address {other:04x}"),
        }
        None
    }

    /// Op `3Fh`–`41h` (`0f8d`): load the three rhythm instruments (voice 6 from `AH`, voices
    /// 7 and 8 from the next bytes), keep their levels at `570h`–`574h`, write their pitches
    /// and switch rhythm keys on (`567h` = 20h).
    fn rhythm_setup(&mut self) {
        let saved_voice = self.voice();
        let saved_op = self.cb(OPERATOR);
        let saved_si = self.r.si;
        let arg = self.ah();
        let mut inst = arg;
        for (n, voice) in [6u8, 7, 8].into_iter().enumerate() {
            self.scb(VOICE, voice);
            let op = self.cb(OPERATOR_OF_VOICE + voice as u16);
            self.scb(OPERATOR, op);
            self.r.dx = 0x388;
            self.set_cl(op);
            self.r.bx = (inst as u16) << 1;
            self.r.si = self.cw(INSTRUMENTS.wrapping_add(self.r.bx));
            match n {
                0 => {
                    let b = self.cb(self.r.si.wrapping_add(6));
                    self.scb(0x0570, b);
                    self.steps += 19;
                }
                1 => {
                    let b = self.cb(self.r.si.wrapping_add(5));
                    self.scb(0x0572, b);
                    let b = self.cb(self.r.si.wrapping_add(6));
                    self.scb(0x0571, b);
                    self.steps += 16;
                }
                _ => {
                    let b = self.cb(self.r.si.wrapping_add(5));
                    self.scb(0x0573, b);
                    let b = self.cb(self.r.si.wrapping_add(6));
                    self.scb(0x0574, b);
                    self.steps += 16;
                }
            }
            self.load_instrument();
            // POP SI, then (except after voice 8) LODSB the next instrument and PUSH SI.
            self.r.si = saved_si.wrapping_add(n as u16);
            self.steps += 1;
            if n < 2 {
                inst = self.lodsb();
                self.set_al(inst);
                self.steps += 2;
            }
        }
        for (voice, rec) in [(6u8, 0x0766u16), (7, 0x07a9), (8, 0x07ec)] {
            let b0 = self.lodsb() & 0x2F;
            self.scb(rec, b0);
            self.steps += 4;
            self.call_write(0xB0 + voice, b0);
            let a0 = self.lodsb();
            self.steps += 2;
            self.call_write(0xA0 + voice, a0);
        }
        self.scb(RHYTHM_KEYS, 0x20);
        self.scb(OPERATOR, saved_op);
        self.scb(VOICE, saved_voice);
        self.r.ax = (arg as u16) << 8 | saved_voice as u16;
        self.steps += 6;
    }

    /// Ops `44h`–`46h` (`10de`, `1197`, `1250`): for each drum bit in `AH` (1 bass drum,
    /// 2 hi-hat, 4 tom-tom, 8 snare, 10h cymbal) set its level from the next byte:
    /// `44h` sets the extra level, `45h` adds it to the stored level, `46h` sets the stored
    /// level. Each writes base + stored + extra, clamped to 3Fh, to the drum's operator.
    fn rhythm_levels(&mut self, target: u16) {
        const DRUMS: [(u8, u16, u16, u16, u8); 5] = [
            (0x01, 0x0572, 0x0577, 0x057c, 0x51),
            (0x02, 0x0574, 0x0579, 0x057e, 0x55),
            (0x04, 0x0573, 0x0578, 0x057d, 0x52),
            (0x08, 0x0571, 0x0576, 0x057b, 0x54),
            (0x10, 0x0570, 0x0575, 0x057a, 0x53),
        ];
        let b = self.lodsb();
        self.set_al(b);
        self.r.bx = self.r.ax;
        self.steps += 2;
        let mask = self.ah();
        for (i, (bit, base, stored, extra, reg)) in DRUMS.into_iter().enumerate() {
            if i > 0 {
                self.r.ax = self.r.bx;
                self.steps += 1;
            }
            self.steps += 2;
            if mask & bit == 0 {
                continue;
            }
            let v = match target {
                0x10de => {
                    self.scb(extra, b);
                    b.wrapping_add(self.cb(base)).wrapping_add(self.cb(stored)).wrapping_add(self.cb(extra))
                }
                0x1197 => b.wrapping_add(self.cb(base)).wrapping_add(self.cb(stored)).wrapping_add(self.cb(extra)),
                _ => {
                    self.scb(stored, b);
                    b.wrapping_add(self.cb(base)).wrapping_add(self.cb(extra))
                }
            };
            self.r.ax = v as u16;
            self.steps += if target == 0x10de { 6 } else { 5 };
            self.clamp();
            let al = self.al();
            if target == 0x1197 {
                self.scb(stored, al);
                self.steps += 1;
            }
            self.steps += 2;
            self.set_ah(reg);
            self.write_reg();
        }
        self.steps += 1;
    }

    // ---- exports ------------------------------------------------------------------------

    /// Port of `EL2ADLIB:0152` (slot `DS:032c`): engine sound at volume `BX` (0–63) and pitch
    /// `CX`; it stops by itself 200 ticks after the last call. Only while effects are on and
    /// no song plays. Registers preserved.
    pub fn engine(&mut self) {
        self.steps += 2;
        if self.db(DS_EFFECTS_ON) == 0 {
            self.steps += 1;
            return;
        }
        self.steps += 2;
        if self.dw(DS_SONG) as i16 >= 0 {
            self.steps += 1;
            return;
        }
        let saved = self.r;
        self.steps += 4;
        let mut bx = self.r.bx;
        if bx > 0x3F {
            bx = 0x3F;
            self.steps += 1;
        }
        let bl = (bx as u8).wrapping_shl(1);
        let cx = self.r.cx.wrapping_sub(0xAE) >> 2;
        let mut ch = 0x3Fu8.wrapping_sub(cx as u8);
        self.steps += 7;
        if ch > 0x3F {
            ch = 0x3F;
            self.steps += 1;
        }
        let busy = self.cb(BUSY).wrapping_add(1);
        self.scb(BUSY, busy);
        self.scb(0x66f0, ch.wrapping_shl(2));
        self.scb(0x66ea, !(bl >> 1) & 0x3F);
        self.steps += 9;
        if self.cw(0x00c3) == 0 {
            self.r.ax = 0x21;
            self.steps += 2;
            self.queue();
        }
        self.scw(0x00c3, 200);
        let busy = self.cb(BUSY).wrapping_sub(1);
        self.scb(BUSY, busy);
        self.r = saved;
        self.steps += 5;
    }

    /// Port of `EL2ADLIB:01b6` (slot `DS:0330`): stop the music unless jukebox mode
    /// (`DS:463f`) is on.
    pub fn stop_music_unless_jukebox(&mut self) {
        self.steps += 2;
        if self.db(DS_JUKEBOX) != 0 {
            self.steps += 1;
            return;
        }
        self.stop_music();
    }

    /// Port of `EL2ADLIB:01c0` (slot `DS:0334`): initialise the driver and enable its tick.
    pub fn init(&mut self) {
        self.steps += 1;
        self.init_state();
        self.sdw(DS_SONG, 0xFFFF);
        let s = self.db(DS_SOUND) | 0x80;
        self.sdb(DS_SOUND, s);
        self.steps += 3;
        self.clear_effects();
        self.steps += 1;
    }

    /// Port of `EL2ADLIB:01d2` (slot `DS:0338`): play song `AX`, or a random enabled song for
    /// `AX` < 0 (only when none is playing). Registers preserved.
    pub fn play_song(&mut self) {
        let saved = self.r;
        self.steps += 4;
        let mut ax = self.r.ax;
        if ax as i16 >= 0 {
            self.steps += 2;
            if self.db(DS_MUSIC_ON) == 0 {
                self.steps += 3;
                return;
            }
        }
        let mut cx: u16 = 0x28;
        self.steps += 3;
        let mut draw = ax as i16 >= 0;
        if !draw {
            self.steps += 2;
            if self.dw(DS_SONG) != 0xFFFF {
                self.r = saved;
                self.steps += 3;
                return;
            }
            self.steps += 2;
            ax = if ax & 1 != 0 {
                7
            } else {
                self.steps += 1;
                6
            };
            self.steps += 1;
            ax = self.main_random(ax);
            draw = true;
        }
        let found = loop {
            debug_assert!(draw);
            self.steps += 3;
            if self.db(DS_SONG_ENABLED.wrapping_add(ax)) != 0 {
                break Some(ax);
            }
            self.steps += 1;
            cx -= 1;
            if cx == 0 {
                break None;
            }
            self.steps += 1;
            ax = self.main_random(7);
        };
        let song = match found {
            Some(s) => Some(s),
            None => {
                self.steps += 1;
                let mut a: u16 = 0xFFFF;
                loop {
                    a = a.wrapping_add(1);
                    self.steps += 4;
                    if self.db(DS_SONG_ENABLED.wrapping_add(a)) != 0 {
                        break Some(a);
                    }
                    self.steps += 2;
                    if a >= 7 {
                        self.steps += 1;
                        break None;
                    }
                }
            }
        };
        if let Some(s) = song {
            self.sdw(DS_SONG, s);
            self.r.ax = s.wrapping_add(0x32);
            self.steps += 3;
            self.queue();
        }
        self.r = saved;
        self.steps += 3;
    }

    /// Port of `1000:069b` (slot `DS:0124`), the game's random number in 0…`n`−1, as the
    /// driver calls it (`CALLF [0124]`, counted by the caller): a two-word additive generator
    /// at `DS:1fd9`, scaled by `n`.
    fn main_random(&mut self, n: u16) -> u16 {
        let lo = self.dw(DS_SEED) as u32 | (self.dw(DS_SEED + 2) as u32) << 16;
        let hi = self.dw(DS_SEED + 4) as u32 | (self.dw(DS_SEED + 6) as u32) << 16;
        let mut ebx = lo.wrapping_add(hi);
        let edx = hi.rotate_left(5);
        ebx = ebx.wrapping_add(edx).rotate_right(16);
        self.sdw(DS_SEED, ebx as u16);
        self.sdw(DS_SEED + 2, (ebx >> 16) as u16);
        self.sdw(DS_SEED + 4, edx as u16);
        self.sdw(DS_SEED + 6, (edx >> 16) as u16);
        self.steps += 1 + 15;
        ((n as u32 * (ebx & 0xFFFF)) >> 16) as u16
    }

    /// Port of `EL2ADLIB:022a` (slot `DS:033c`): reset the chip and disable the tick.
    pub fn shutdown(&mut self) {
        self.steps += 2;
        self.reset();
        self.steps += 1;
        let s = self.db(DS_SOUND) & 0x7F;
        self.sdb(DS_SOUND, s);
        self.steps += 2;
    }

    /// Port of `EL2ADLIB:0233` (slot `DS:0340`): effect `AX` (game numbering, mapped through
    /// the table at `EL2ADLIB:0095`). Registers preserved.
    pub fn effect(&mut self) {
        self.steps += 2;
        if self.db(DS_EFFECTS_ON) == 0 {
            self.steps += 1;
            return;
        }
        self.steps += 2;
        if self.r.ax == 0x14 {
            self.steps += 2;
            if self.db(DS_EFFECT_14_ON) == 0 {
                self.steps += 1;
                return;
            }
        }
        self.mapped_effect();
    }

    /// `0246`–`025a` and `0266`–`027f`: look up effect `AX` and start it.
    fn mapped_effect(&mut self) {
        let saved = self.r;
        let code = self.cb(EFFECT_MAP.wrapping_add(self.r.ax));
        self.steps += 6;
        if (code as i8) >= 0 {
            self.set_al(code);
            self.r.bx = saved.ax;
            self.steps += 1;
            self.start_effect();
        }
        self.r = saved;
        self.steps += 3;
    }

    /// Port of `EL2ADLIB:025b` (slot `DS:0348`): effect `AX` if `BX` ≠ 0. `BX` (a volume or
    /// distance, *inferred*) is otherwise unused. Registers preserved.
    pub fn effect_with_volume(&mut self) {
        self.steps += 2;
        if self.db(DS_EFFECTS_ON) == 0 {
            self.steps += 1;
            return;
        }
        self.steps += 2;
        if self.r.bx == 0 {
            self.steps += 1;
            return;
        }
        let saved = self.r;
        self.steps += 2;
        let code = self.cb(EFFECT_MAP.wrapping_add(self.r.ax));
        self.steps += 6;
        if (code as i8) >= 0 {
            self.set_al(code);
            self.steps += 1;
            self.start_effect();
        }
        self.r = saved;
        self.steps += 3;
    }

    /// Port of `EL2ADLIB:0280` (slot `DS:0344`): effect `AX` at range `BL` (0–14), only while
    /// `DS:5709` ≠ 0. Leaves `BX` = the range's volume from `EL2ADLIB:00c7`.
    pub fn effect_at_range(&mut self) {
        self.steps += 2;
        if self.db(DS_EFFECTS_ON) == 0 {
            self.steps += 1;
            return;
        }
        self.steps += 2;
        if self.bl() >= 0x0F {
            self.steps += 1;
            return;
        }
        self.steps += 2;
        let active = (0..4).any(|i| self.h.read8(lin(self.ds, DS_RANGE_ACTIVE + i)) != 0);
        if !active {
            self.steps += 1;
            return;
        }
        self.set_bh(0);
        let v = self.cb(RANGE_VOLUME.wrapping_add(self.r.bx));
        self.set_bl(v);
        self.steps += 3;
        self.effect_with_volume();
    }

    /// Port of `EL2ADLIB:029e` (slot `DS:034c`): the 202.27 Hz tick from IRQ 0. Counts down the
    /// effect timers every tick and runs the sequencer on 327 of every 910 ticks (72.68 Hz).
    /// Registers preserved.
    pub fn tick(&mut self) {
        self.steps += 2;
        if self.cb(BUSY) != 0 {
            self.steps += 1;
            return;
        }
        let saved = self.r;
        let busy = self.cb(BUSY).wrapping_add(1);
        self.scb(BUSY, busy);
        self.steps += 3;
        for (t, stop) in EFFECT_TIMERS {
            self.steps += 2;
            let v = self.cw(t);
            if v != 0 {
                self.scw(t, v - 1);
                self.steps += 2;
                if v == 1 {
                    self.r.ax = stop;
                    self.steps += 2;
                    self.queue();
                }
            }
        }
        let (d, borrow) = self.cw(DIVIDER).overflowing_sub(0x147);
        self.scw(DIVIDER, d);
        self.steps += 2;
        if borrow {
            self.scw(DIVIDER, d.wrapping_add(0x38E));
            self.steps += 2;
            self.drain_queue();
            self.steps += 1;
            self.step_voices();
            self.steps += 1;
            self.tempo();
        }
        self.r = saved;
        let busy = self.cb(BUSY).wrapping_sub(1);
        self.scb(BUSY, busy);
        self.steps += 4;
    }

    /// Port of `EL2ADLIB:033b` (slot `DS:0350`): silence everything. Registers preserved.
    pub fn silence_all(&mut self) {
        let saved = self.r;
        self.steps += 3;
        self.clear_effects();
        self.r = saved;
        self.steps += 3;
    }

    /// Port of `EL2ADLIB:036f` (slot `DS:0354`): stop the music. Registers preserved.
    pub fn stop_music(&mut self) {
        let saved = self.r;
        self.r.ax = 0x23;
        self.steps += 4;
        self.queue();
        self.sdw(DS_SONG, 0xFFFF);
        self.r = saved;
        self.steps += 4;
    }
}
