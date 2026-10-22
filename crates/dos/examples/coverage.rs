//! Record which original code runs during a scripted session, for finding the code behind one
//! action by comparing two runs with GameFrame's `coverdiff` (docs/COVERAGE.md).
//!
//! Usage: cargo run --release -p frontier-dos --example coverage -- [options] [events...]
//!
//!   --data DIR    the game files (default `game-data/original`)
//!   --out DIR     where to write (default `game-data/derived/coverage`); the run goes to DIR/NAME
//!   --name NAME   the run's name (default `run`)
//!   --secs N      virtual seconds to run (default 30)
//!
//! Events, at virtual second T (fractions allowed), in any order:
//!
//!   T:SC          press and release set-1 scancode SC (hex), e.g. `20:39` = space
//!   T:SC+ T:SC-   hold the key down, let it go
//!   T:@X,Y        click the left button with the game's pointer at screen pixel X,Y
//!   T:@X,Y,r      the same with the right button
//!   T:~DX,DY      move the mouse by DX,DY mickeys
//!   T:b1+ T:b1-   press, release mouse button 1 (left), 2 (right)
//!   T:cover       start recording here (default: from the start)
//!   T:shot        save the frame as DIR/NAME/shot-T.png
//!
//! The game moves its pointer by the driver's motion counters (`int 33h AX=000Bh`), half a
//! pixel per mickey (*measured*), not from the driver position. A click therefore pushes the
//! pointer into the top-left corner, moves it by 2·X, 2·Y mickeys 0.1 s later (and sets the driver position to
//! X,Y, which other code reads), holds the button
//! down from 0.2 s to 0.3 s.
//!
//! Input is applied at fixed 10 ms slice boundaries of virtual time, so two runs with the same
//! events are identical instruction for instruction, and a baseline and an action run differ
//! only after the action. No native hooks are installed: every instruction is the original's.
//!
//! Each image that ran gets `IMAGE.cov`, the Ghidra flat address (`0x10000` + offset from the
//! load segment) of every instruction that ran, one per line in hex, and `IMAGE.entries`, the
//! addresses entered by a call, far jump or interrupt. `frame.png` is the last frame.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use frontier_dos::{Machine, Stop, Vfs};

#[derive(Clone, Copy, Debug)]
enum Event {
    Tap(u8),
    Down(u8),
    Up(u8),
    /// Relative mouse motion in mickeys.
    Move(i32, i32),
    /// Set the driver's position (`int 33h AX=0003h`) without motion.
    Place(i32, i32),
    Button(u16, bool),
    Cover,
    Shot,
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

fn parse_event(s: &str) -> Option<Vec<(f64, Event)>> {
    let (t, what) = s.split_once(':')?;
    let t: f64 = t.parse().ok()?;
    let ev = match what {
        "cover" => Event::Cover,
        "shot" => Event::Shot,
        "b1+" => Event::Button(1, true),
        "b1-" => Event::Button(1, false),
        "b2+" => Event::Button(2, true),
        "b2-" => Event::Button(2, false),
        _ if what.starts_with('~') => {
            let (x, y) = what[1..].split_once(',')?;
            Event::Move(x.parse().ok()?, y.parse().ok()?)
        }
        _ => match what.strip_prefix('@') {
            Some(xy) => {
                let mut it = xy.split(',');
                let x: i32 = it.next()?.parse().ok()?;
                let y: i32 = it.next()?.parse().ok()?;
                let b = if it.next() == Some("r") { 2 } else { 1 };
                return Some(vec![
                    (t, Event::Move(-4000, -4000)),
                    (t + 0.1, Event::Move(2 * x, 2 * y)),
                    (t + 0.1, Event::Place(x, y)),
                    (t + 0.2, Event::Button(b, true)),
                    (t + 0.3, Event::Button(b, false)),
                ]);
            }
            None => match what.as_bytes().last()? {
                b'+' => Event::Down(u8::from_str_radix(&what[..what.len() - 1], 16).ok()?),
                b'-' => Event::Up(u8::from_str_radix(&what[..what.len() - 1], 16).ok()?),
                _ => Event::Tap(u8::from_str_radix(what, 16).ok()?),
            },
        },
    };
    Some(vec![(t, ev)])
}

fn save_frame(m: &Machine, path: &Path) {
    let mut rgba = vec![0u8; 320 * 200 * 4];
    m.frame_rgba(&mut rgba);
    std::fs::write(path, frontier_dos::png::encode_rgba(320, 200, &rgba)).unwrap();
}

fn main() {
    let mut data = PathBuf::from("game-data/original");
    let mut out = PathBuf::from("game-data/derived/coverage");
    let mut name = "run".to_string();
    let mut secs = 30.0;
    let mut events = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data = args.next().expect("--data DIR").into(),
            "--out" => out = args.next().expect("--out DIR").into(),
            "--name" => name = args.next().expect("--name NAME"),
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).expect("--secs N"),
            _ => events.extend(parse_event(&a).unwrap_or_else(|| panic!("bad event {a}"))),
        }
    }
    events.sort_by(|a: &(f64, Event), b| a.0.partial_cmp(&b.0).unwrap());
    let dir = out.join(&name);
    std::fs::create_dir_all(&dir).unwrap();

    let mut vfs = Vfs::new();
    load_dir(&mut vfs, &data, "");
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    if !events.iter().any(|e| matches!(e.1, Event::Cover)) {
        m.coverage_start();
    }
    let slice = (m.dev.ips / 100.0) as u64;
    let t0 = std::time::Instant::now();
    let mut printed = 0;
    let stop = loop {
        while let Some(&(t, ev)) = events.first() {
            if m.now() < t {
                break;
            }
            match ev {
                Event::Tap(sc) => {
                    m.key(sc);
                    m.key(sc | 0x80);
                }
                Event::Down(sc) => m.key(sc),
                Event::Up(sc) => m.key(sc | 0x80),
                Event::Move(dx, dy) => m.mouse_move(dx, dy),
                Event::Place(x, y) => (m.mouse.x, m.mouse.y) = (x, y),
                Event::Button(b, down) => m.mouse_button(b, down),
                Event::Cover => m.coverage_start(),
                Event::Shot => save_frame(&m, &dir.join(format!("shot-{t:06.2}.png"))),
            }
            events.remove(0);
        }
        let st = m.run(slice);
        for l in &m.log[printed..] {
            println!("[{:8.3}] {l}", m.now());
        }
        printed = m.log.len();
        if st != Stop::Budget || m.now() >= secs {
            break st;
        }
    };
    save_frame(&m, &dir.join("frame.png"));
    let cov = m.coverage_take();
    let mut summary = format!("run {name}: {stop:?} at {:.2} s virtual, {} instructions\n", m.now(), m.cpu.icount);
    for (image, c) in &cov {
        let base = if image == "linear" { 0 } else { 0x10000 };
        let lines = |set: &std::collections::BTreeSet<u32>| set.iter().fold(String::new(), |mut s, o| {
            let _ = writeln!(s, "{:x}", base + o);
            s
        });
        std::fs::write(dir.join(format!("{image}.cov")), lines(&c.exec)).unwrap();
        std::fs::write(dir.join(format!("{image}.entries")), lines(&c.entries)).unwrap();
        let _ = writeln!(summary, "  {image:14} {:6} instructions, {:5} entries", c.exec.len(), c.entries.len());
    }
    std::fs::write(dir.join("summary.txt"), &summary).unwrap();
    print!("{summary}");
    println!("host {:.1} s", t0.elapsed().as_secs_f64());
}
