//! The AdLib / Sound Blaster FM chip on ports 388h–389h (and the Sound Blaster's FM mirror at
//! 228h–229h), rendered from virtual time so a run produces the same samples every time.
//!
//! Before every port access the chip is advanced to the current virtual time, so a register
//! write lands on the sample it would have landed on in hardware (to within one sample, 20 µs).
//! The host drains [`Adlib::samples`] at the chip's native 49,716 Hz and resamples them.

use frontier_opl::{Opl2, SAMPLE_RATE};

pub struct Adlib {
    pub chip: Opl2,
    index: u8,
    /// Mono samples at [`SAMPLE_RATE`] not yet taken by the host.
    pub samples: Vec<i16>,
    /// Stop collecting samples when nobody drains them (headless runs).
    pub capture: bool,
    /// Register writes, for tests and the DOSBox comparison: (sample number, register, value).
    pub write_log: Option<Vec<(u64, u8, u8)>>,
}

impl Default for Adlib {
    fn default() -> Self {
        Self::new()
    }
}

impl Adlib {
    pub fn new() -> Self {
        Adlib { chip: Opl2::new(), index: 0, samples: Vec::new(), capture: false, write_log: None }
    }

    /// Generate samples up to virtual time `now` (seconds).
    pub fn advance(&mut self, now: f64) {
        let target = (now * SAMPLE_RATE) as u64;
        let have = self.chip.samples_generated();
        if target <= have {
            return;
        }
        let n = (target - have) as usize;
        if self.capture {
            let start = self.samples.len();
            self.samples.resize(start + n, 0);
            self.chip.generate(&mut self.samples[start..]);
        } else {
            for _ in 0..n {
                self.chip.sample();
            }
        }
    }

    pub fn read(&mut self, port: u16, now: f64) -> u8 {
        self.advance(now);
        if port & 1 == 0 {
            self.chip.status()
        } else {
            // The data port is write-only; the bus floats.
            0xFF
        }
    }

    pub fn write(&mut self, port: u16, v: u8, now: f64) {
        self.advance(now);
        if port & 1 == 0 {
            self.index = v;
        } else {
            if let Some(log) = &mut self.write_log {
                log.push((self.chip.samples_generated(), self.index, v));
            }
            self.chip.write(self.index, v);
        }
    }
}
