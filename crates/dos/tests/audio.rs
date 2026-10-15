//! The original AdLib driver on the runtime's OPL2 (docs/AUDIO.md). Skips when game-data/ is
//! absent.

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
    m.dev.adlib.capture = true;
    m.dev.adlib.write_log = Some(Vec::new());
    m.boot("FRONTIER.EXE").unwrap();
    Some(m)
}

/// Run `secs` of virtual time; return the register writes and a hash of the samples.
fn record(m: &mut Machine, secs: f64) -> (Vec<(u64, u8, u8)>, u64, i32) {
    while m.now() < secs {
        m.run(250_000);
    }
    m.dev.adlib.advance(m.now());
    let samples = std::mem::take(&mut m.dev.adlib.samples);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &samples {
        h = (h ^ *s as u16 as u64).wrapping_mul(0x100_0000_01b3);
    }
    let peak = samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    (m.dev.adlib.write_log.take().unwrap(), h, peak)
}

#[test]
fn adlib_driver_plays_the_intro_music() {
    let Some(mut m) = machine() else { return };
    let (writes, _, peak) = record(&mut m, 8.0);
    assert!(m.log.iter().any(|l| l.starts_with("exec EL2ADLIB.OVL")), "driver not loaded: {:#?}", m.log);
    // `EL2ADLIB:0920` (reset) runs first: waveform select on, CSM/NTS off, rhythm off.
    assert_eq!(&writes[..3].iter().map(|w| (w.1, w.2)).collect::<Vec<_>>(), &[(0x01, 0x20), (0x08, 0x00), (0xBD, 0x00)]);
    let key_ons = writes.iter().filter(|w| (0xB0..=0xB8).contains(&w.1) && w.2 & 0x20 != 0).count();
    assert!(key_ons > 50, "only {key_ons} key-ons in 8 s");
    assert!(peak > 0, "silent");
}

#[test]
fn audio_is_deterministic() {
    let Some(mut a) = machine() else { return };
    let Some(mut b) = machine() else { return };
    let ra = record(&mut a, 6.0);
    let rb = record(&mut b, 6.0);
    assert_eq!(ra.0, rb.0);
    assert_eq!(ra.1, rb.1);
}
