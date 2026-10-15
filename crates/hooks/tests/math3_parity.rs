//! Parity: every `EL2MATH3` port in `frontier_game::math3` against the original routine, run on
//! the reference runtime inside the booted game (real DS and tables, real `int 0` handler).
//! Registers, every byte written and the instruction count are compared. Skips when game-data/
//! is absent.

use std::path::Path;

use frontier_dos::cpu::{DS, EAX, EBP, ECX, EDI, EDX, ES, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::math3::{ANGLE_A, ANGLE_B, DECLINE, PORTS};
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
        let mut declined = 0;
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
            if name == "project" && rng.next() % 4 != 0 {
                // Mostly points in front of the eye, at every depth scale.
                let shift = rng.next() % 24;
                m.cpu.r[ECX] = (rng.dword() & 0x7FFF_FFFF) >> shift;
            }
            m.enter(seg, off);
            let input = regs_from(&m.cpu);
            let out = run_port(&m.cpu, port);
            if out.steps == DECLINE {
                // Not handled by the port (a path not ported yet, or an input where the original
                // never returns, e.g. EL2MATH3:1e80 with AX = 8000h).
                declined += 1;
                continue;
            }
            let entry = state(&m.cpu);
            let diff = match record_original(&mut m.cpu, 1_000_000) {
                Some(orig) => compare(&out, &orig, &m.cpu.mem),
                None => Some(format!("the original never returns; the port claims {} steps", out.steps)),
            };
            if let Some(d) = diff {
                if std::env::var("MATH3_DEBUG").is_ok_and(|n| n == name) {
                    // The vertex definition the routine dispatched on (EL2MATH3:0d3a).
                    let bp = entry.r[5] as u16;
                    let defs = bp.wrapping_add(0xff24);
                    let si = m.cpu.mem.rw(lin(entry.ss, defs as u32));
                    let fs = m.cpu.mem.rw(lin(entry.ss, defs.wrapping_add(2) as u32));
                    let ax = (entry.r[0] as u16) & 0xffdf;
                    let w = m.cpu.mem.rw(lin(fs, si.wrapping_add(ax >> 4) as u32));
                    eprintln!("{name}: ax {ax:04x} type {:02x} def {w:04x}: {d}", w >> 8);
                }
                if mismatches == 0 {
                    failures.push(format!("{name} (EL2MATH3:{off:04x}, slot {slot:04x}): input {input:x?} {entry:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        if declined > 0 {
            eprintln!("{name}: {declined}/{cases} inputs declined by the port");
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

/// The vertex routines (`v0300`, `v02fc`, `v0304`) on consistent synthetic models: a frame far
/// above the stack pointer, definitions of every type whose references name valid vertices,
/// records in every state. Random registers alone rarely reach the deeper types.
#[test]
fn vertex_types_match_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MATH3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let ports: Vec<_> = PORTS.iter().filter(|p| matches!(p.2, "transform_vertex" | "project_vertex" | "vertex_position")).collect();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    const VERTICES: u16 = 32;
    const RECORDS: u16 = 0x9000;
    const BP: u16 = 0xE000;
    const MODEL: u16 = OUTPUT;
    let mut failures = Vec::new();
    let mut per_type = [0u32; 256];
    for &&(off, _slot, name, port) in &ports {
        let (mut mismatches, mut declined, cases) = (0, 0, 6000);
        for _ in 0..cases {
            let mem = &mut m.cpu.mem;
            let w = |mem: &mut frontier_dos::mem::Memory, seg: u16, off: u16, v: u16| mem.ww(lin(seg, off as u32), v);
            // The frame: matrix, translation, scale, mark, far pointers.
            for k in 0..9 {
                let v = rng.next();
                w(mem, STACK, BP.wrapping_sub(0x24) + 2 * k, v);
            }
            for k in 0..6 {
                let v = rng.next();
                w(mem, STACK, BP.wrapping_sub(0x10) + 2 * k, v);
            }
            w(mem, STACK, BP.wrapping_sub(0xd4), rng.next() % 0x18);
            let mark = (rng.next() % 254 + 2) as u8;
            mem.wb(lin(STACK, BP.wrapping_sub(0x9a) as u32), mark);
            w(mem, STACK, BP.wrapping_sub(0xdc), 0x0000);
            w(mem, STACK, BP.wrapping_sub(0xda), MODEL);
            w(mem, STACK, BP.wrapping_sub(4), 0x0800);
            w(mem, STACK, BP.wrapping_sub(2), MODEL);
            w(mem, STACK, BP.wrapping_sub(0xd8), 0x1000);
            w(mem, STACK, BP.wrapping_sub(0xd6), MODEL);
            for k in 0..0x40 {
                let v = rng.next();
                w(mem, STACK, BP.wrapping_sub(0x40) + 2 * k, v);
                let v = rng.next();
                w(mem, MODEL, 0x1072 + 2 * k, v);
            }
            let len = rng.next();
            w(mem, MODEL, 0x0816, len);
            // Definitions, two words per mirror pair.
            for pair in 0..VERTICES / 2 {
                let kind: u8 = match rng.next() % 26 {
                    k @ 0..=21 => k as u8,
                    22 => 0x80 | rng.next() as u8,
                    _ => rng.next() as u8,
                };
                let idx = |r: &mut Rng| (r.next() % VERTICES) as u8;
                let (bl, dx) = match kind {
                    3 | 4 | 11..=20 => (idx(&mut rng), (idx(&mut rng) as u16) << 8 | idx(&mut rng) as u16),
                    5..=8 => (rng.next() as u8, idx(&mut rng) as u16),
                    _ => (rng.next() as u8, rng.next()),
                };
                let bl = if matches!(kind, 19 | 20) { rng.next() as u8 } else { bl };
                w(mem, MODEL, pair * 4, (kind as u16) << 8 | bl as u16);
                w(mem, MODEL, pair * 4 + 2, dx);
            }
            // Records in every state.
            for v in 0..VERTICES {
                let r = RECORDS + v * 0x20;
                for k in 0..16 {
                    let x = rng.next();
                    w(mem, STACK, r + 2 * k, x);
                }
                let state = match rng.next() % 5 {
                    0 | 1 => 0,
                    2 => 0xffff,
                    3 => 0x0100 | mark as u16,
                    _ => rng.next(),
                };
                w(mem, STACK, r + 0x12, state);
            }
            w(mem, ds, 0x1f39, RECORDS);
            for k in 0..4 {
                let v = rng.next();
                w(mem, ds, 0x1fd9 + 2 * k, v);
            }
            let vertex = rng.next() % VERTICES;
            let rec = RECORDS + vertex * 0x20;
            for r in 0..8 {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.r[ESP] = 0x8000;
            m.cpu.r[EBP] = (m.cpu.r[EBP] & 0xFFFF_0000) | BP as u32;
            m.cpu.s[DS] = ds;
            m.cpu.s[ES] = OUTPUT;
            m.cpu.s[SS] = STACK;
            m.cpu.flags &= !frontier_dos::cpu::DF;
            let ax = if name == "vertex_position" { vertex } else { vertex << 5 };
            m.cpu.r[EAX] = (m.cpu.r[EAX] & 0xFFFF_0000) | ax as u32;
            m.cpu.r[EDI] = (m.cpu.r[EDI] & 0xFFFF_0000) | rec as u32;
            let st = m.cpu.mem.rw(lin(STACK, rec as u32 + 0x12));
            m.cpu.r[EDX] = (m.cpu.r[EDX] & 0xFFFF_0000) | st as u32;
            let def = m.cpu.mem.rw(lin(MODEL, ((vertex & !1) * 2) as u32));
            m.enter(seg, off);
            let out = run_port(&m.cpu, port);
            if out.steps == DECLINE {
                declined += 1;
                continue;
            }
            per_type[(def >> 8) as usize] += 1;
            let entry = state(&m.cpu);
            let diff = match record_original(&mut m.cpu, 5_000_000) {
                Some(orig) => compare(&out, &orig, &m.cpu.mem),
                None => Some(format!("the original never returns; the port claims {} steps", out.steps)),
            };
            if let Some(d) = diff {
                if std::env::var("MATH3_DEBUG").is_ok() {
                    eprintln!("{name}: vertex {vertex} def {def:04x}: {d}");
                }
                if mismatches == 0 {
                    failures.push(format!("{name}: vertex {vertex} def {def:04x} {entry:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        eprintln!("{name}: {declined}/{cases} declined");
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{cases} mismatches"));
        }
    }
    let covered = (0..=21).filter(|&t| per_type[t] > 0).count();
    eprintln!("vertex types 0-21 reached at the top level: {covered}/22");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
