//! Parity: galaxy generation ports (`frontier_game::galaxy`) against the original routines in
//! `EL2M02`, run on the reference runtime inside the booted game with `EL2M02` and `EL2M19`
//! resident. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{CS, DS, ECX, ES, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::galaxy::PORTS_M02;
use frontier_game::names::PORTS_M09;
use frontier_game::system::PORTS_M00;
use frontier_game::math3::DECLINE;
use frontier_hooks::{compare, record_original_until, regs_from, run_port};

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
    m.run(5_000_000);
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    // Load modules 2 (EL2M02) and 19 (EL2M19) the way the game does: call handler +04 through
    // the module table, which reaches the lazy-load thunk while the module is not resident
    // (docs/OVERLAYS.md §4). AL = 0 makes EL2M02's handler +04 do nothing.
    for slot in [19u16, 2, 9, 0] {
        let si = slot * 8;
        let hdr_off = m.cpu.mem.rw(lin(ds, 0x0396 + si as u32));
        let hdr_seg = m.cpu.mem.rw(lin(ds, 0x0398 + si as u32));
        let off = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 4));
        let seg = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 6));
        m.cpu.s[DS] = ds;
        m.cpu.r[ESI] = si as u32;
        m.cpu.r[0] = 0;
        m.call(seg, off, 50_000_000).expect("module load");
    }
    Some(m)
}

struct X(u64);
impl X {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as u32
    }
}

#[test]
fn galaxy_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let Some(m02) = m.image_segment("EL2M02.OVL") else {
        eprintln!("skipping: EL2M02 not resident after start-up");
        return;
    };
    assert!(m.image_segment("EL2M19.OVL").is_some(), "EL2M19 not resident");
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let mut x = X(0x5DEE_CE66_D1CE_4E5B);
    let mut failures = Vec::new();
    let m09 = m.image_segment("EL2M09.OVL").expect("EL2M09 resident");
    let m00 = m.image_segment("EL2M00.OVL").expect("EL2M00 resident");
    let tables: Vec<(u16, &[(u16, u16, &str, frontier_game::math3::Port)])> = vec![(m02, PORTS_M02), (m09, PORTS_M09), (m00, PORTS_M00)];
    for &(seg, ports) in &tables {
    for &(off, _slot, name, port) in ports {
        // GALAXY_ONLY=name runs one routine (debugging).
        if std::env::var("GALAXY_ONLY").is_ok_and(|o| o != name) {
            continue;
        }
        // EL2M00:50d6 is checked in a game-generated system instead
        // (binary_matches_original_in_a_generated_system): in random states the original itself
        // crashes, e.g. a saturated mass walks the class table into data and calls a garbage
        // classifier address.
        if name == "binary" {
            continue;
        }
        let (mut mismatches, mut declined, mut compared, mut nonzero) = (0, 0, 0, 0);
        let cases = match name {
            "sector_stars" => 5_000,
            "setup_body" | "finish_body" => 2_000,
            _ => 20_000,
        };
        for i in 0..cases {
            for r in 0..8 {
                m.cpu.r[r] = x.next();
            }
            // Mostly in-galaxy sectors (13-bit), some outside; CX mostly small selectors.
            if i % 8 != 0 {
                m.cpu.r[0] = (m.cpu.r[0] & 0xFFFF_0000) | (x.next() & 0x1FFF);
                m.cpu.r[3] = (m.cpu.r[3] & 0xFFFF_0000) | (x.next() & 0x1FFF);
            }
            if i % 4 != 0 {
                m.cpu.r[ECX] = (m.cpu.r[ECX] & 0xFFFF_0000) | (x.next() % 0x14);
            }
            if name == "sector_stars" {
                // A frame for the star list, sectors (some from the catalogue at EL2M02:00a2),
                // and the renderer flag DS:215b mostly clear.
                m.cpu.r[5] = 0x8000;
                let (sx, sy) = if i % 3 == 0 {
                    let e = 0x00a2 + 4 * (x.next() % 47);
                    (m.cpu.mem.rw(lin(m02, e)), m.cpu.mem.rw(lin(m02, e + 2)))
                } else {
                    ((x.next() & 0x1FFF) as u16, (x.next() & 0x1FFF) as u16)
                };
                m.cpu.mem.ww(lin(ds, 0x2149), sx);
                m.cpu.mem.ww(lin(ds, 0x214d), sy);
                m.cpu.mem.ww(lin(ds, 0x215b), if i % 2 == 0 { 0xE1 } else { 0x61 });
            }
            m.cpu.mem.wd(lin(ds, 0x2153), x.next());
            m.cpu.mem.wd(lin(ds, 0x2157), x.next());
            m.cpu.r[ESP] = 0xFFF0;
            m.cpu.s[DS] = ds;
            m.cpu.s[ES] = 0xB000;
            m.cpu.s[SS] = 0xA000;
            m.cpu.push16(0x1234); // near return offset, never executed
            if name == "name" {
                // Kind in AX (mostly the generator's 0–17), seed in CX:BX, output at SS:9000h.
                m.cpu.r[0] = (m.cpu.r[0] & 0xFFFF_0000) | if i % 8 == 0 { x.next() & 0x1F } else { x.next() % 18 };
                m.cpu.r[7] = (m.cpu.r[7] & 0xFFFF_0000) | 0x9000;
                m.cpu.pop16();
                m.cpu.push16(m.cpu.s[CS]);
                m.cpu.push16(0x1234);
            }
            if seg == m00 {
                // A body record at ES:SI = B000:0100, its temperature, seed and flags; the
                // parent value DS:2049; the system seed.
                m.cpu.s[ES] = 0xB000;
                m.cpu.r[6] = (m.cpu.r[6] & 0xFFFF_0000) | 0x0100;
                let rec = lin(0xB000, 0x0100);
                for k in 0..0x20 {
                    m.cpu.mem.ww(rec + 2 * k, x.next() as u16);
                }
                m.cpu.mem.ww(rec + 0x20, (x.next() % 0x240) as u16);
                m.cpu.mem.ww(lin(ds, 0x2049), (x.next() % 0x140) as u16);
                m.cpu.mem.wd(lin(ds, 0x2021), x.next());
                m.cpu.mem.wd(lin(ds, 0x2025), x.next());
                m.cpu.r[0] = (m.cpu.r[0] & 0xFFFF_0000) | (x.next() % 16);
                m.cpu.flags &= !0x0400;
                // Body counters: count of bodies (some at the limit of 18), the running index
                // and the parent index, so [204b] - [202d] points back into the records.
                let made = (x.next() % 0x13) as u8;
                m.cpu.mem.wb(lin(ds, 0x204d), made);
                let idx = (x.next() % 8) as u16;
                m.cpu.mem.ww(lin(ds, 0x202d), idx);
                m.cpu.mem.ww(lin(ds, 0x204b), idx + (x.next() % 3) as u16);
                if name == "setup_body" || name == "finish_body" {
                    // Records at B000:0100 upwards; the parent chain base [2041]:[203f] there
                    // too, ending at a body whose +08h is 0; the parent [7e0] one record back;
                    // a system name; masses across the class thresholds; a classifier.
                    m.cpu.mem.ww(lin(ds, 0x2041), 0xB000);
                    m.cpu.mem.ww(lin(ds, 0x203f), 0x0100);
                    for r in 0..6u32 {
                        let a = lin(0xB000, 0x0100 + 0x40 * r);
                        m.cpu.mem.ww(a + 8, if r == 0 { 0 } else { (x.next() % (r + 1)) as u16 });
                        m.cpu.mem.wd(a + 0x18, if x.next() % 3 == 0 { 0 } else { x.next() });
                        m.cpu.mem.wb(a + 0x3b, x.next() as u8);
                        m.cpu.mem.write_bytes(a + 0x22, b"Ross 154\0");
                    }
                    let r = 1 + x.next() % 4;
                    let rec = 0x0100 + 0x40 * r as u16;
                    m.cpu.r[6] = (m.cpu.r[6] & 0xFFFF_0000) | rec as u32;
                    m.cpu.mem.ww(lin(ds, 0x07e0), rec - 0x40);
                    // Parent index k names record k-1: strictly before this one.
                    m.cpu.mem.ww(lin(ds, 0x2037), (x.next() % (r + 1)) as u16);
                    m.cpu.mem.write_bytes(lin(0xB000, 0x0f00), b"Sol\0");
                    m.cpu.mem.ww(lin(ds, 0x2029), 0x0f00);
                    m.cpu.mem.ww(lin(ds, 0x202b), 0xB000);
                    let mass = match x.next() % 4 {
                        0 => x.next() % 0x100,
                        1 => x.next() % 0x10000,
                        2 => 0x0100_0000 + x.next() % 0x0010_0000,
                        _ => x.next(),
                    };
                    m.cpu.mem.wd(lin(ds, 0x07bc), mass);
                    m.cpu.mem.wd(lin(ds, 0x07c0), x.next() % 0x400);
                    m.cpu.mem.wd(lin(ds, 0x07c8), x.next());
                    m.cpu.mem.ww(lin(ds, 0x07e8), [0u16, 0x4990, 0x4abb, 0x4b69, 0x4bda][(x.next() % 5) as usize]);
                    m.cpu.mem.wb(lin(ds, 0x2034), (x.next() % 3) as u8);
                    for b in 0x2030..0x2034u32 {
                        m.cpu.mem.wb(lin(ds, b), (x.next() % 12) as u8);
                    }
                    m.cpu.r[7] = (m.cpu.r[7] & 0xFFFF_0000) | (0x0100 + 0x40 * (x.next() % 5)) as u32;
                }
                if name == "big_body" {
                    m.cpu.r[0] = (m.cpu.r[0] & 0xFFFF_0000) | (x.next() % 4);
                }
            }
            m.cpu.s[CS] = seg;
            m.cpu.ip = off as u32;
            let input = regs_from(&m.cpu);
            let out = run_port(&m.cpu, port);
            if out.steps == DECLINE {
                declined += 1;
                continue;
            }
            compared += 1;
            if out.cpu.r[0] as u16 != 0 {
                nonzero += 1;
            }
            let diff = match record_original_until(&mut m.cpu, std::env::var("GALAXY_BUDGET").ok().and_then(|b| b.parse().ok()).unwrap_or(200_000), out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
                // EL2M00:468a's caller tests ZF (EL2M00:4fb1): compare ZF and SF there too.
                Some(orig) if matches!(name, "setup_body" | "finish_body") && (orig.cpu.flags ^ out.cpu.flags) & 0xC0 != 0 => {
                    Some(format!("flags: original {:04x}, port {:04x}", orig.cpu.flags, out.cpu.flags))
                }
                Some(orig) => compare(&out, &orig, &m.cpu.mem),
                None => Some("the original did not return".into()),
            };
            if let Some(d) = diff {
                if mismatches == 0 {
                    failures.push(format!("{name} ({off:04x}): input {input:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        eprintln!("{name}: {compared} compared ({nonzero} with non-zero AX), {declined} declined");
        assert!(compared > cases / 2 && nonzero > cases / 10, "{name}: too few meaningful cases");
        let _ = nonzero;
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{cases} mismatches"));
        }
    }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `EL2M00:50d6` (binary stars) needs a whole generated system around it, so it is checked in the
/// system the game generates at the start (Ross 154): the game is driven into the cockpit, then
/// the routine is called with masses across its bands (single star plus a quarter-mass companion,
/// and the barycentre pair) against the original.
#[test]
fn binary_matches_original_in_a_generated_system() {
    // A plain boot: the game is only played, never interrupted by test calls, until the cockpit.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/original");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: {} missing (run tools/extract-assets.sh)", dir.display());
        return;
    };
    let mut vfs = Vfs::new();
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let second = m.dev.ips as u64;
    assert_eq!(m.run(20 * second), frontier_dos::Stop::Budget);
    m.key(0x39);
    m.key(0xB9);
    assert_eq!(m.run(4 * second), frontier_dos::Stop::Budget);
    m.key(0x02);
    m.key(0x82);
    assert_eq!(m.run(9 * second), frontier_dos::Stop::Budget);
    let ds0 = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    eprintln!("at {:.1} virtual s; bodies counted: {}", m.now(), m.cpu.mem.rw(lin(ds0, 0x202d)));
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    // EL2M00 may have been evicted after generating the system: load it through its own thunk
    // (handler +04 of slot 0), with the game's registers saved around the call.
    if m.image_segment("EL2M00.OVL").is_none() {
        let (r, sg, ip, fl) = (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags);
        let hdr_off = m.cpu.mem.rw(lin(ds, 0x0396));
        let hdr_seg = m.cpu.mem.rw(lin(ds, 0x0398));
        let off = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 4));
        let seg = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 6));
        m.cpu.s[DS] = ds;
        m.cpu.r[ESI] = 0;
        m.call(seg, off, 50_000_000).expect("load EL2M00");
        (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags) = (r, sg, ip, fl);
    }
    let m00 = m.image_segment("EL2M00.OVL").expect("EL2M00 resident");
    let port = frontier_game::system::PORTS_M00.iter().find(|p| p.2 == "binary").unwrap().3;
    let mut x = X(0x0BAD_5EED_1234_5678);
    let (mut compared, mut failures) = (0, Vec::new());
    for i in 0..400 {
        let mass: u32 = match i % 4 {
            0 => 0x0a21_fe80 + x.next() % (0x0a6e_49c0 - 0x0a21_fe80),
            1 => 0x0103_6640 + x.next() % 0x0900_0000,
            2 => 0x0a6e_49c0 + x.next() % 0x1000_0000,
            _ => 0xffff_ffef + x.next() % 0x10,
        };
        m.cpu.r[0] = mass & 0xffff;
        m.cpu.r[2] = mass >> 16;
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.flags &= !0x0400;
        // Keep the body pointer inside the record block between calls.
        let base = m.cpu.mem.rw(lin(ds, 0x203f));
        m.cpu.mem.ww(lin(ds, 0x07dc), base.wrapping_add(0x40 * (2 + x.next() % 6) as u16));
        m.cpu.mem.ww(lin(ds, 0x202d), 2 + (x.next() % 6) as u16);
        m.cpu.push16(0x1234);
        m.cpu.s[CS] = m00;
        m.cpu.ip = 0x50d6;
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            continue;
        }
        compared += 1;
        let diff = match record_original_until(&mut m.cpu, 2_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 3 {
                failures.push(format!("mass {mass:08x}: {d}"));
            }
        }
    }
    eprintln!("binary: {compared} compared, {} mismatches", failures.len());
    assert!(compared > 100, "too few cases compared");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
