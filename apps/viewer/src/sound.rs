//! Sound in the viewer, on the shared audio output (`frontier-platform`).
//!
//! - The AdLib music and effects are played by the original driver's port
//!   (`frontier_game::adlib`) running alone on the user's `EL2ADLIB.OVL` image, ticked at the
//!   game's 202.27 Hz into the OPL2 model: the same notes the game plays.
//! - The `.ROL` MIDI songs were written for a Roland MT-32 (`EL2ROL`, docs/AUDIO.md §4). There
//!   is no MT-32 model here, so they are played on the OPL2 chip with one generic instrument.
//!   Deviation: an approximation for listening only; the original never plays them on an AdLib.

use frontier_formats::{overlay, smf};
use frontier_game::adlib::{self, Driver, Host, Regs};
use frontier_opl::{Opl2, SAMPLE_RATE};
use frontier_platform::audio::Output;

/// IRQ 0 rate the driver is ticked at (`1000:ce5e`: 1,193,182 / 5899).
const TICK_HZ: f64 = 1_193_182.0 / 5899.0;
/// Where the viewer places the driver and the main data segment in its flat memory.
const CS: u16 = 0x1000;
const DS: u16 = 0x3000;
/// Main data segment variables the driver reads (docs/AUDIO.md §1).
const DS_EFFECTS_ON: u16 = 0x4636;
const DS_MUSIC_ON: u16 = 0x463b;
const DS_SONG_ENABLED: u16 = 0x4640;
/// Songs are commands 32h–3Dh (12 entries); effects map through 40 bytes at `EL2ADLIB:0095`.
pub const SONGS: u16 = 12;
pub const EFFECTS: u16 = 40;

/// A flat 1 MB real-mode memory plus the OPL2 chip the driver writes.
struct Machine {
    mem: Vec<u8>,
    opl: Opl2,
    index: u8,
}

impl Host for Machine {
    fn read8(&mut self, a: u32) -> u8 {
        self.mem[a as usize & 0xfffff]
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.mem[a as usize & 0xfffff] = v;
    }
    fn out8(&mut self, _steps: u64, port: u16, v: u8) {
        // Deviation: writes land at the tick they are made in, not at their instruction count.
        match port {
            0x388 => self.index = v,
            0x389 => self.opl.write(self.index, v),
            _ => {}
        }
    }
}

enum Source {
    Adlib { m: Box<Machine>, ticks: f64 },
    Midi(Box<MidiSynth>),
}

pub struct Player {
    out: Option<Output>,
    source: Option<Source>,
    pub now_playing: String,
    samples: Vec<i16>,
}

impl Player {
    pub fn new() -> Player {
        Player { out: None, source: None, now_playing: String::new(), samples: Vec::new() }
    }

    fn ensure_output(&mut self) -> bool {
        if self.out.is_none() {
            self.out = Output::new();
        }
        if let Some(o) = &self.out {
            o.resume();
        }
        self.out.is_some()
    }

    pub fn stop(&mut self) {
        self.source = None;
        self.now_playing.clear();
    }

    pub fn playing(&self) -> bool {
        self.source.is_some()
    }

    /// Start the original driver on `EL2ADLIB.OVL` and play song `n` (0–11) or effect `n`.
    pub fn play_adlib(&mut self, ovl: &[u8], exe: Option<&[u8]>, n: u16, song: bool) -> Result<(), String> {
        if !self.ensure_output() {
            return Err("no audio output".into());
        }
        let m = start_adlib(ovl, exe, n, song)?;
        self.now_playing = format!("EL2ADLIB {} {n}", if song { "song" } else { "effect" });
        self.source = Some(Source::Adlib { m: Box::new(m), ticks: 0.0 });
        Ok(())
    }

    pub fn play_midi(&mut self, name: &str, song: &smf::Smf) -> Result<(), String> {
        if !self.ensure_output() {
            return Err("no audio output".into());
        }
        self.source = Some(Source::Midi(Box::new(MidiSynth::new(song.timeline()))));
        self.now_playing = format!("{name} (OPL2 approximation)");
        Ok(())
    }

    /// Produce `dt` seconds of sound and queue it.
    pub fn pump(&mut self, dt: f64) {
        let Some(out) = &mut self.out else { return };
        self.samples.clear();
        let n = (dt.min(0.1) * SAMPLE_RATE) as usize;
        match &mut self.source {
            Some(Source::Adlib { m, ticks }) => {
                let per_tick = SAMPLE_RATE / TICK_HZ;
                let mut made = 0;
                while made < n {
                    let before = (*ticks * per_tick) as usize;
                    *ticks += 1.0;
                    call(m, Regs::default(), |d| d.tick());
                    let k = (*ticks * per_tick) as usize - before;
                    let start = self.samples.len();
                    self.samples.resize(start + k, 0);
                    m.opl.generate(&mut self.samples[start..]);
                    made += k;
                }
            }
            Some(Source::Midi(s)) => {
                self.samples.resize(n, 0);
                s.render(&mut self.samples);
                if s.finished() {
                    self.source = None;
                    self.now_playing.clear();
                }
            }
            None => {}
        }
        out.pump(&mut self.samples);
    }
}

/// The driver on its own: `EL2ADLIB.OVL`'s image at `CS`, the main data segment from
/// `FRONTIER.EXE` at `DS`, music and effects switched on, then `init` and song or effect `n`.
fn start_adlib(ovl: &[u8], exe: Option<&[u8]>, n: u16, song: bool) -> Result<Machine, String> {
    let img = overlay::image(ovl).map_err(|e| e.to_string())?;
    let mut m = Machine { mem: vec![0; 1 << 20], opl: Opl2::new(), index: 0 };
    let cs = adlib::lin(CS, 0) as usize;
    m.mem[cs..cs + img.len()].copy_from_slice(img);
    // The main data segment as the file holds it, so the driver sees the game's defaults.
    if let Some(exe) = exe {
        if let Ok(h) = frontier_formats::mz::parse(exe) {
            let at = h.header_size() + frontier_formats::model::MAIN_DS * 16;
            let ds = adlib::lin(DS, 0) as usize;
            let n = exe.len().saturating_sub(at).min(0x10000);
            m.mem[ds..ds + n].copy_from_slice(&exe[at..at + n]);
        }
    }
    for (off, v) in [(DS_EFFECTS_ON, 1u8), (DS_MUSIC_ON, 1)] {
        m.mem[adlib::lin(DS, off) as usize] = v;
    }
    for s in 0..SONGS {
        m.mem[adlib::lin(DS, DS_SONG_ENABLED + s) as usize] = 1;
    }
    call(&mut m, Regs::default(), |d| d.init());
    let r = Regs { ax: n, bx: 1, ..Default::default() };
    if song {
        call(&mut m, r, |d| d.play_song());
    } else {
        call(&mut m, r, |d| d.effect());
    }
    Ok(m)
}

fn call(m: &mut Machine, r: Regs, f: impl FnOnce(&mut Driver)) {
    let mut d = Driver::new(m, CS, DS, r);
    f(&mut d);
}

/// MIDI on the OPL2: nine voices, one organ-like two-operator patch, notes allocated to the
/// oldest free voice. Deviation: see the module comment.
pub struct MidiSynth {
    events: Vec<smf::Timed>,
    next: usize,
    time: f64,
    opl: Opl2,
    /// (channel, note, start time) per OPL voice.
    voices: [Option<(u8, u8, f64)>; 9],
}

const OPERATOR: [u8; 9] = [0x00, 0x01, 0x02, 0x08, 0x09, 0x0a, 0x10, 0x11, 0x12];

impl MidiSynth {
    fn new(events: Vec<smf::Timed>) -> MidiSynth {
        let mut opl = Opl2::new();
        opl.write(0x01, 0x20);
        for (v, &op) in OPERATOR.iter().enumerate() {
            for (reg, val) in [
                (0x20, 0x01),
                (0x23, 0x01),
                (0x40, 0x10),
                (0x43, 0x00),
                (0x60, 0xf3),
                (0x63, 0xf2),
                (0x80, 0x45),
                (0x83, 0x56),
                (0xe0, 0),
                (0xe3, 0),
            ] {
                opl.write(reg + op, val);
            }
            opl.write(0xc0 + v as u8, 0x08);
        }
        MidiSynth { events, next: 0, time: 0.0, opl, voices: [None; 9] }
    }

    fn finished(&self) -> bool {
        self.next >= self.events.len() && self.voices.iter().all(Option::is_none)
    }

    fn key(&mut self, v: usize, note: u8, on: bool) {
        let f = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
        let mut block = 0u8;
        let mut fnum = f * (1u64 << 20) as f64 / SAMPLE_RATE;
        while fnum >= 1024.0 && block < 7 {
            fnum /= 2.0;
            block += 1;
        }
        let fnum = fnum.min(1023.0) as u16;
        self.opl.write(0xa0 + v as u8, fnum as u8);
        self.opl.write(0xb0 + v as u8, (on as u8) << 5 | block << 2 | (fnum >> 8) as u8);
    }

    fn event(&mut self, e: smf::Timed) {
        let ch = e.status & 15;
        let off = e.status >> 4 == 8 || (e.status >> 4 == 9 && e.data[1] == 0);
        if off {
            if let Some(v) = self.voices.iter().position(|x| matches!(x, Some((c, n, _)) if *c == ch && *n == e.data[0])) {
                self.key(v, e.data[0], false);
                self.voices[v] = None;
            }
        } else if e.status >> 4 == 9 && ch != 9 {
            let v = self
                .voices
                .iter()
                .position(Option::is_none)
                .unwrap_or_else(|| (0..9).min_by(|&a, &b| self.voices[a].unwrap().2.total_cmp(&self.voices[b].unwrap().2)).unwrap());
            if let Some((_, n, _)) = self.voices[v] {
                self.key(v, n, false);
            }
            let level = 0x3f - (e.data[1] >> 1).min(0x3f);
            self.opl.write(0x43 + OPERATOR[v], level / 2);
            self.key(v, e.data[0], true);
            self.voices[v] = Some((ch, e.data[0], self.time));
        }
    }

    fn render(&mut self, out: &mut [i16]) {
        let mut done = 0;
        while done < out.len() {
            let next_t = self.events.get(self.next).map(|e| e.time).unwrap_or(f64::MAX);
            let until = ((next_t - self.time) * SAMPLE_RATE).ceil().max(0.0) as usize;
            let k = until.min(out.len() - done);
            self.opl.generate(&mut out[done..done + k]);
            done += k;
            self.time += k as f64 / SAMPLE_RATE;
            while self.next < self.events.len() && self.events[self.next].time <= self.time {
                let e = self.events[self.next];
                self.next += 1;
                self.event(e);
            }
            if self.next >= self.events.len() && k == 0 {
                self.opl.generate(&mut out[done..]);
                for v in 0..9 {
                    if let Some((_, n, _)) = self.voices[v].take() {
                        self.key(v, n, false);
                    }
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The driver, run on the user's own files, starts every song and makes the chip sound.
    /// Skips without game-data/.
    #[test]
    fn songs_make_sound() {
        let dir = std::path::Path::new("../../game-data/original");
        let (Ok(ovl), Ok(exe)) = (std::fs::read(dir.join("EL2ADLIB.OVL")), std::fs::read(dir.join("FRONTIER.EXE"))) else {
            eprintln!("skipping: game-data/original missing");
            return;
        };
        for n in 0..SONGS {
            // Songs 8–11 start after a longer lead-in (measured: within 20 s).
            let ticks = if n >= 8 { 4000 } else { 405 };
            let mut m = start_adlib(&ovl, Some(&exe), n, true).unwrap();
            let mut peak = 0i16;
            let mut buf = vec![0i16; 246];
            for _ in 0..ticks {
                call(&mut m, Regs::default(), |d| d.tick());
                m.opl.generate(&mut buf);
                peak = peak.max(buf.iter().map(|s| s.saturating_abs()).max().unwrap());
            }
            assert!(peak > 500, "song {n}: peak {peak}");
        }
    }
}
