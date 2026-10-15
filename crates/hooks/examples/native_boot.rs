//! Boot the original with every native port installed, start a game, and report how often the
//! ported routines ran and whether the frames match a run without ports.
//!
//! Usage: cargo run --release -p frontier-hooks --example native_boot -- [game-dir] [seconds] [--verify]
//! `--verify` runs the original alongside each port and compares (timing identical to no ports).
//! Writes game-data/derived/captures/native-NN.png (with ports) every 5 virtual seconds.

use std::path::Path;

use frontier_dos::{Machine, Stop, Vfs};

fn machine(dir: &str) -> Machine {
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir(dir).expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    m
}

/// Space at 20 s (skip intro), click start option 1 at 24 s (docs/RUNTIME.md).
fn script(m: &mut Machine, t: f64, done: &mut u8) {
    if *done == 0 && t >= 20.0 {
        m.key(0x39);
        m.key(0xB9);
        *done = 1;
    }
    if *done == 1 && t >= 24.0 {
        let (dx, dy) = (124 - m.mouse.x, 90 - m.mouse.y);
        m.mouse_move(dx, dy);
        m.mouse_button(1, true);
        m.mouse_button(1, false);
        *done = 2;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(40.0);
    let mut plain = machine(&dir);
    let verify = args.iter().any(|a| a == "--verify");
    let mut native = machine(&dir);
    let stats = if verify { Some(frontier_hooks::verify_math3(&mut native)) } else {
        frontier_hooks::install_all(&mut native);
        None
    };
    println!("{} hooks installed", native.hook_count());
    let out = Path::new("game-data/derived/captures");
    std::fs::create_dir_all(out).unwrap();
    let slice = (native.dev.ips / 100.0) as u64;
    let (mut s1, mut s2) = (0u8, 0u8);
    let mut shot = 0;
    let (mut a, mut b) = (vec![0u8; 256000], vec![0u8; 256000]);
    let mut first_diff = None;
    let t0 = std::time::Instant::now();
    while native.now() < secs {
        let st = native.run(slice);
        plain.run(slice);
        let t = native.now();
        script(&mut native, t, &mut s1);
        let t = plain.now();
        script(&mut plain, t, &mut s2);
        if st != Stop::Budget {
            println!("native stopped: {st:?}");
            break;
        }
        native.frame_rgba(&mut a);
        plain.frame_rgba(&mut b);
        if a != b && first_diff.is_none() {
            first_diff = Some(native.now());
        }
        if native.now() >= shot as f64 * 5.0 {
            std::fs::write(out.join(format!("native-{shot:02}.png")), frontier_dos::png::encode_rgba(320, 200, &a)).unwrap();
            shot += 1;
        }
    }
    println!("{:.1} virtual s in {:.1} host s; ported routines ran {} times", native.now(), t0.elapsed().as_secs_f64(), native.hook_calls);
    if let Some(st) = stats {
        let st = st.borrow();
        println!("shadow verify: {} calls, {} mismatches, {} declined (interrupt due inside)", st.calls, st.mismatches, st.declined);
        for e in &st.examples {
            println!("  {e:x?}");
        }
    }
    match first_diff {
        None => println!("frames identical to the run without ports"),
        Some(t) => println!("frames first differ at {t:.2} s"),
    }
}
