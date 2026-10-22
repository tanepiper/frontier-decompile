//! Code coverage (docs/COVERAGE.md): recording must not change the run, and it must file code
//! under the image it ran in, including overlays evicted and reloaded at another segment.
//! Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::{Machine, Vfs};

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
    Some(m)
}

#[test]
fn recording_does_not_change_the_run() {
    let (Some(mut plain), Some(mut rec)) = (machine(), machine()) else { return };
    rec.coverage_start();
    // 0.1 s virtual: past the module loads, including EL2M08's eviction and reload.
    plain.run(2_500_000);
    rec.run(2_500_000);
    assert_eq!((plain.cpu.r, plain.cpu.s, plain.cpu.ip), (rec.cpu.r, rec.cpu.s, rec.cpu.ip));
    assert_eq!(plain.cpu.mem.slice(0, 0xA0000), rec.cpu.mem.slice(0, 0xA0000));
    assert_eq!(plain.log, rec.log);

    let cov = rec.coverage_take();
    assert!(rec.cpu.cover.is_none());
    let main = &cov["FRONTIER.EXE"];
    // The entry point, 0000:008f in the file (Ghidra 1000:008f), ran first.
    assert!(main.exec.contains(&0x8f));
    // `load_overlay` (1000:d34c) is called for every driver overlay (docs/OVERLAYS.md §1).
    assert!(main.entries.contains(&0xd34c));
    // Every module ran its TSR stub at offset 0, wherever DOS put it.
    for i in 0..18 {
        let m = &cov[&format!("EL2M{i:02}.OVL")];
        assert!(m.exec.contains(&0), "EL2M{i:02} stub");
    }
    // Nothing of the program is filed under bare linear addresses except the BIOS stubs.
    assert!(cov.get("linear").is_none_or(|c| c.exec.iter().all(|&l| l >= 0xF0000)));
}
