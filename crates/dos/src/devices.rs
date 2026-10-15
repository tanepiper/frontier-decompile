//! Port-mapped hardware the game touches: the 8259 PIC, the 8253 PIT, the keyboard controller,
//! the VGA DAC/sequencer/CRTC and its status register. Time is virtual: it advances with executed
//! instructions at [`Devices::ips`] instructions per second, so runs are deterministic. The FM
//! sound chip is in [`crate::audio`].

use std::collections::VecDeque;

use crate::audio::Adlib;
use crate::cpu::Io;

pub const PIT_HZ: f64 = 1_193_182.0;

pub struct Pic {
    pub imr: u8,
    pub irr: u8,
    pub isr: u8,
}

impl Pic {
    /// The highest-priority request that may interrupt now, if any.
    pub fn next(&self) -> Option<u8> {
        let req = self.irr & !self.imr;
        for i in 0..8 {
            if self.isr & (1 << i) != 0 {
                return None;
            }
            if req & (1 << i) != 0 {
                return Some(i);
            }
        }
        None
    }

    pub fn ack(&mut self, irq: u8) {
        self.irr &= !(1 << irq);
        self.isr |= 1 << irq;
    }

    fn eoi(&mut self) {
        for i in 0..8 {
            if self.isr & (1 << i) != 0 {
                self.isr &= !(1 << i);
                return;
            }
        }
    }
}

pub struct Pit {
    pub reload: [u16; 3],
    mode: [u8; 3],
    access: [u8; 3],
    write_lo: [bool; 3],
    read_lo: [bool; 3],
    latch: [Option<u16>; 3],
    /// Virtual time (seconds) at which counter 0 was last loaded.
    loaded_at: [f64; 3],
}

impl Pit {
    /// Period of counter 0 (IRQ 0) in seconds.
    pub fn period(&self) -> f64 {
        let r = if self.reload[0] == 0 { 65536.0 } else { self.reload[0] as f64 };
        r / PIT_HZ
    }

    fn count(&self, ch: usize, now: f64) -> u16 {
        let r = if self.reload[ch] == 0 { 65536.0 } else { self.reload[ch] as f64 };
        let ticks = ((now - self.loaded_at[ch]) * PIT_HZ) as u64;
        let c = r as u64 - (ticks % r as u64);
        if self.mode[ch] == 3 {
            // Square wave counts down by two, twice per period.
            ((c * 2) % r as u64) as u16
        } else {
            c as u16
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vga {
    /// 6-bit DAC values, 256 × RGB.
    pub dac: [[u8; 3]; 256],
    dac_write: u8,
    dac_read: u8,
    dac_comp: u8,
    pub seq: [u8; 8],
    seq_index: u8,
    pub crtc: [u8; 32],
    crtc_index: u8,
    pub gc: [u8; 16],
    gc_index: u8,
    attr_flip: bool,
    status_toggle: bool,
    /// Bumped on every DAC write so hosts can re-upload the palette lazily.
    pub dac_version: u64,
}

pub struct Devices {
    pub pic: Pic,
    pub pit: Pit,
    pub vga: Vga,
    /// Emulated instructions per second (the speed of the virtual PC).
    pub ips: f64,
    /// Virtual time in seconds, updated by the machine before each I/O burst.
    pub now: f64,
    /// Next time IRQ 0 fires.
    pub next_tick: f64,
    pub kbd_queue: VecDeque<u8>,
    pub kbd_data: u8,
    pub kbd_full: bool,
    port61: u8,
    /// Ports read or written that have no model, for the log.
    pub unknown_ports: std::collections::BTreeSet<u16>,
    /// OPL2 FM chip (AdLib / Sound Blaster FM).
    pub adlib: Adlib,
}

impl Devices {
    pub fn new(ips: f64) -> Self {
        Devices {
            pic: Pic { imr: 0xB8, irr: 0, isr: 0 },
            pit: Pit {
                reload: [0; 3],
                mode: [3; 3],
                access: [3; 3],
                write_lo: [true; 3],
                read_lo: [true; 3],
                latch: [None; 3],
                loaded_at: [0.0; 3],
            },
            vga: Vga {
                dac: [[0; 3]; 256],
                dac_write: 0,
                dac_read: 0,
                dac_comp: 0,
                seq: [0; 8],
                seq_index: 0,
                crtc: [0; 32],
                crtc_index: 0,
                gc: [0; 16],
                gc_index: 0,
                attr_flip: false,
                status_toggle: false,
                dac_version: 0,
            },
            ips,
            now: 0.0,
            next_tick: 65536.0 / PIT_HZ,
            kbd_queue: VecDeque::new(),
            kbd_data: 0,
            kbd_full: false,
            port61: 0,
            unknown_ports: Default::default(),
            adlib: Adlib::new(ips),
        }
    }

    pub fn raise_irq(&mut self, irq: u8) {
        self.pic.irr |= 1 << irq;
    }

    /// Move the next queued scancode into the controller and raise IRQ 1 if the previous one
    /// has been taken.
    pub fn pump_keyboard(&mut self) {
        if !self.kbd_full && self.pic.isr & 2 == 0 && self.pic.irr & 2 == 0 {
            if let Some(code) = self.kbd_queue.pop_front() {
                self.kbd_data = code;
                self.kbd_full = true;
                self.raise_irq(1);
            }
        }
    }


    pub fn set_dac(&mut self, index: u8, rgb: [u8; 3]) {
        self.vga.dac[index as usize] = [rgb[0] & 63, rgb[1] & 63, rgb[2] & 63];
        self.vga.dac_version += 1;
    }
}

impl Vga {
    /// Vertical retrace at 70 Hz (mode 13h timing) at virtual time `now`; the last ~5 % of
    /// each frame is retrace.
    pub fn retrace(now: f64) -> bool {
        let frame = 1.0 / 70.086;
        let phase = (now % frame) / frame;
        phase > 0.955
    }

    /// `IN` from a VGA port (3C0h–3DFh) at virtual time `now`.
    pub fn inb(&mut self, port: u16, now: f64) -> u8 {
        match port {
            0x3C7 => self.dac_read,
            0x3C8 => self.dac_write,
            0x3C9 => {
                let v = self.dac[self.dac_read as usize][self.dac_comp as usize];
                self.dac_comp += 1;
                if self.dac_comp == 3 {
                    self.dac_comp = 0;
                    self.dac_read = self.dac_read.wrapping_add(1);
                }
                v
            }
            0x3C4 => self.seq_index,
            0x3C5 => self.seq[(self.seq_index & 7) as usize],
            0x3CE => self.gc_index,
            0x3CF => self.gc[(self.gc_index & 15) as usize],
            0x3D4 => self.crtc_index,
            0x3D5 => self.crtc[(self.crtc_index & 31) as usize],
            0x3DA => {
                self.attr_flip = false;
                self.status_toggle = !self.status_toggle;
                let vr = Vga::retrace(now);
                (if vr { 0x08 | 0x01 } else { 0 }) | self.status_toggle as u8
            }
            _ => 0xFF,
        }
    }

    /// `OUT` to a VGA port (3C0h–3DFh).
    pub fn outb(&mut self, port: u16, v: u8) {
        match port {
            0x3C8 => {
                self.dac_write = v;
                self.dac_comp = 0;
            }
            0x3C7 => {
                self.dac_read = v;
                self.dac_comp = 0;
            }
            0x3C9 => {
                let i = self.dac_write as usize;
                self.dac[i][self.dac_comp as usize] = v & 63;
                self.dac_version += 1;
                self.dac_comp += 1;
                if self.dac_comp == 3 {
                    self.dac_comp = 0;
                    self.dac_write = self.dac_write.wrapping_add(1);
                }
            }
            0x3C6 => {}
            0x3C4 => self.seq_index = v,
            0x3C5 => self.seq[(self.seq_index & 7) as usize] = v,
            0x3CE => self.gc_index = v,
            0x3CF => self.gc[(self.gc_index & 15) as usize] = v,
            0x3D4 => self.crtc_index = v,
            0x3D5 => self.crtc[(self.crtc_index & 31) as usize] = v,
            0x3C0 => self.attr_flip = !self.attr_flip,
            _ => {}
        }
    }
}

impl Io for Devices {
    fn inb(&mut self, port: u16) -> u8 {
        match port {
            0x20 => self.pic.irr,
            0x21 => self.pic.imr,
            0x40..=0x42 => {
                let ch = (port - 0x40) as usize;
                let v = self.pit.latch[ch].unwrap_or_else(|| self.pit.count(ch, self.now));
                match self.pit.access[ch] {
                    1 => {
                        self.pit.latch[ch] = None;
                        v as u8
                    }
                    2 => {
                        self.pit.latch[ch] = None;
                        (v >> 8) as u8
                    }
                    _ => {
                        let lo = self.pit.read_lo[ch];
                        self.pit.read_lo[ch] = !lo;
                        if lo {
                            self.pit.latch[ch] = Some(v);
                            v as u8
                        } else {
                            self.pit.latch[ch] = None;
                            (v >> 8) as u8
                        }
                    }
                }
            }
            0x60 => {
                self.kbd_full = false;
                self.kbd_data
            }
            0x61 => {
                // Bit 4 toggles with DRAM refresh; bit 5 mirrors PIT channel 2 output.
                let refresh = ((self.now * 66_666.0) as u64 & 1) as u8;
                (self.port61 & 0x0F) | (refresh << 4)
            }
            0x64 => 0x14 | self.kbd_full as u8,
            0x3C0..=0x3DF => self.vga.inb(port, self.now),
            0x388 | 0x389 | 0x228 | 0x229 => self.adlib.read(port, self.now),
            0x201 => 0xFF,
            _ => {
                self.unknown_ports.insert(port);
                0xFF
            }
        }
    }

    fn outb(&mut self, port: u16, v: u8) {
        match port {
            0x20 => {
                if v & 0x20 != 0 {
                    self.pic.eoi();
                }
            }
            0x21 => self.pic.imr = v,
            0x40..=0x42 => {
                let ch = (port - 0x40) as usize;
                let r = self.pit.reload[ch];
                let nr = match self.pit.access[ch] {
                    1 => (r & 0xFF00) | v as u16,
                    2 => (r & 0x00FF) | (v as u16) << 8,
                    _ => {
                        let lo = self.pit.write_lo[ch];
                        self.pit.write_lo[ch] = !lo;
                        if lo {
                            (r & 0xFF00) | v as u16
                        } else {
                            (r & 0x00FF) | (v as u16) << 8
                        }
                    }
                };
                self.pit.reload[ch] = nr;
                if self.pit.access[ch] != 3 || self.pit.write_lo[ch] {
                    self.pit.loaded_at[ch] = self.now;
                    if ch == 0 {
                        self.next_tick = self.now + self.pit.period();
                    }
                }
            }
            0x43 => {
                let ch = (v >> 6) as usize;
                if ch == 3 {
                    return;
                }
                let access = (v >> 4) & 3;
                if access == 0 {
                    self.pit.latch[ch] = Some(self.pit.count(ch, self.now));
                    self.pit.read_lo[ch] = true;
                } else {
                    self.pit.access[ch] = access;
                    self.pit.mode[ch] = (v >> 1) & 7;
                    self.pit.write_lo[ch] = true;
                    self.pit.read_lo[ch] = true;
                    self.pit.latch[ch] = None;
                }
            }
            0x61 => self.port61 = v,
            0x3C0..=0x3DF => self.vga.outb(port, v),
            0x388 | 0x389 | 0x228 | 0x229 => self.adlib.write(port, v, self.now),
            _ => {
                self.unknown_ports.insert(port);
            }
        }
    }
}
