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
        let (mut mismatches, mut declined, mut compared, mut nonzero) = (0, 0, 0, 0);
        let cases = if name == "sector_stars" { 5_000 } else { 20_000 };
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
            let diff = match record_original_until(&mut m.cpu, 1_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
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
