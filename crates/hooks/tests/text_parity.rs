//! Parity: the modules' text handlers (`frontier_game::text`) against the originals, with every
//! module loaded through its own thunk. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{CS, DS, ES, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::math3::DECLINE;
use frontier_game::text::MODULE_TEXT;
use frontier_hooks::{compare, record_original_until, run_port};

fn machine() -> Option<Machine> {
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
    Some(m)
}

/// Load module `slot` through handler +04 of its module table entry (the lazy-load thunk).
fn load(m: &mut Machine, ds: u16, slot: u16) {
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

#[test]
fn module_text_matches_original() {
    let Some(mut m) = machine() else { return };
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let mut failures = Vec::new();
    for &(module, table) in MODULE_TEXT {
        let slot: u16 = module[4..6].parse().unwrap();
        let (off, _, name, port) = table[0];
        let mut seg = m.image_segment(module);
        if seg.is_none() {
            load(&mut m, ds, slot);
            seg = m.image_segment(module);
        }
        let seg = seg.expect("module resident");
        let (mut compared, mut declined, mut bad) = (0, 0, 0);
        for id in 0..0x200u32 {
            m.cpu.r[0] = id * 2;
            m.cpu.r[7] = 0x8000;
            m.cpu.r[ESP] = 0xFFF0;
            m.cpu.s[SS] = 0xA000;
            m.cpu.s[DS] = ds;
            m.cpu.s[ES] = 0xB000;
            // A sub-name for the nested-string escape.
            m.cpu.mem.write_bytes(lin(0xA000, 0x0100), b"Merlin\0");
            m.cpu.mem.ww(lin(ds, 0x07e4), 0x0100);
            m.cpu.mem.ww(lin(ds, 0x07e6), 0xA000);
            m.enter(seg, off);
            let out = run_port(&m.cpu, port);
            if out.steps == DECLINE {
                declined += 1;
                continue;
            }
            compared += 1;
            let diff = match record_original_until(&mut m.cpu, 1_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
                Some(orig) => compare(&out, &orig, &m.cpu.mem),
                None => Some("the original did not return".into()),
            };
            if let Some(d) = diff {
                if bad == 0 {
                    failures.push(format!("{name} ({module}:{off:04x}) id {id}: {d}"));
                }
                bad += 1;
            }
        }
        let _ = CS;
        eprintln!("{name}: {compared} compared, {declined} declined, {bad} mismatches");
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
