//! Count calls of every `EL2MCGA3` vector entry during play, with the instructions each takes
//! (inclusive of nested driver calls and interrupts) and the callers, without changing anything.
//! Runs the machine one instruction at a time, so it is slow (about a minute for 60 s).
//!
//! Usage: cargo run --release -p frontier-hooks --example mcga3_census -- [game-dir] [seconds]

use std::collections::BTreeMap;

use frontier_dos::cpu::{CS, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};

/// Entries of the vector table slots `DS:01d0`–`026f` in `EL2MCGA3` (docs/VIDEO.md).
const ENTRIES: &[(u16, u16)] = &[
    (0x01d0, 0x015b), (0x01d4, 0x2f4e), (0x01d8, 0x2f5a), (0x01dc, 0x3489), (0x01e0, 0x349c),
    (0x01e4, 0x34c9), (0x01e8, 0x34ac), (0x01ec, 0x06ba), (0x01f0, 0x072a), (0x01f4, 0x3399),
    (0x01f8, 0x3343), (0x01fc, 0x3108), (0x0200, 0x5a20), (0x0204, 0x44d1), (0x0208, 0x3635),
    (0x020c, 0x05b8), (0x0210, 0x28f9), (0x0214, 0x2dd1), (0x0218, 0x1a1b), (0x021c, 0x02ce),
    (0x0220, 0x02d3), (0x0224, 0x076e), (0x0228, 0x1e41), (0x022c, 0x1e74), (0x0230, 0x0583),
    (0x0234, 0x5b50), (0x0238, 0x59ce), (0x023c, 0x5a29), (0x0240, 0x04d8), (0x0244, 0x5d4a),
    (0x0248, 0x142a), (0x024c, 0x0811), (0x0250, 0x34e7), (0x0254, 0x0681), (0x0258, 0x012b),
    (0x025c, 0x0796), (0x0260, 0x1b3f), (0x0264, 0x1b62), (0x0268, 0x0788), (0x026c, 0x078f),
];

#[derive(Default)]
struct Stat {
    calls: u64,
    instrs: u64,
    max: u64,
    callers: BTreeMap<String, u64>,
}

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
    let mut stats: BTreeMap<u16, Stat> = BTreeMap::new();
    // Open calls: (slot, linear return address, SP after RETF, icount at entry).
    let mut open: Vec<(u16, u32, u16, u64)> = Vec::new();
    let tick = (m.dev.ips / 100.0) as u64;
    let mut next_script = 0u64;
    let mut stage = 0;
    while m.now() < secs {
        if m.cpu.icount >= next_script {
            next_script += tick;
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
        let here = lin(m.cpu.s[CS], m.cpu.ip) & 0xF_FFFF;
        let sp = m.cpu.r[ESP] as u16;
        while let Some(&(slot, ret, ret_sp, start)) = open.last() {
            if here == ret && sp.wrapping_sub(ret_sp) < 0x8000 {
                open.pop();
                let n = m.cpu.icount - start;
                let s = stats.get_mut(&slot).unwrap();
                s.instrs += n;
                s.max = s.max.max(n);
            } else {
                break;
            }
        }
        if let Some(seg) = m.image_segment("EL2MCGA3.OVL") {
            if m.cpu.s[CS] == seg {
                if let Some(&(slot, _)) = ENTRIES.iter().find(|e| e.1 as u32 == m.cpu.ip) {
                    let ss = m.cpu.s[SS];
                    let ip = m.cpu.mem.rw(lin(ss, sp as u32));
                    let cs = m.cpu.mem.rw(lin(ss, sp.wrapping_add(2) as u32));
                    let ret = lin(cs, ip as u32) & 0xF_FFFF;
                    let who = m
                        .images()
                        .iter()
                        .filter(|im| lin(im.seg, 0) <= ret)
                        .max_by_key(|im| im.seg)
                        .map(|im| format!("{}:{:04x}", im.name.trim_end_matches(".OVL").trim_end_matches(".EXE"), ret - lin(im.seg, 0)))
                        .unwrap_or_default();
                    let s = stats.entry(slot).or_default();
                    s.calls += 1;
                    *s.callers.entry(who).or_default() += 1;
                    open.push((slot, ret, sp.wrapping_add(4), m.cpu.icount));
                }
            }
        }
        m.run(1);
    }
    for &(slot, off) in ENTRIES {
        let s = stats.remove(&slot).unwrap_or_default();
        let avg = if s.calls > 0 { s.instrs / s.calls } else { 0 };
        let mut callers: Vec<_> = s.callers.into_iter().collect();
        callers.sort_by(|a, b| b.1.cmp(&a.1));
        callers.truncate(6);
        println!("{slot:04x} EL2MCGA3:{off:04x} calls {:>8} instrs {:>11} avg {:>7} max {:>8} {:?}", s.calls, s.instrs, avg, s.max, callers);
    }
}
