//! Yamaha YM3812 (OPL2), the FM chip on the AdLib and Sound Blaster cards that `EL2ADLIB.OVL`
//! drives (docs/AUDIO.md).
//!
//! This is an original implementation of the chip's documented behaviour, clocked exactly like
//! the hardware: one output sample every 72 master clocks (3.579545 MHz / 72 = 49,716 Hz). It
//! models what the published die and datasheet analyses describe:
//!
//! - the log-sine and exponent ROMs (the chip has no multiplier: an operator looks up
//!   −log₂ sin of its phase, adds its attenuation in the log domain, and converts back);
//! - the 19-bit phase accumulators with the frequency multiplier table, vibrato and the rhythm
//!   section's phase tricks for hi-hat, snare and cymbal, and the 23-bit noise generator;
//! - the envelope generator's 9-bit attenuation (0.1875 dB per step), its global counter that
//!   decides when each rate steps, the exponential attack and linear decay, key scaling of rate
//!   and level, total level and tremolo;
//! - the four OPL2 waveforms (enabled by register 01h bit 5), operator feedback, FM and
//!   additive connections, rhythm mode, and the two timers with the status register.
//!
//! Output is the chip's 16-bit digital sum (the YM3014 DAC stage is not modelled, so there is
//! no extra quantisation noise). Everything is integer and deterministic.

mod resample;

pub use resample::Resampler;

/// Master clock of an AdLib / Sound Blaster OPL2 (NTSC colour-burst crystal).
pub const CLOCK_HZ: f64 = 3_579_545.0;
/// Native output rate: one sample per 72 master clocks.
pub const SAMPLE_RATE: f64 = CLOCK_HZ / 72.0;

/// Frequency multiplier ×2 (MULT 0 is ½).
const MULT_X2: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];
/// Key-scale level ROM, indexed by the top four F-number bits (units of 0.75 dB at block 7).
const KSL_ROM: [i32; 16] = [0, 32, 40, 45, 48, 51, 53, 55, 56, 58, 59, 60, 61, 62, 63, 64];
/// KSL register value → right shift of the key-scale attenuation (off, 3, 1.5, 6 dB/octave).
const KSL_SHIFT: [u32; 4] = [8, 1, 2, 0];
/// Extra envelope steps for rates ≥ 48, by the rate's low two bits and the counter phase.
const RATE_STEP: [[u8; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];
/// Register offset (low five bits of 20h–F5h) → operator slot, or none for the gaps.
const SLOT_OF_OFFSET: [i8; 32] = [
    0, 1, 2, 3, 4, 5, -1, -1, 6, 7, 8, 9, 10, 11, -1, -1, 12, 13, 14, 15, 16, 17, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1,
];

/// Attenuation meaning "silent" in the operator's log domain.
const SILENT: u32 = 0x1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
struct Operator {
    // Register fields.
    am: bool,
    vib: bool,
    sustain: bool,
    ksr: bool,
    mult: u8,
    ksl: u8,
    tl: u8,
    ar: u8,
    dr: u8,
    sl: u8,
    rr: u8,
    wave: u8,
    // State.
    phase: u32,
    level: u16,
    stage: Stage,
    /// Key-on sources: bit 0 the channel's key bit, bit 1 the rhythm register.
    key: u8,
    out: i32,
    prev_out: i32,
}

impl Operator {
    const fn new() -> Self {
        Operator {
            am: false,
            vib: false,
            sustain: false,
            ksr: false,
            mult: 0,
            ksl: 0,
            tl: 0,
            ar: 0,
            dr: 0,
            sl: 0,
            rr: 0,
            wave: 0,
            phase: 0,
            level: 0x1FF,
            stage: Stage::Release,
            key: 0,
            out: 0,
            prev_out: 0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Channel {
    fnum: u16,
    block: u8,
    feedback: u8,
    additive: bool,
}

/// One YM3812.
pub struct Opl2 {
    ops: [Operator; 18],
    chans: [Channel; 9],
    log_sin: [u16; 256],
    exp: [u16; 256],
    wave_select: bool,
    note_select: bool,
    deep_am: bool,
    deep_vib: bool,
    rhythm: bool,
    rhythm_keys: u8,
    /// Samples generated since reset; drives the LFOs and the envelope clock.
    clock: u64,
    env_counter: u64,
    noise: u32,
    trem_pos: u32,
    vib_pos: u32,
    timer_reload: [u8; 2],
    timer_count: [u8; 2],
    timer_run: [bool; 2],
    timer_mask: [bool; 2],
    status: u8,
}

impl Default for Opl2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Opl2 {
    pub fn new() -> Self {
        let mut log_sin = [0u16; 256];
        let mut exp = [0u16; 256];
        for i in 0..256 {
            // Quarter sine, sampled at the middle of each of 256 steps, as −log₂ in 1/256 units.
            let s = (((i as f64) + 0.5) * std::f64::consts::PI / 512.0).sin();
            log_sin[i] = (-s.log2() * 256.0).round() as u16;
            // 2^(−x/256) as 10 fraction bits plus the implicit leading one.
            let j = 255 - i;
            exp[i] = (((2f64.powf(j as f64 / 256.0) - 1.0) * 1024.0).round() as u16) + 1024;
        }
        Opl2 {
            ops: [Operator::new(); 18],
            chans: [Channel::default(); 9],
            log_sin,
            exp,
            wave_select: false,
            note_select: false,
            deep_am: false,
            deep_vib: false,
            rhythm: false,
            rhythm_keys: 0,
            clock: 0,
            env_counter: 0,
            noise: 1,
            trem_pos: 0,
            vib_pos: 0,
            timer_reload: [0; 2],
            timer_count: [0; 2],
            timer_run: [false; 2],
            timer_mask: [false; 2],
            status: 0,
        }
    }

    /// The status register (port 388h read): bit 7 IRQ, bit 6 timer 1, bit 5 timer 2. The low
    /// bits read back as `110b` on an OPL2, which is how programs tell it from an OPL3.
    pub fn status(&self) -> u8 {
        self.status | 0x06
    }

    /// Write `value` to register `reg` (port 389h after selecting the index on 388h).
    pub fn write(&mut self, reg: u8, value: u8) {
        match reg {
            0x01 => self.wave_select = value & 0x20 != 0,
            0x02 => self.timer_reload[0] = value,
            0x03 => self.timer_reload[1] = value,
            0x04 => {
                if value & 0x80 != 0 {
                    self.status = 0;
                    return;
                }
                self.timer_mask = [value & 0x40 != 0, value & 0x20 != 0];
                for t in 0..2 {
                    let run = value & (1 << t) != 0;
                    if run && !self.timer_run[t] {
                        self.timer_count[t] = self.timer_reload[t];
                    }
                    self.timer_run[t] = run;
                }
            }
            // Bit 7 (CSM speech mode) is not used by any Frontier driver and is not modelled.
            0x08 => self.note_select = value & 0x40 != 0,
            0x20..=0x35 | 0x40..=0x55 | 0x60..=0x75 | 0x80..=0x95 | 0xE0..=0xF5 => {
                let slot = SLOT_OF_OFFSET[(reg & 0x1F) as usize];
                if slot < 0 {
                    return;
                }
                let op = &mut self.ops[slot as usize];
                match reg & 0xE0 {
                    0x20 => {
                        op.am = value & 0x80 != 0;
                        op.vib = value & 0x40 != 0;
                        op.sustain = value & 0x20 != 0;
                        op.ksr = value & 0x10 != 0;
                        op.mult = value & 0x0F;
                    }
                    0x40 => {
                        op.ksl = value >> 6;
                        op.tl = value & 0x3F;
                    }
                    0x60 => {
                        op.ar = value >> 4;
                        op.dr = value & 0x0F;
                    }
                    0x80 => {
                        op.sl = value >> 4;
                        op.rr = value & 0x0F;
                    }
                    _ => op.wave = value & 0x03,
                }
            }
            0xA0..=0xA8 => {
                let c = &mut self.chans[(reg - 0xA0) as usize];
                c.fnum = (c.fnum & 0x300) | value as u16;
            }
            0xB0..=0xB8 => {
                let ch = (reg - 0xB0) as usize;
                let c = &mut self.chans[ch];
                c.fnum = (c.fnum & 0xFF) | ((value as u16 & 3) << 8);
                c.block = (value >> 2) & 7;
                let on = value & 0x20 != 0;
                let (m, k) = Self::channel_slots(ch);
                self.set_key(m, 1, on);
                self.set_key(k, 1, on);
            }
            0xBD => {
                self.deep_am = value & 0x80 != 0;
                self.deep_vib = value & 0x40 != 0;
                self.rhythm = value & 0x20 != 0;
                let keys = if self.rhythm { value & 0x1F } else { 0 };
                self.rhythm_keys = keys;
                // Bass drum: both operators of channel 6. Then hi-hat, tom-tom, snare, cymbal.
                self.set_key(12, 2, keys & 0x10 != 0);
                self.set_key(15, 2, keys & 0x10 != 0);
                self.set_key(13, 2, keys & 0x01 != 0);
                self.set_key(14, 2, keys & 0x04 != 0);
                self.set_key(16, 2, keys & 0x08 != 0);
                self.set_key(17, 2, keys & 0x02 != 0);
            }
            0xC0..=0xC8 => {
                let c = &mut self.chans[(reg - 0xC0) as usize];
                c.feedback = (value >> 1) & 7;
                c.additive = value & 1 != 0;
            }
            _ => {}
        }
    }

    /// Modulator and carrier slot of a channel.
    fn channel_slots(ch: usize) -> (usize, usize) {
        let m = (ch / 3) * 6 + ch % 3;
        (m, m + 3)
    }

    fn channel_of_slot(slot: usize) -> usize {
        (slot / 6) * 3 + (slot % 6) % 3
    }

    /// Key-scale rate offset for a channel (0–15 with KSR set, else 0–3).
    fn key_scale(&self, ch: usize, ksr: bool) -> u32 {
        let c = &self.chans[ch];
        let bit = if self.note_select { (c.fnum >> 8) & 1 } else { (c.fnum >> 9) & 1 };
        let ks = ((c.block as u32) << 1) | bit as u32;
        if ksr {
            ks
        } else {
            ks >> 2
        }
    }

    fn effective_rate(&self, slot: usize, reg_rate: u8) -> u32 {
        if reg_rate == 0 {
            return 0;
        }
        let op = &self.ops[slot];
        let ks = self.key_scale(Self::channel_of_slot(slot), op.ksr);
        (reg_rate as u32 * 4 + ks).min(63)
    }

    fn set_key(&mut self, slot: usize, source: u8, on: bool) {
        let was = self.ops[slot].key;
        let now = if on { was | source } else { was & !source };
        self.ops[slot].key = now;
        if was == 0 && now != 0 {
            let instant = self.effective_rate(slot, self.ops[slot].ar) >= 60;
            let op = &mut self.ops[slot];
            op.stage = Stage::Attack;
            op.phase = 0;
            if instant {
                op.level = 0;
            }
        } else if was != 0 && now == 0 {
            self.ops[slot].stage = Stage::Release;
        }
    }

    /// Advance the timers by one sample. Timer 1 counts every 4 samples (80 µs), timer 2 every
    /// 16 (320 µs).
    fn tick_timers(&mut self) {
        for t in 0..2 {
            let period = if t == 0 { 4 } else { 16 };
            if !self.timer_run[t] || self.clock % period != 0 {
                continue;
            }
            let (v, overflow) = self.timer_count[t].overflowing_add(1);
            self.timer_count[t] = if overflow { self.timer_reload[t] } else { v };
            if overflow && !self.timer_mask[t] {
                self.status |= 0x80 | (0x40 >> t);
            }
        }
    }

    /// Attenuation of one operator this sample, 0 (loudest) to 511, in 0.1875 dB steps.
    fn attenuation(&self, slot: usize, tremolo: u32) -> u32 {
        let op = &self.ops[slot];
        let c = &self.chans[Self::channel_of_slot(slot)];
        let ksl = ((KSL_ROM[(c.fnum >> 6) as usize] << 2) - ((8 - c.block as i32) << 5)).max(0) as u32;
        let mut a = op.level as u32 + ((op.tl as u32) << 2) + (ksl >> KSL_SHIFT[op.ksl as usize]);
        if op.am {
            a += tremolo;
        }
        a.min(0x1FF)
    }

    /// Step one operator's envelope generator.
    fn step_envelope(&mut self, slot: usize, odd: bool, timer_add: u32) {
        let op = self.ops[slot];
        let sl = if op.sl == 15 { 31 } else { op.sl as u16 };
        let mut stage = op.stage;
        if stage == Stage::Attack && op.level == 0 {
            stage = Stage::Decay;
        }
        if stage == Stage::Decay && op.level >> 4 == sl {
            stage = Stage::Sustain;
        }
        let reg_rate = match stage {
            Stage::Attack => op.ar,
            Stage::Decay => op.dr,
            Stage::Sustain => {
                if op.sustain {
                    0
                } else {
                    op.rr
                }
            }
            Stage::Release => op.rr,
        };
        let rate = self.effective_rate(slot, reg_rate);
        let (hi, lo) = (rate >> 2, rate & 3);
        let shift: u32 = if rate == 0 {
            0
        } else if hi < 12 {
            if !odd {
                0
            } else {
                match hi + timer_add {
                    12 => 1,
                    13 => (lo >> 1) & 1,
                    14 => lo & 1,
                    _ => 0,
                }
            }
        } else {
            let s = (hi - 12) + RATE_STEP[lo as usize][(self.env_counter & 3) as usize] as u32;
            if s & 4 != 0 {
                4
            } else if s == 0 {
                odd as u32
            } else {
                s
            }
        };
        let mut level = op.level as i32;
        match stage {
            Stage::Attack => {
                if shift > 0 && hi < 15 {
                    level += ((!level) << shift) >> 5;
                } else if hi == 15 {
                    level = 0;
                }
                level = level.max(0);
            }
            _ => {
                if level & 0x1F8 == 0x1F8 {
                    level = 0x1FF;
                } else if shift > 0 {
                    level = (level + (1 << (shift - 1))).min(0x1FF);
                }
            }
        }
        let o = &mut self.ops[slot];
        o.level = level as u16;
        o.stage = stage;
    }

    /// 10-bit phase of an operator this sample, then advance its accumulator.
    fn step_phase(&mut self, slot: usize) -> u32 {
        let op = self.ops[slot];
        let c = self.chans[Self::channel_of_slot(slot)];
        let mut f = c.fnum as i32;
        if op.vib {
            let range = (f >> 7) & 7;
            let pos = self.vib_pos;
            let mut v = match pos & 3 {
                0 => 0,
                2 => range,
                _ => range >> 1,
            };
            if !self.deep_vib {
                v >>= 1;
            }
            if pos & 4 != 0 {
                v = -v;
            }
            f += v;
        }
        let base = (((f as u32) & 0x3FF) << c.block) >> 1;
        let inc = (base * MULT_X2[op.mult as usize]) >> 1;
        let out = (op.phase >> 9) & 0x3FF;
        self.ops[slot].phase = (op.phase + inc) & 0x7FFFF;
        out
    }

    /// Operator output (13-bit signed) for a 10-bit phase and a 9-bit attenuation.
    fn wave(&self, wave: u8, phase: u32, atten: u32) -> i32 {
        let p = phase & 0x3FF;
        let w = if self.wave_select { wave } else { 0 };
        let quarter = |p: u32| -> u32 {
            let i = if p & 0x100 != 0 { !p & 0xFF } else { p & 0xFF };
            self.log_sin[i as usize] as u32
        };
        let (log, negative) = match w {
            0 => (quarter(p), p & 0x200 != 0),
            1 => (if p & 0x200 != 0 { SILENT } else { quarter(p) }, false),
            2 => (quarter(p), false),
            _ => (if p & 0x100 != 0 { SILENT } else { quarter(p) }, false),
        };
        let total = (log + (atten << 3)).min(0x1FFF);
        let mag = ((self.exp[(total & 0xFF) as usize] as i32) << 1) >> (total >> 8);
        if negative {
            !mag
        } else {
            mag
        }
    }

    fn feedback_input(&self, slot: usize, ch: usize) -> u32 {
        let fb = self.chans[ch].feedback;
        if fb == 0 {
            return 0;
        }
        let op = &self.ops[slot];
        ((op.prev_out + op.out) >> (9 - fb)) as u32
    }

    fn set_out(&mut self, slot: usize, v: i32) {
        let op = &mut self.ops[slot];
        op.prev_out = op.out;
        op.out = v;
    }

    /// Generate one sample at [`SAMPLE_RATE`].
    pub fn sample(&mut self) -> i16 {
        // Global clocks: tremolo every 64 samples (210 positions, 3.7 Hz), vibrato every 1024
        // (8 positions, 6.1 Hz), the envelope counter every second sample.
        if self.clock & 63 == 63 {
            self.trem_pos = (self.trem_pos + 1) % 210;
        }
        if self.clock & 1023 == 1023 {
            self.vib_pos = (self.vib_pos + 1) & 7;
        }
        let odd = self.clock & 1 == 1;
        let mut timer_add = 0;
        if odd {
            self.env_counter += 1;
            let low = self.env_counter & 0x1FFF;
            timer_add = if low == 0 { 0 } else { low.trailing_zeros() + 1 };
        }
        let tri = if self.trem_pos > 105 { 210 - self.trem_pos } else { self.trem_pos };
        let tremolo = tri >> if self.deep_am { 2 } else { 4 };

        for slot in 0..18 {
            self.step_envelope(slot, odd, timer_add);
        }
        let mut phase = [0u32; 18];
        for (slot, p) in phase.iter_mut().enumerate() {
            *p = self.step_phase(slot);
        }

        let mut mix: i32 = 0;
        let melodic = if self.rhythm { 6 } else { 9 };
        for ch in 0..melodic {
            let (m, k) = Self::channel_slots(ch);
            let fb = self.feedback_input(m, ch);
            let mo = self.wave(self.ops[m].wave, phase[m].wrapping_add(fb), self.attenuation(m, tremolo));
            self.set_out(m, mo);
            let modulation = if self.chans[ch].additive { 0 } else { mo as u32 };
            let co = self.wave(self.ops[k].wave, phase[k].wrapping_add(modulation), self.attenuation(k, tremolo));
            self.set_out(k, co);
            mix += if self.chans[ch].additive { mo + co } else { co };
        }
        if self.rhythm {
            mix += self.rhythm_section(&phase, tremolo);
        }

        // Noise generator: 23-bit LFSR, taps 0 and 14.
        let bit = (self.noise ^ (self.noise >> 14)) & 1;
        self.noise = (self.noise >> 1) | (bit << 22);
        self.tick_timers();
        self.clock += 1;
        mix.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }

    /// Channels 6–8 in rhythm mode: bass drum (both operators of 6), hi-hat (13), tom-tom (14),
    /// snare (16) and cymbal (17). Each percussion output counts double.
    fn rhythm_section(&mut self, phase: &[u32; 18], tremolo: u32) -> i32 {
        // Bass drum: a normal two-operator voice, but only the carrier is heard.
        let fb = self.feedback_input(12, 6);
        let mo = self.wave(self.ops[12].wave, phase[12].wrapping_add(fb), self.attenuation(12, tremolo));
        self.set_out(12, mo);
        let modulation = if self.chans[6].additive { 0 } else { mo as u32 };
        let bd = self.wave(self.ops[15].wave, phase[15].wrapping_add(modulation), self.attenuation(15, tremolo));
        self.set_out(15, bd);

        let hh_p = phase[13];
        let cy_p = phase[17];
        let bit = |p: u32, n: u32| (p >> n) & 1;
        let ring = (bit(hh_p, 2) ^ bit(hh_p, 7)) | (bit(hh_p, 3) ^ bit(cy_p, 5)) | (bit(cy_p, 3) ^ bit(cy_p, 5));
        let noise = self.noise & 1;

        let hh_phase = (ring << 9) | if ring ^ noise != 0 { 0xD0 } else { 0x34 };
        let hh = self.wave(self.ops[13].wave, hh_phase, self.attenuation(13, tremolo));
        self.set_out(13, hh);
        let tt = self.wave(self.ops[14].wave, phase[14], self.attenuation(14, tremolo));
        self.set_out(14, tt);
        let sd_phase = (bit(hh_p, 8) << 9) | ((bit(hh_p, 8) ^ noise) << 8);
        let sd = self.wave(self.ops[16].wave, sd_phase, self.attenuation(16, tremolo));
        self.set_out(16, sd);
        let cy_phase = (ring << 9) | 0x80;
        let cy = self.wave(self.ops[17].wave, cy_phase, self.attenuation(17, tremolo));
        self.set_out(17, cy);

        2 * (bd + hh + tt + sd + cy)
    }

    /// Fill `out` with consecutive samples.
    pub fn generate(&mut self, out: &mut [i16]) {
        for s in out {
            *s = self.sample();
        }
    }

    /// Samples generated since reset.
    pub fn samples_generated(&self) -> u64 {
        self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(opl: &mut Opl2, fnum: u16, block: u8) {
        // Channel 0: a plain sine on the carrier, modulator silent, instant attack, full sustain.
        opl.write(0x20, 0x21);
        opl.write(0x23, 0x21);
        opl.write(0x40, 0x3F);
        opl.write(0x43, 0x00);
        opl.write(0x60, 0xF0);
        opl.write(0x63, 0xF0);
        opl.write(0x80, 0x0F);
        opl.write(0x83, 0x0F);
        opl.write(0xC0, 0x00);
        opl.write(0xA0, fnum as u8);
        opl.write(0xB0, 0x20 | (block << 2) | (fnum >> 8) as u8);
    }

    #[test]
    fn rom_tables_have_known_end_points() {
        let opl = Opl2::new();
        // −log₂ sin at the smallest phase step and at the peak.
        assert_eq!(opl.log_sin[0], 2137);
        assert_eq!(opl.log_sin[255], 0);
        assert_eq!(opl.exp[0], 2042);
        assert_eq!(opl.exp[255], 1024);
    }

    #[test]
    fn silent_after_reset() {
        let mut opl = Opl2::new();
        let mut buf = [0i16; 1000];
        opl.generate(&mut buf);
        assert!(buf.iter().all(|&s| s == 0));
    }

    #[test]
    fn sine_has_expected_pitch_and_peak() {
        // F-number 577, block 4: f = 577 × 49716 / 2^(20−4) ≈ 437.7 Hz.
        let mut opl = Opl2::new();
        tone(&mut opl, 577, 4);
        let mut buf = vec![0i16; SAMPLE_RATE as usize];
        opl.generate(&mut buf);
        let crossings = buf.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as f64;
        let expected = 577.0 * SAMPLE_RATE / (1u32 << 16) as f64;
        assert!((crossings - expected).abs() < 2.0, "{crossings} vs {expected}");
        let peak = buf.iter().map(|&s| (s as i32).abs()).max().unwrap();
        assert!((4000..=4085).contains(&peak), "peak {peak}");
    }

    #[test]
    fn key_off_releases_to_silence() {
        let mut opl = Opl2::new();
        tone(&mut opl, 577, 4);
        let mut buf = vec![0i16; 2000];
        opl.generate(&mut buf);
        opl.write(0x83, 0x0F); // release rate 15: fastest
        opl.write(0xB0, (4 << 2) | 2); // key off
        let mut tail = vec![0i16; 4000];
        opl.generate(&mut tail);
        assert!(tail[3000..].iter().all(|&s| s.abs() <= 1));
    }

    #[test]
    fn timer_one_sets_status() {
        let mut opl = Opl2::new();
        opl.write(0x04, 0x80);
        opl.write(0x02, 0xFF);
        opl.write(0x04, 0x21); // start timer 1, mask timer 2
        assert_eq!(opl.status() & 0xE0, 0);
        let mut buf = [0i16; 8];
        opl.generate(&mut buf);
        assert_eq!(opl.status() & 0xE0, 0xC0);
        opl.write(0x04, 0x80);
        assert_eq!(opl.status(), 0x06);
    }

    #[test]
    fn deterministic() {
        let run = || {
            let mut opl = Opl2::new();
            tone(&mut opl, 345, 5);
            opl.write(0xBD, 0x3F); // rhythm mode, all drums
            let mut buf = vec![0i16; 20_000];
            opl.generate(&mut buf);
            buf
        };
        assert_eq!(run(), run());
    }
}
