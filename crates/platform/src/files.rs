//! The player's game files: which names the DOS release ships, and reading a folder natively.

/// A file as the apps receive it: its name (DOS-style `\` for subfolders) and its bytes.
pub type File = (String, Vec<u8>);

/// Names the DOS release ships (docs/FINDINGS.md §1), for fetching from the dev server.
pub const SHIPPED: &[&str] = &[
    "FRONTIER.EXE",
    "EL2SETUP.OVL",
    "EL2MATH3.OVL",
    "EL2MCGA3.OVL",
    "EL2ADLIB.OVL",
    "EL2ROL.OVL",
    "EL2SPK.OVL",
    "EL2M00.OVL",
    "EL2M01.OVL",
    "EL2M02.OVL",
    "EL2M03.OVL",
    "EL2M04.OVL",
    "EL2M05.OVL",
    "EL2M06.OVL",
    "EL2M07.OVL",
    "EL2M08.OVL",
    "EL2M09.OVL",
    "EL2M10.OVL",
    "EL2M11.OVL",
    "EL2M12.OVL",
    "EL2M13.OVL",
    "EL2M14.OVL",
    "EL2M15.OVL",
    "EL2M16.OVL",
    "EL2M17.OVL",
    "EL2M18.OVL",
    "EL2M19.OVL",
    "EL2G.GRF",
    "PIC000.GRF",
    "INTRO.ROL",
    "JUPALT.ROL",
    "VALKRIES.ROL",
    "Babayaga.rol",
    "Barem.rol",
    "Bluedan.rol",
    "Elitet.rol",
    "Ggofkiev.rol",
    "Hallking.rol",
    "LAVE",
    "MARS",
    "MOON",
    "VTOL",
    "BATTLE",
    "LATIS._01",
    "LATIS._02",
    "LATIS._03",
    "LATIS._04",
];

/// Whether the set holds the main program, which every app needs.
pub fn has_program(files: &[File]) -> bool {
    files.iter().any(|(n, _)| n.eq_ignore_ascii_case("FRONTIER.EXE"))
}

/// Every file under `root`, recursively, with subfolders joined by `\`.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_dir(root: &std::path::Path) -> Vec<File> {
    fn walk(root: &std::path::Path, rel: &str, out: &mut Vec<File>) {
        let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(root, &r, out);
            } else if let Ok(d) = std::fs::read(e.path()) {
                out.push((r.replace('/', "\\"), d));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out
}
