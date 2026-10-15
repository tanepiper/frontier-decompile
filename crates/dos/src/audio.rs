//! The AdLib / Sound Blaster FM chip on ports 388h–389h (and the Sound Blaster's FM mirror at
//! 228h–229h), rendered from virtual time so a run produces the same samples every time.
//!
//! Before every port access the chip is advanced to the current virtual time, so a register
//! write lands on the sample it would have landed on in hardware (to within one sample, 20 µs).
//! The host drains [`Adlib::samples`] at the chip's native 49,716 Hz and resamples them.
//!
//! Native ports of the sound driver do not execute `OUT` instructions: they push their port
//! writes, stamped with the instruction count at which the original would execute them, onto
//! [`Adlib::pending`]. Those land on exactly the same samples as the original's.

use std::cell::RefCell;
use std::rc::Rc;

use frontier_opl::{Opl2, SAMPLE_RATE};

/// Port writes from native code: (instruction count, port, value), in order.
pub type PortQueue = Rc<RefCell<Vec<(u64, u16, u8)>>>;

pub struct Adlib {
    pub chip: Opl2,
    index: u8,
    /// Mono samples at [`SAMPLE_RATE`] not yet taken by the host.
    pub samples: Vec<i16>,
    /// Stop collecting samples when nobody drains them (headless runs).
    pub capture: bool,
    /// Register writes, for tests and the DOSBox comparison: (sample number, register, value).
    pub write_log: Option<Vec<(u64, u8, u8)>>,
    /// Writes from native ports, applied in order at their instruction counts.
    pub pending: PortQueue,
    /// Instructions per virtual second, to place [`Adlib::pending`] writes in time.
    ips: f64,
}

impl Adlib {
    pub fn new(ips: f64) -> Self {
        Adlib {
            chip: Opl2::new(),
            index: 0,
            samples: Vec::new(),
            capture: false,
            write_log: None,
            pending: Rc::new(RefCell::new(Vec::new())),
            ips,
        }
    }

    /// Generate samples up to virtual time `now` (seconds), applying native writes due by then.
    pub fn advance(&mut self, now: f64) {
        if !self.pending.borrow().is_empty() {
            let writes = std::mem::take(&mut *self.pending.borrow_mut());
            for (icount, port, v) in writes {
                let t = icount as f64 / self.ips;
                self.render_to(t);
                self.apply(port, v);
            }
        }
        self.render_to(now);
    }

    fn render_to(&mut self, now: f64) {
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
        self.apply(port, v);
    }

    fn apply(&mut self, port: u16, v: u8) {
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
