//! Parity: every `EL2MATH3` port in `frontier_game::math3` against the original routine, run on
//! the reference runtime inside the booted game (real DS, real `int 0` handler). Skips when
//! game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::DS;
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::math3::{Regs, PORTS};
use frontier_hooks::{regs_from, regs_to, DIVIDE_RESUME};

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
}

#[test]
fn math3_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MATH3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    m.cpu.s[DS] = ds;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for &(off, slot, name, port) in PORTS {
        let mut mismatches = 0;
        let mut hangs = 0;
        let cases = 4000;
        for _ in 0..cases {
            let input = Regs { ax: rng.word(), bx: rng.word(), cx: rng.word(), dx: rng.word(), si: rng.word(), di: rng.word(), bp: rng.word() };
            m.cpu.mem.ww(lin(ds, DIVIDE_RESUME), 0x5A5A);
            regs_to(&mut m.cpu, &input);
            m.cpu.s[DS] = ds;
            let start = m.cpu.icount;
            match m.call(seg, off, 200_000) {
                Ok(()) => {}
                // The original never returns for this input (e.g. EL2MATH3:1e80 with AX bit 15
                // set loops forever); outside the routine's domain, so not compared.
                Err(frontier_dos::Stop::Budget) => {
                    hangs += 1;
                    m.cpu.r[frontier_dos::cpu::ESP] = 0xFFF0;
                    continue;
                }
                Err(e) => panic!("{name} {input:x?}: {e:?}"),
            }
            let want = regs_from(&m.cpu);
            let want_resume = m.cpu.mem.rw(lin(ds, DIVIDE_RESUME));
            // Instructions of the routine: all executed ones but the host return stub's.
            let want_steps = m.cpu.icount - start - 1;
            let mut got = input;
            let exit = port(&mut got);
            let resume = exit.resume.unwrap_or(0x5A5A);
            if got != want || resume != want_resume || exit.steps != want_steps {
                if mismatches == 0 {
                    failures.push(format!("{name} (EL2MATH3:{off:04x}, slot {slot:04x}): input {input:x?}\n  original {want:x?} [09dc]={want_resume:04x} steps {want_steps}\n  port     {got:x?} [09dc]={resume:04x} steps {}", exit.steps));
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
