//! Reads every data file of the user's own copy: EL2G.GRF, PIC000.GRF, the .ROL songs, the model
//! tables and the overlay headers. Skips when `game-data/` is absent.

use std::path::{Path, PathBuf};

use frontier_formats::{grf, identify, model, overlay, smf};

const ORIGINAL: &str = "../../game-data/original";

fn dir() -> Option<PathBuf> {
    let d = Path::new(ORIGINAL);
    if d.join("FRONTIER.EXE").exists() {
        Some(d.to_path_buf())
    } else {
        eprintln!("skipping: {ORIGINAL}/FRONTIER.EXE missing (run tools/extract-assets.sh)");
        None
    }
}

fn names(d: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    n.sort();
    n
}

#[test]
fn el2g_archive_uses_every_byte_and_every_image_unpacks() {
    let Some(d) = dir() else { return };
    let data = std::fs::read(d.join("EL2G.GRF")).unwrap();
    let entries = grf::parse_archive(&data).unwrap();
    assert_eq!(entries.len(), 225);
    // The terminator is the last word of the file: every byte is consumed.
    let last = entries.last().unwrap();
    assert!(last.offset < data.len() - 2);
    assert_eq!(&data[data.len() - 2..], &[0, 0]);
    for (i, e) in entries.iter().enumerate() {
        let img = e.image().expect("all entries have headers").unwrap_or_else(|err| panic!("entry {i}: {err}"));
        assert_eq!(img.pixels.len(), img.width * img.height, "entry {i}");
        if e.header.unwrap().flags & grf::FLAG_PACKED != 0 {
            // A packed entry is consumed exactly by its unpacking.
            let (_, used) = grf::unpack(&e.data, img.width * img.height).unwrap();
            assert_eq!(used, e.data.len(), "entry {i}");
        }
    }
}

#[test]
fn screenshot_decodes_to_one_frame() {
    let Some(d) = dir() else { return };
    let data = std::fs::read(d.join("PIC000.GRF")).unwrap();
    let s = grf::parse_screenshot(&data).unwrap();
    assert_eq!(s.pixels.len(), 64000);
    assert!(s.palette.iter().flatten().all(|&v| v < 64), "6-bit DAC values");
}

#[test]
fn every_rol_file_is_midi() {
    let Some(d) = dir() else { return };
    let mut n = 0;
    for name in names(&d).iter().filter(|n| n.to_ascii_uppercase().ends_with(".ROL")) {
        let data = std::fs::read(d.join(name)).unwrap();
        assert_eq!(identify::identify(name, &data), identify::Kind::Midi, "{name}");
        let s = smf::parse(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!s.tracks.is_empty() && !s.timeline().is_empty(), "{name}");
        n += 1;
    }
    assert_eq!(n, 9);
}

#[test]
fn model_tables_resolve() {
    let Some(d) = dir() else { return };
    let exe = std::fs::read(d.join("FRONTIER.EXE")).unwrap();
    let (image, table) = model::main_table(&exe).unwrap();
    assert_eq!(table.models.len(), 240);
    let mut complete = 0;
    let mut stops = std::collections::BTreeMap::new();
    for off in table.models.iter().flatten() {
        let h = model::header(&image, *off).unwrap();
        model::vertex_defs(&image, &h).unwrap();
        match model::walk(&image, off + h.mesh as usize).stopped {
            None => complete += 1,
            Some(why) => *stops.entry(why.split(" at ").next().unwrap().to_string()).or_insert(0) += 1,
        }
    }
    let present = table.models.iter().flatten().count();
    eprintln!("{complete} of {present} models walk to their end; stops: {stops:?}");
    assert_eq!(complete, present, "every model walks to its end: stops {stops:?}");

    let texts = model::texts(&exe);
    assert_eq!(texts.get(4).map(String::as_str), Some("Mine"));
    assert_eq!(texts.get(0x2b).map(String::as_str), Some("Long-range Cruiser"));
    let roles = model::roles(&image, &table, &texts);
    // Measured in the game (docs/FORMATS.md §4): the F3 status screen and a shipyard list.
    let named = |n: usize| match &roles[n] {
        Some(model::Role::Named(s)) => s.as_str(),
        _ => "",
    };
    assert_eq!(named(16), "Eagle long range fighter");
    assert_eq!(named(27), "Constrictor");
    assert_eq!(named(31), "Tiger Trader");
    assert_eq!(named(36), "Puma Clipper");
    // Every ship name in the list names one model of 9–41, except ". IXI ." (text 17), which no
    // model's data block names.
    for t in &texts[10..=0x2b] {
        if !t.contains("__") && t != ". IXI ." {
            assert!((9..=41).any(|n| named(n) == t), "no model of 9–41 is named {t}");
        }
    }
    assert_eq!(roles[42], Some(model::Role::Structure));
    let mut groups = std::collections::BTreeMap::new();
    for r in roles.iter().flatten() {
        *groups.entry(r.group()).or_insert(0) += 1;
    }
    eprintln!("groups: {groups:?}");

    let m08 = std::fs::read(d.join("EL2M08.OVL")).unwrap();
    let img = overlay::image(&m08).unwrap();
    let t = model::table(img, model::EL2M08_BASE).unwrap();
    assert_eq!(t.models.len(), 240);
    for off in t.models.iter().flatten() {
        let h = model::header(img, *off).unwrap();
        assert_eq!(model::walk(img, off + h.mesh as usize).stopped, None, "EL2M08 model at {off:05x}");
    }
}

#[test]
fn overlay_headers_parse() {
    let Some(d) = dir() else { return };
    for name in names(&d).iter().filter(|n| n.to_ascii_uppercase().ends_with(".OVL")) {
        let data = std::fs::read(d.join(name)).unwrap();
        let kind = identify::identify(name, &data);
        let o = overlay::parse(&data, kind == identify::Kind::Module).unwrap_or_else(|e| panic!("{name}: {e}"));
        if let overlay::Overlay::Driver { exports, first_slot } = o {
            assert!(!exports.is_empty(), "{name}: no exports from slot {first_slot:04x}");
        }
    }
}

/// The parts the original drew for the shipyard's Tiger Trader (model 31) with its object
/// variables (*measured* with model_census: lookups of models 2, 111 ×3, 120–123, 127), against
/// the walker evaluating the model's logic with the same variables.
#[test]
fn tiger_trader_parts_match_the_game() {
    let Some(d) = dir() else { return };
    let exe = std::fs::read(d.join("FRONTIER.EXE")).unwrap();
    let (image, table) = model::main_table(&exe).unwrap();
    let mut paint = model::Paint::new(std::sync::Arc::new(model::sine_table(&image)));
    paint.object.copy_from_slice(&model::SHOWN_SHIP);
    fn parts(image: &[u8], table: &model::Table, n: usize, paint: &mut model::Paint, out: &mut Vec<u16>, depth: u32) {
        let off = table.models[n].unwrap();
        let h = model::header(image, off).unwrap();
        let m = model::walk_with(image, off + h.mesh as usize, model::Branches::Near, paint);
        let mut subs = m.sub_paints.into_iter();
        for (prim, _) in m.prims {
            if let model::Prim::Sub { model: s, .. } = prim {
                out.push(s);
                let mut child = subs.next().unwrap();
                if depth < 4 {
                    parts(image, table, s as usize, &mut child, out, depth + 1);
                }
            }
        }
    }
    let mut out = Vec::new();
    parts(&image, &table, 31, &mut paint, &mut out, 0);
    out.sort();
    assert_eq!(out, vec![2, 111, 111, 111, 120, 121, 122, 123, 127]);
    // Its livery: texture entry 9 (measured: the faces added 00090000h).
    assert_eq!(model::texture_entry(paint.colour), Some(9));
}
