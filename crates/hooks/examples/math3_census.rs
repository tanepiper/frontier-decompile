//! Count calls of every `EL2MATH3` vector entry during play, ported or not, without changing
//! anything (each counting hook declines, so the original runs).
//!
//! Usage: cargo run --release -p frontier-hooks --example math3_census -- [game-dir] [seconds]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use frontier_dos::{Machine, Vfs};

/// Entries of the vector table slots `DS:0270`–`032b` in `EL2MATH3` (docs/OVERLAYS.md §2).
const ENTRIES: &[(u16, u16)] = &[
    (0x0270, 0x2bae), (0x0274, 0x0bcf), (0x0278, 0x0ba1), (0x027c, 0x1d43), (0x0280, 0x1d6d),
    (0x0284, 0x1da1), (0x0288, 0x1d07), (0x028c, 0x1e03), (0x0290, 0x1e22), (0x0294, 0x1e4a),
    (0x0298, 0x1e65), (0x029c, 0x1e34), (0x02a0, 0x1e8d), (0x02a4, 0x1e9a), (0x02a8, 0x1e80),
    (0x02ac, 0x1eaf), (0x02b0, 0x1d2e), (0x02b4, 0x1cf5), (0x02b8, 0x1cfe), (0x02bc, 0x1dd2),
    (0x02c0, 0x1dec), (0x02c4, 0x1db6), (0x02c8, 0x0922), (0x02cc, 0x0938), (0x02d0, 0x1d41),
    (0x02d4, 0x1d63), (0x02d8, 0x24f2), (0x02dc, 0x0ac9), (0x02e0, 0x09e9), (0x02e4, 0x275b),
    (0x02e8, 0x258f), (0x02ec, 0x26be), (0x02f0, 0x2a0e), (0x02f4, 0x1ebb), (0x02f8, 0x1f1f),
    (0x02fc, 0x0ccc), (0x0300, 0x0d2b), (0x0304, 0x0c35), (0x0308, 0x1bcd), (0x030c, 0x1ba7),
    (0x0310, 0x1763), (0x0314, 0x19c2), (0x0318, 0x2a67), (0x031c, 0x0967), (0x0320, 0x0952),
    (0x0324, 0x1f93), (0x0328, 0x202e),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir(&dir).expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let counts: Rc<RefCell<BTreeMap<u16, u64>>> = Rc::default();
    for &(slot, off) in ENTRIES {
        let counts = counts.clone();
        m.hook("EL2MATH3.OVL", off, Box::new(move |_, _| {
            *counts.borrow_mut().entry(slot).or_default() += 1;
            false
        }));
    }
    let slice = (m.dev.ips / 100.0) as u64;
    let mut stage = 0;
    while m.now() < secs {
        m.run(slice);
        let t = m.now();
        if stage == 0 && t >= 20.0 {
            m.key(0x39);
            m.key(0xB9);
            stage = 1;
        }
        if stage == 1 && t >= 24.0 {
            let (dx, dy) = (124 - m.mouse.x, 90 - m.mouse.y);
            m.mouse_move(dx, dy);
            m.mouse_button(1, true);
            m.mouse_button(1, false);
            stage = 2;
        }
    }
    for &(slot, off) in ENTRIES {
        println!("{slot:04x} EL2MATH3:{off:04x} {}", counts.borrow().get(&slot).copied().unwrap_or(0));
    }
}
