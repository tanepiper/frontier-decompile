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
        "triangle" | "triangle_clipped" | "quad" | "quad_clipped" => {
            let inside = name == "triangle" || name == "quad";
            let (w, h) = (video::WIDTH, video::VIEW_HEIGHT);
            let c = |r: &mut Rng, s: u16| if inside { r.below(s) } else { r.coord(s) };
            let v = [c(rng, w), c(rng, h), c(rng, w), c(rng, h), c(rng, w), c(rng, h)];
            for (r, val) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            let (x3, y3) = (c(rng, w), c(rng, h));
            m.cpu.mem.ww(lin(ds, video::QUAD_X3 as u32), x3);
            m.cpu.mem.ww(lin(ds, video::QUAD_Y3 as u32), y3);
            m.cpu.s[ES] = rng.next();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            // Stale edge starts are read when an edge is horizontal.
            for a in [video::START_A, video::START_B] {
                m.cpu.mem.wd(lin(ds, a as u32), rng.dword());
            }
            true
        }
        "line" => {
            // Rows are not clipped: keep them on the screen (or a little off it, which writes
            // past the frame within the segment, as the original does).
            let v = [rng.coord(video::WIDTH), rng.below(200), rng.coord(video::WIDTH), rng.below(200)];
            for (r, val) in [EAX, EBX, ECX, EDX].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            if rng.below(8) == 0 {
                // Short and degenerate lines.
                m.cpu.r[ECX] = (m.cpu.r[ECX] & 0xFFFF_0000) | (v[0].wrapping_add(rng.below(5)).wrapping_sub(2)) as u32;
                m.cpu.r[EDX] = (m.cpu.r[EDX] & 0xFFFF_0000) | (v[1].wrapping_add(rng.below(5)).wrapping_sub(2)) as u32;
            }
            m.cpu.r[ESI] = rng.dword();
            m.cpu.r[EBP] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "disc" | "disc_clipped" => {
            let size = match rng.below(6) {
                0 => rng.below(8),
                1 => rng.next(),
                2 => 0x3e0 + rng.below(32),
                _ => rng.below(200),
            };
            m.cpu.r[EBX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::WIDTH) as u32;
            m.cpu.r[ECX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::VIEW_HEIGHT) as u32;
            if name == "disc" && rng.below(2) == 0 {
                // The unchecked entry: keep the disc touching the view.
                m.cpu.r[ECX] = (m.cpu.r[ECX] & 0xFFFF_0000) | rng.below(video::VIEW_HEIGHT) as u32;
            }
            m.cpu.r[EDX] = (rng.dword() & 0xFFFF_0000) | size as u32;
            m.cpu.r[EAX] = rng.dword();
            m.cpu.r[ESI] = rng.dword();
            m.cpu.r[EBP] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "blob" => {
            // Start at a shape: the byte after a terminator (or the table start).
            let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
            let base = lin(seg, video::BLOB_TABLE as u32);
            let mut starts = vec![0u16];
            for i in 0..0x1c0u16 {
                if (m.cpu.mem.rb(base + i as u32) as i8) < 0 {
                    starts.push(i + 1);
                }
            }
            starts.pop();
            let bp = starts[rng.below(starts.len() as u16) as usize];
            m.cpu.r[EBP] = (rng.dword() & 0xFFFF_0000) | bp as u32;
            m.cpu.r[EBX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::WIDTH) as u32;
            m.cpu.r[EDI] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::VIEW_HEIGHT) as u32;
            m.cpu.r[EAX] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "fill_rows" => {
            // The caller's frame starts after the return address the call pushes (SP = 7FFCh).
            let bp = 0x7FFCu16;
            let at = |o: u16| lin(STACK, bp.wrapping_add(o) as u32);
            let inside = rng.below(2) == 0;
            let rows = match rng.below(8) {
                0 => rng.below(4),
                _ => 1 + rng.below(video::VIEW_HEIGHT),
            };
            m.cpu.mem.ww(at(video::ROWS_COUNT), rows);
            let top = if rng.below(4) == 0 { rng.next() % 200 } else { rng.below(rows.max(1)) };
            m.cpu.mem.ww(at(video::ROWS_TOP), top);
            m.cpu.mem.ww(at(video::ROWS_INSIDE), if inside { 1 + rng.below(5) } else { 0 });
            for y in 0..video::VIEW_HEIGHT {
                let end = [0, 0x2a0, 0x540, 0x7e0][if rng.below(5) == 0 { 0 } else { rng.below(4) } as usize];
                m.cpu.mem.ww(at(4 + 2 * y), end);
                for k in 1..8 {
                    let v = if inside { rng.below(video::WIDTH) } else { rng.coord(video::WIDTH) };
                    m.cpu.mem.ww(at(4 + 2 * y + k * video::ROWS_STRIDE), v);
                }
            }
            for r in [EAX, EBX, ECX, EDX, ESI] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "fill_masks" | "fill_masks_args" => {
            // SP = 7FFCh after the call; with arguments, 8 words above the return address.
            let sp = 0x7FFCu16;
            let bp = if name == "fill_masks_args" { sp + 16 } else { sp };
            for i in 0..8 {
                m.cpu.mem.ww(lin(STACK, (sp + 4 + 2 * i) as u32), rng.next());
            }
            for w in [0x7bcu16, 0x7c0, 0x7c4, 0x7c8, 0x7cc, 0x7d0, 0x7d4, 0x7d8] {
                m.cpu.mem.ww(lin(ds, w as u32), rng.next());
            }
            let at = |o: u16| lin(STACK, bp.wrapping_add(o) as u32);
            let rows = match rng.below(8) {
                0 => rng.below(3),
                1 => 160 + rng.below(16),
                _ => 1 + rng.below(video::VIEW_HEIGHT),
            };
            m.cpu.mem.ww(at(6), rows);
            for i in 0..0x40u16 {
                m.cpu.mem.wb(at(8 + i), rng.next() as u8);
            }
            for y in 0..176u16 {
                let list = 0x4a + 0x80 * y;
                let k = if rng.below(4) == 0 { 0 } else { rng.below(10) };
                m.cpu.mem.ww(at(list), 4 * k);
                for j in 0..40u16 {
                    let v = match rng.below(4) {
                        0 => 0,
                        1 => 2 * rng.below(24),
                        _ => rng.coord(video::WIDTH),
                    };
                    m.cpu.mem.ww(at(list + 2 + 2 * j), v);
                }
                m.cpu.mem.ww(at(0x544a + 2 * y), 2 * rng.below(24));
            }
            m.cpu.mem.ww(at(0x544a + 0x14e), 2 * rng.below(24));
            for r in [EAX, EBX, ECX, EDX, ESI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
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
            m.cpu.r[ESP] = if name.starts_with("fill_") { 0x8000 } else { 0xFF00 };
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
            let mut orig_writes = std::collections::BTreeMap::new();
            let diff = match record_original(&mut m.cpu, &mut m.dev, 50_000_000) {
                Some(orig) => {
                    let d = compare(&out, &shadow, &orig, &m.cpu.mem, &m.dev.vga);
                    orig_writes = orig.writes;
                    d
                }
                None => Some(format!("the original did not return; the port claims {} steps", out.steps)),
            };
            if let Some(d) = diff {
                if mismatches == 0 && std::env::var_os("MCGA3_DEBUG").is_some() {
                    let mut ws: Vec<(u32, u8)> = shadow.writes().filter(|w| w.0 >= 0xA0000 && w.0 < 0xB0000 || w.0 >= 0xE0000).collect();
                    ws.sort();
                    eprintln!("port frame writes: {} first {:x?}", ws.len(), &ws[..ws.len().min(8)]);
                    let o: Vec<_> = orig_writes.iter().filter(|w| *w.0 >= 0xA0000 && *w.0 < 0xB0000 || *w.0 >= 0xE0000).take(8).collect();
                    eprintln!("orig frame writes: {} first {:x?}", orig_writes.iter().filter(|w| *w.0 >= 0xA0000 && *w.0 < 0xB0000 || *w.0 >= 0xE0000).count(), o);
                }
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
