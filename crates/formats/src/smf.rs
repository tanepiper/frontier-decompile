//! Standard MIDI Files: the nine `.ROL` / `.rol` songs that `EL2ROL.OVL` loads for the Roland
//! MT-32 (`EL2ROL:1078`, docs/AUDIO.md §4). Despite the extension they are SMF (`MThd`), not
//! AdLib Visual Composer files. This reader follows the published SMF 1.0 layout; how `EL2ROL`
//! walks the events is not traced yet.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Smf {
    /// 0 single track, 1 simultaneous tracks, 2 independent sequences.
    pub format: u16,
    /// Ticks per quarter note, or a negative SMPTE frame rate in the high byte.
    pub division: u16,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Ticks since the previous event of the track.
    pub delta: u32,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// A channel message: status byte (with channel) and its one or two data bytes.
    Channel { status: u8, data: [u8; 2] },
    /// `FF type len data`.
    Meta { kind: u8, data: Vec<u8> },
    /// `F0` / `F7` system exclusive.
    SysEx { data: Vec<u8> },
}

impl EventKind {
    /// Human-readable one-line description.
    pub fn describe(&self) -> String {
        match self {
            EventKind::Channel { status, data } => {
                let ch = (status & 15) + 1;
                match status >> 4 {
                    0x8 => format!("ch{ch:<2} note off  {:3} vel {}", data[0], data[1]),
                    0x9 if data[1] == 0 => format!("ch{ch:<2} note off  {:3}", data[0]),
                    0x9 => format!("ch{ch:<2} note on   {:3} vel {}", data[0], data[1]),
                    0xA => format!("ch{ch:<2} aftertouch {:3} {}", data[0], data[1]),
                    0xB => format!("ch{ch:<2} control   {:3} = {}", data[0], data[1]),
                    0xC => format!("ch{ch:<2} program   {}", data[0]),
                    0xD => format!("ch{ch:<2} pressure  {}", data[0]),
                    _ => format!("ch{ch:<2} pitch bend {}", (data[0] as i32 | (data[1] as i32) << 7) - 8192),
                }
            }
            EventKind::Meta { kind, data } => match kind {
                0x03 => format!("track name \"{}\"", String::from_utf8_lossy(data)),
                0x01 | 0x02 | 0x04 | 0x05 | 0x06 => format!("text {kind:02X} \"{}\"", String::from_utf8_lossy(data)),
                0x2F => "end of track".into(),
                0x51 if data.len() == 3 => {
                    let us = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32;
                    format!("tempo {us} µs/quarter ({:.1} bpm)", 60_000_000.0 / us.max(1) as f64)
                }
                0x58 if data.len() >= 2 => format!("time signature {}/{}", data[0], 1u32 << data[1].min(7)),
                _ => format!("meta {kind:02X} ({} bytes)", data.len()),
            },
            EventKind::SysEx { data } => format!("sysex ({} bytes)", data.len()),
        }
    }
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.d.get(self.p).ok_or(Error::Truncated { needed: self.p + 1, available: self.d.len() })?;
        self.p += 1;
        Ok(b)
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.d.get(self.p..self.p + n).ok_or(Error::Truncated { needed: self.p + n, available: self.d.len() })?;
        self.p += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn var(&mut self) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.byte()?;
            v = v << 7 | (b & 0x7f) as u32;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(Error::Invalid("variable-length quantity longer than 4 bytes"))
    }
}

/// Data bytes that follow a channel status.
fn data_len(status: u8) -> usize {
    match status >> 4 {
        0xC | 0xD => 1,
        _ => 2,
    }
}

pub fn parse(data: &[u8]) -> Result<Smf> {
    let mut r = Reader { d: data, p: 0 };
    if r.bytes(4)? != b"MThd" {
        return Err(Error::Invalid("missing MThd"));
    }
    let hlen = r.u32()? as usize;
    let format = r.u16()?;
    let ntracks = r.u16()?;
    let division = r.u16()?;
    r.p = 8 + hlen;
    let mut tracks = Vec::new();
    for _ in 0..ntracks {
        let id = r.bytes(4)?.to_vec();
        let len = r.u32()? as usize;
        let start = r.p;
        let end =
            start.checked_add(len).filter(|&e| e <= data.len()).ok_or(Error::Truncated { needed: start + len, available: data.len() })?;
        if id != b"MTrk" {
            r.p = end;
            continue;
        }
        let mut t = Reader { d: &data[..end], p: start };
        let mut events = Vec::new();
        let mut running = 0u8;
        while t.p < end {
            let delta = t.var()?;
            let mut status = t.byte()?;
            let kind = if status == 0xff {
                let kind = t.byte()?;
                let n = t.var()? as usize;
                EventKind::Meta { kind, data: t.bytes(n)?.to_vec() }
            } else if status == 0xf0 || status == 0xf7 {
                let n = t.var()? as usize;
                EventKind::SysEx { data: t.bytes(n)?.to_vec() }
            } else {
                let mut d = [0u8; 2];
                if status < 0x80 {
                    // Running status: this byte is the first data byte.
                    d[0] = status;
                    status = running;
                    if status < 0x80 {
                        return Err(Error::Invalid("running status without a previous status"));
                    }
                    if data_len(status) == 2 {
                        d[1] = t.byte()?;
                    }
                } else {
                    running = status;
                    for v in d.iter_mut().take(data_len(status)) {
                        *v = t.byte()?;
                    }
                }
                EventKind::Channel { status, data: d }
            };
            let end_of_track = matches!(kind, EventKind::Meta { kind: 0x2f, .. });
            events.push(Event { delta, kind });
            if end_of_track {
                break;
            }
        }
        tracks.push(Track { events });
        r.p = end;
    }
    Ok(Smf { format, division, tracks })
}

/// A note or controller change on the song's time line, in seconds, merged over all tracks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timed {
    pub time: f64,
    pub status: u8,
    pub data: [u8; 2],
}

impl Smf {
    /// Every channel message in time order, with tempo changes applied (format 0 and 1; a
    /// format-2 file is played track after track).
    pub fn timeline(&self) -> Vec<Timed> {
        let ppq = if self.division & 0x8000 == 0 { self.division.max(1) as f64 } else { 96.0 };
        // (absolute tick, track, index, event)
        let mut all: Vec<(u64, usize, usize, &EventKind)> = Vec::new();
        let mut base = 0u64;
        for (ti, t) in self.tracks.iter().enumerate() {
            let mut tick = base;
            for (i, e) in t.events.iter().enumerate() {
                tick += e.delta as u64;
                all.push((tick, ti, i, &e.kind));
            }
            if self.format == 2 {
                base = tick;
            }
        }
        all.sort_by_key(|&(tick, ti, i, _)| (tick, ti, i));
        let mut out = Vec::new();
        let (mut last_tick, mut seconds, mut us_per_q) = (0u64, 0.0f64, 500_000.0f64);
        for (tick, _, _, k) in all {
            seconds += (tick - last_tick) as f64 * us_per_q / ppq / 1e6;
            last_tick = tick;
            match k {
                EventKind::Meta { kind: 0x51, data } if data.len() == 3 => {
                    us_per_q = ((data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32) as f64;
                }
                EventKind::Channel { status, data } => out.push(Timed { time: seconds, status: *status, data: *data }),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_status_and_tempo() {
        let mut f = b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec();
        let trk: &[u8] = &[
            0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, // tempo 500000
            0x00, 0x90, 60, 100, // note on
            0x60, 60, 0, // running status: note off after one quarter
            0x00, 0xff, 0x2f, 0x00,
        ];
        f.extend_from_slice(b"MTrk");
        f.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        f.extend_from_slice(trk);
        let s = parse(&f).unwrap();
        assert_eq!(s.tracks[0].events.len(), 4);
        let tl = s.timeline();
        assert_eq!(tl.len(), 2);
        assert!((tl[1].time - 0.5).abs() < 1e-9);
        assert_eq!(tl[1].data, [60, 0]);
    }
}
