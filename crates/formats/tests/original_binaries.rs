//! Parses the MZ headers of the user's own copy of the game. Skips when `game-data/` is absent.

use std::path::Path;

const ORIGINAL: &str = "../../game-data/original";

#[test]
fn frontier_and_overlays_are_mz_images() {
    let dir = Path::new(ORIGINAL);
    if !dir.join("FRONTIER.EXE").exists() {
        eprintln!("skipping: {ORIGINAL}/FRONTIER.EXE missing (run tools/extract-assets.sh)");
        return;
    }
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n == "FRONTIER.EXE" || n.to_ascii_uppercase().ends_with(".OVL"))
        .collect();
    names.sort();
    assert!(names.len() > 20, "expected FRONTIER.EXE plus overlays, found {names:?}");
    for name in &names {
        let data = std::fs::read(dir.join(name)).unwrap();
        let mz = frontier_formats::mz::parse(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(mz.header_size() + mz.image_size() <= data.len(), "{name}: image exceeds file");
    }
}
