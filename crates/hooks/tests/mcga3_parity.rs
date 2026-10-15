//! Parity: every `EL2MCGA3` port in `frontier_game::video` against the original routine, run on
//! the reference runtime inside the booted game (real DS, tables, back buffer, `int 0` handler).
//! Registers, every byte written (the frame buffer included), the VGA port accesses with their
//! timing, the VGA state and the instruction count are compared. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{DS, EAX, EBP, EBX, ECX, EDI, EDX, ES, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::video::{self, EXPORTS};
use frontier_hooks::state;
use frontier_hooks::video::{compare, record_original, run_port, Shadow};

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
    // Past start-up: the driver initialised (back buffer, EL2G.GRF) and the intro drawing.
    m.run(50_000_000);
    assert!(m.image_segment("EL2MCGA3.OVL").is_some(), "EL2MCGA3 not loaded");
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
    fn below(&mut self, n: u16) -> u16 {
        self.next() % n
    }
    fn dword(&mut self) -> u32 {
        (self.next() as u32) << 16 | self.next() as u32
    }
    /// A coordinate: mostly on screen, sometimes just off it, sometimes anything.
    fn coord(&mut self, size: u16) -> u16 {
        match self.below(10) {
            0 => self.next(),
            1 => self.below(64).wrapping_sub(32),
            2 => size.wrapping_sub(16).wrapping_add(self.below(48)),
            3 => self.below(size + 200).wrapping_sub(100),
            _ => self.below(size),
        }
    }
}

/// Scratch stack segment, clear of everything the routines touch.
const STACK: u16 = 0x9000;

/// Inputs for one call of export `name`; returns false if this export has no generator.
fn setup(m: &mut Machine, rng: &mut Rng, name: &str, ds: u16) -> bool {
    let target = if rng.below(2) == 0 { 0xA000 } else { m.cpu.mem.rw(lin(ds, video::BACK_BUFFER as u32)) };
    match name {
        "triangle" | "triangle_clipped" => {
            let inside = name == "triangle";
            let (w, h) = (video::WIDTH, video::VIEW_HEIGHT);
            let c = |r: &mut Rng, s: u16| if inside { r.below(s) } else { r.coord(s) };
            let v = [c(rng, w), c(rng, h), c(rng, w), c(rng, h), c(rng, w), c(rng, h)];
            for (r, val) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            m.cpu.s[ES] = rng.next();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            // Stale edge starts are read when an edge is horizontal.
            for a in [video::START_A, video::START_B] {
                m.cpu.mem.wd(lin(ds, a as u32), rng.dword());
            }
            true
        }
        _ => false,
    }
}

#[test]
fn mcga3_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let cases: usize = std::env::var("MCGA3_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(3000);
    let only = std::env::var("MCGA3_ONLY").ok();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut shadow = Shadow::default();
    let mut failures = Vec::new();
    for &(off, slot, name, port) in EXPORTS {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let (mut mismatches, mut declined, mut ran) = (0, 0, 0);
        for _ in 0..cases {
            for r in [EDI] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.r[ESP] = 0xFF00;
            m.cpu.s[DS] = ds;
            m.cpu.s[SS] = STACK;
            m.cpu.flags &= !frontier_dos::cpu::DF;
            if !setup(&mut m, &mut rng, name, ds) {
                break;
            }
            m.enter(seg, off);
            let entry = state(&m.cpu);
            let out = match run_port(&m.cpu, &m.dev.vga, m.dev.ips, &mut shadow, port) {
                Ok(o) => o,
                Err(_) => {
                    declined += 1;
                    continue;
                }
            };
            ran += 1;
            let diff = match record_original(&mut m.cpu, &mut m.dev, 50_000_000) {
                Some(orig) => compare(&out, &shadow, &orig, &m.cpu.mem, &m.dev.vga),
                None => Some(format!("the original did not return; the port claims {} steps", out.steps)),
            };
            if let Some(d) = diff {
                if mismatches < 3 {
                    failures.push(format!("{name} (EL2MCGA3:{off:04x}, slot {slot:04x}): input {entry:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        eprintln!("{name}: {ran} compared, {declined} declined, {mismatches} mismatches");
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{ran} mismatches"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
