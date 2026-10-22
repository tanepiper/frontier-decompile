//! Code coverage of the original program, for finding the code an action runs (docs/COVERAGE.md).
//!
//! Off by default. When on, the CPU sets one bit per linear address for every instruction it
//! starts, and a second bit for every address entered by a call, a far jump or an interrupt
//! (function entries, used to seed Ghidra). [`crate::Machine`] moves the bits into per-image
//! sets of offsets whenever a program image is loaded or freed, so an overlay evicted and
//! reloaded at another segment still reports offsets in its own file's numbering.

use std::collections::{BTreeMap, BTreeSet};

const WORDS: usize = 0x10_0000 / 64;

/// The CPU's raw bitmaps, indexed by linear address (1 MB).
pub struct Recorder {
    exec: Vec<u64>,
    entries: Vec<u64>,
    /// The next instruction started is the target of a call, far jump or interrupt.
    pub(crate) entry_pending: bool,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder { exec: vec![0; WORDS], entries: vec![0; WORDS], entry_pending: false }
    }
}

impl Recorder {
    #[inline(always)]
    pub(crate) fn mark(&mut self, l: u32) {
        let (w, b) = ((l >> 6) as usize, 1u64 << (l & 63));
        self.exec[w] |= b;
        if std::mem::take(&mut self.entry_pending) {
            self.entries[w] |= b;
        }
    }

    /// Hand every set bit to `f(linear, is_entry)` and clear the bitmaps.
    pub(crate) fn drain(&mut self, mut f: impl FnMut(u32, bool)) {
        for w in 0..WORDS {
            let mut bits = std::mem::take(&mut self.exec[w]);
            let entries = std::mem::take(&mut self.entries[w]);
            while bits != 0 {
                let i = bits.trailing_zeros();
                bits &= bits - 1;
                f((w as u32) << 6 | i, entries & (1 << i) != 0);
            }
        }
    }
}

/// What ran in one program image: offsets from its load segment (PSP + 10h), so Ghidra's flat
/// address is `0x10000 + offset` (load segment `1000`).
#[derive(Debug, Default, Clone)]
pub struct ImageCoverage {
    /// Start of every instruction executed.
    pub exec: BTreeSet<u32>,
    /// Every address entered by a call, a far jump or an interrupt.
    pub entries: BTreeSet<u32>,
}

/// Coverage per image name (`FRONTIER.EXE`, `EL2M08.OVL`); code outside every image (the
/// runtime's BIOS stubs) is filed under `"linear"` with linear addresses.
pub type CoverageMap = BTreeMap<String, ImageCoverage>;
