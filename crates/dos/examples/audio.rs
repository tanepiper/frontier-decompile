//! Record what the original AdLib driver (`EL2ADLIB.OVL`) plays on the reference runtime.
//!
//! Usage: cargo run --release -p frontier-dos --example audio -- [game-dir] [seconds] [out-dir] [keys...]
//!
//! Boots the user's FRONTIER.EXE (sound selection 2, docs/AUDIO.md), runs `seconds` of virtual
//! time (default 30) and writes `out-dir/boot.wav` (default `game-data/derived/audio`): mono,
//! 16-bit, at the OPL2's native 49,716 Hz. It also writes `boot-opl.txt`, every register write as
//! `sample register value` (hex), for comparison with a DOSBox capture. `keys` are `T:SC` (press
//! and release set-1 scancode SC, hex, at virtual second T).

use std::io::Write;
use std::path::Path;

use frontier_dos::{Machine, Vfs};
use frontier_opl::SAMPLE_RATE;

fn load_dir(vfs: &mut Vfs, root: &Path, rel: &str) {
    for e in std::fs::read_dir(root.join(rel)).expect("game dir") {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
        if e.file_type().unwrap().is_dir() {
            vfs.insert_dir(&r);
            load_dir(vfs, root, &r.replace('\\', "/"));
        } else {
            vfs.insert(&r, std::fs::read(e.path()).unwrap());
        }
    }
}

fn write_wav(path: &Path, rate: u32, samples: &[i16]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let data = (samples.len() * 2) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&1u16.to_le_bytes()).unwrap(); // mono
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * 2).to_le_bytes()).unwrap();
    f.write_all(&2u16.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data.to_le_bytes()).unwrap();
    for s in samples {
        f.write_all(&s.to_le_bytes()).unwrap();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let out = args.get(2).cloned().unwrap_or_else(|| "game-data/derived/audio".into());
    let mut keys: Vec<(f64, u8)> = args
        .iter()
        .skip(3)
        .filter_map(|k| {
            let (t, sc) = k.split_once(':')?;
            Some((t.parse().ok()?, u8::from_str_radix(sc, 16).ok()?))
        })
        .collect();
    keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut vfs = Vfs::new();
    load_dir(&mut vfs, Path::new(&dir), "");
    let mut m = Machine::new(vfs);
    m.dev.adlib.capture = true;
    m.dev.adlib.write_log = Some(Vec::new());
    m.boot("FRONTIER.EXE").unwrap();
    let slice = (m.dev.ips / 100.0) as u64;
    let mut printed = 0;
    while m.now() < secs {
        m.run(slice);
        for l in &m.log[printed..] {
            println!("[{:8.3}] {l}", m.now());
        }
        printed = m.log.len();
        while let Some(&(t, sc)) = keys.first() {
            if m.now() < t {
                break;
            }
            m.key(sc);
            m.key(sc | 0x80);
            keys.remove(0);
        }
    }
    m.dev.adlib.advance(m.now());
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let samples = std::mem::take(&mut m.dev.adlib.samples);
    write_wav(&out.join("boot.wav"), SAMPLE_RATE.round() as u32, &samples);
    let log = m.dev.adlib.write_log.take().unwrap();
    let mut f = std::io::BufWriter::new(std::fs::File::create(out.join("boot-opl.txt")).unwrap());
    for (t, r, v) in &log {
        writeln!(f, "{t} {r:02x} {v:02x}").unwrap();
    }
    let peak = samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    println!("{} samples, {} register writes, peak {peak}", samples.len(), log.len());
}
