//! Boot the user's original FRONTIER.EXE on the reference runtime, headless.
//!
//! Usage: cargo run --release -p frontier-dos --example boot -- [game-dir] [seconds] [keys...]
//!
//! Loads every file in `game-dir` (default `game-data/original`), runs for `seconds` of virtual
//! time (default 10) and writes the VGA frame every second to
//! `game-data/derived/captures/boot-NN.png`. `keys` are `T:SC` pairs: at virtual second T press
//! and release set-1 scancode SC (hex), e.g. `3:39` = space at 3 s; or `T:@X,Y` to move the
//! mouse to X,Y (driver coordinates) and click the left button.

use std::path::Path;

use frontier_dos::{Machine, Stop, Vfs};

#[derive(Clone, Copy)]
enum Input {
    Key(u8),
    Click(i32, i32),
}

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let mut keys: Vec<(f64, Input)> = args
        .iter()
        .skip(2)
        .filter_map(|k| {
            let (t, what) = k.split_once(':')?;
            let input = match what.strip_prefix('@') {
                Some(xy) => {
                    let (x, y) = xy.split_once(',')?;
                    Input::Click(x.parse().ok()?, y.parse().ok()?)
                }
                None => Input::Key(u8::from_str_radix(what, 16).ok()?),
            };
            Some((t.parse().ok()?, input))
        })
        .collect();
    keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut vfs = Vfs::new();
    load_dir(&mut vfs, Path::new(&dir), "");
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let out = Path::new("game-data/derived/captures");
    std::fs::create_dir_all(out).unwrap();
    let slice = (m.dev.ips / 100.0) as u64;
    let mut next_shot = 1.0;
    let mut shot = 0;
    let mut printed = 0;
    let t0 = std::time::Instant::now();
    let stop = loop {
        let st = m.run(slice);
        for l in &m.log[printed..] {
            println!("[{:8.3}] {l}", m.now());
        }
        printed = m.log.len();
        while let Some(&(t, sc)) = keys.first() {
            if m.now() < t {
                break;
            }
            match sc {
                Input::Key(sc) => {
                    m.key(sc);
                    m.key(sc | 0x80);
                }
                Input::Click(x, y) => {
                    let (dx, dy) = (x - m.mouse.x, y - m.mouse.y);
                    m.mouse_move(dx, dy);
                    m.mouse_button(1, true);
                    m.mouse_button(1, false);
                }
            }
            keys.remove(0);
        }
        if m.now() >= next_shot {
            let mut rgba = vec![0u8; 320 * 200 * 4];
            m.frame_rgba(&mut rgba);
            let p = out.join(format!("boot-{shot:02}.png"));
            std::fs::write(&p, frontier_dos::png::encode_rgba(320, 200, &rgba)).unwrap();
            shot += 1;
            next_shot += 1.0;
        }
        if st != Stop::Budget {
            break st;
        }
        if m.now() >= secs {
            break st;
        }
    };
    println!("stop: {stop:?} after {} instructions ({:.2} s virtual, {:.2} s host)", m.cpu.icount, m.now(), t0.elapsed().as_secs_f64());
    println!("cs:ip {:04x}:{:04x} ds {:04x} es {:04x} ss:sp {:04x}:{:04x}", m.cpu.s[1], m.cpu.ip, m.cpu.s[3], m.cpu.s[0], m.cpu.s[2], m.cpu.r[4]);
    println!("unknown ports: {:x?}", m.dev.unknown_ports);
    if !m.console.is_empty() {
        println!("console: {}", m.console);
    }
    if let Stop::Fault(_) = stop {
        let tail: Vec<String> = m.cpu.recent().iter().rev().take(16).map(|(c, i)| format!("{c:04x}:{i:04x}")).collect();
        println!("recent (newest first): {}", tail.join(" "));
    }
}
