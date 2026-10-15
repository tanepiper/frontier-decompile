//! Parity: every `EL2MATH3` port in `frontier_game::math3` against the original routine, run on
//! the reference runtime inside the booted game (real DS and tables, real `int 0` handler).
//! Registers, every byte written and the instruction count are compared. Skips when game-data/
//! is absent.

use std::path::Path;

use frontier_dos::cpu::{DS, ES, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::math3::{ANGLE_A, ANGLE_B, PORTS};
use frontier_hooks::{compare, record_original, regs_from, run_port, state, DIVIDE_RESUME};

fn booted() -> Option<Machine> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/original");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: {} missing (run tools/extract-assets.sh)", dir.display());
        return None;
    };
    let mut vfs = Vfs::new();
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    // Past start-up: EL2MATH3 resident and the int 0 handler installed (docs/RUNTIME.md).
    m.run(5_000_000);
    assert!(m.image_segment("EL2MATH3.OVL").is_some(), "EL2MATH3 not loaded");
    Some(m)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as u16
    }
    /// Mostly random words, with edge values mixed in.
    fn word(&mut self) -> u16 {
        const EDGE: [u16; 12] = [0, 1, 2, 0x7FFF, 0x8000, 0x8001, 0xFFFF, 0xFFFE, 0x4000, 0xC000, 0x001F, 0x0020];
        match self.next() % 8 {
            0 => EDGE[(self.next() as usize) % EDGE.len()],
            1 => self.next() % 64,
            2 => (self.next() % 64).wrapping_neg(),
            _ => self.next(),
        }
    }
    fn dword(&mut self) -> u32 {
        (self.next() as u32) << 16 | self.word() as u32
    }
}

/// Scratch segments for the stack and for output structures: video memory, which no routine
/// reads. Separate, so a structure never overlaps the return address.
const STACK: u16 = 0xA000;
const OUTPUT: u16 = 0xB000;

#[test]
fn math3_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MATH3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for &(off, slot, name, port) in PORTS {
        let mut mismatches = 0;
        let mut hangs = 0;
        let cases = 4000;
        for _ in 0..cases {
            for r in 0..8 {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.r[ESP] = 0xFFF0;
            m.cpu.s[DS] = ds;
            m.cpu.s[ES] = OUTPUT;
            m.cpu.s[SS] = STACK;
            m.cpu.flags = (m.cpu.flags & !frontier_dos::cpu::DF) | if rng.next() % 8 == 0 { frontier_dos::cpu::DF } else { 0 };
            m.cpu.mem.ww(lin(ds, DIVIDE_RESUME as u32), 0x5A5A);
            m.cpu.mem.ww(lin(ds, ANGLE_A as u32), rng.word());
            m.cpu.mem.ww(lin(ds, ANGLE_B as u32), rng.word());
            m.enter(seg, off);
            let input = regs_from(&m.cpu);
            let entry = (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags);
            let Some(orig) = record_original(&mut m.cpu, 200_000) else {
                // The original never returns for this input (e.g. EL2MATH3:1e80 with AX bit 15
                // set loops forever); outside the routine's domain, so not compared.
                hangs += 1;
                continue;
            };
            // Back to the state before the call, then the port.
            for (&a, &(old, _)) in &orig.writes {
                m.cpu.mem.wb(a, old);
            }
            (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags) = entry;
            let out = run_port(&m.cpu, port);
            if let Some(d) = compare(&out, &orig, &m.cpu.mem) {
                if mismatches == 0 {
                    failures.push(format!("{name} (EL2MATH3:{off:04x}, slot {slot:04x}): input {input:x?} {:x?}\n  {d}", state(&m.cpu)));
                }
                mismatches += 1;
            }
        }
        if hangs > 0 {
            eprintln!("{name}: {hangs}/{cases} inputs never return in the original (skipped)");
        }
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{cases} mismatches"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// With every port installed the game runs from boot into the cockpit (the intro does not call
/// these routines; play does), and the ports are actually used.
#[test]
fn game_runs_with_ports_installed() {
    let Some(mut m) = booted() else { return };
    frontier_hooks::install_all(&mut m);
    let second = m.dev.ips as u64;
    assert_eq!(m.run(20 * second), frontier_dos::Stop::Budget);
    m.key(0x39); // space: skip the intro
    m.key(0xB9);
    assert_eq!(m.run(4 * second), frontier_dos::Stop::Budget);
    let (dx, dy) = (124 - m.mouse.x, 90 - m.mouse.y);
    m.mouse_move(dx, dy); // start option 1, Ross 154
    m.mouse_button(1, true);
    m.mouse_button(1, false);
    assert_eq!(m.run(6 * second), frontier_dos::Stop::Budget);
    assert!(m.hook_calls > 1000, "ported routines ran only {} times", m.hook_calls);
}
