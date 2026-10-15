//! Boot the user's original FRONTIER.EXE on the runtime and check the start-up sequence observed
//! in docs/RUNTIME.md. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::{Machine, Stop, Vfs};

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
fn boots_through_overlay_loading() {
    let Some(mut m) = machine() else { return };
    let stop = m.run(5_000_000);
    assert_eq!(stop, Stop::Budget, "runtime stopped early; log: {:#?}", m.log);
    let execs: Vec<&str> = m.log.iter().filter(|l| l.contains(" at PSP")).filter_map(|l| l.strip_prefix("exec ")).map(|l| l.split(' ').next().unwrap()).collect();
    let mut expected = vec!["EL2SETUP.OVL", "EL2MATH3.OVL", "EL2ADLIB.OVL", "EL2MCGA3.OVL"];
    let modules: Vec<String> = (0..18).map(|i| format!("EL2M{i:02}.OVL")).collect();
    expected.extend(modules.iter().map(String::as_str));
    assert_eq!(&execs[..expected.len()], &expected[..], "overlay load order (docs/OVERLAYS.md §3–4)");
    assert!(m.log.iter().any(|l| l == "EMS: allocated 48 pages as handle 1"));
    assert!(m.log.iter().any(|l| l == "video mode 13h"));
}
