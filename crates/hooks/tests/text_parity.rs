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

/// The dispatcher `1000:e89d` across its id ranges: the main table, the `3016h` formatter, the
/// `[DS:5703]` table and module strings (every module loaded first), against the original.
#[test]
fn dispatcher_matches_original() {
    let Some(mut m) = machine() else { return };
    let main = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg;
    let ds = main + 0x1615;
    for slot in [0u16, 3, 5, 7, 10, 11, 14, 15, 17, 18] {
        load(&mut m, ds, slot);
    }
    let port = frontier_game::text::MAIN_TEXT[0].3;
    let mut ids: Vec<u16> = (0..0x300).step_by(3).collect();
    ids.extend([0x3016; 8]);
    ids.extend((0x4000..0x4200).step_by(5));
    for slot in [0u16, 3, 5, 7, 10, 11, 14, 15, 17, 18] {
        ids.extend((0..0x40).map(|k| 0x8000 | slot << 9 | k));
    }
    let (mut compared, mut declined, mut failures) = (0, 0, Vec::new());
    for id in ids {
        m.cpu.r[0] = id as u32;
        m.cpu.r[7] = 0x8000;
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.s[ES] = 0xB000;
        m.cpu.mem.write_bytes(lin(0xA000, 0x0100), b"Merlin\0");
        m.cpu.mem.ww(lin(ds, 0x07e4), 0x0100);
        m.cpu.mem.ww(lin(ds, 0x07e6), 0xA000);
        m.enter(main, 0xe89d);
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        let diff = match frontier_hooks::record_original(&mut m.cpu, 1_000_000) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 4 {
                failures.push(format!("id {id:04x}: {d}"));
            }
        }
    }
    eprintln!("dispatch: {compared} compared, {declined} declined, {} mismatches", failures.len());
    assert!(compared > 300, "too few compared");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
