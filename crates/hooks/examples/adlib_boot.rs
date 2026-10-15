//! Boot the game twice, once with the original AdLib driver and once with the native port
//! (`frontier_hooks::adlib`), press through the intro into a game, and compare the frames and
//! every audio sample (docs/AUDIO.md).
//!
//! Usage: cargo run --release -p frontier-hooks --example adlib_boot -- [game-dir] [seconds] [--verify]
//! `--verify` instead runs the original alongside the port at every driver call and compares.

use frontier_dos::{Machine, Stop, Vfs};

fn machine(dir: &str) -> Machine {
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir(dir).expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.dev.adlib.capture = true;
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
    let verify = args.iter().any(|a| a == "--verify");
    let mut plain = machine(&dir);
    let mut native = machine(&dir);
    let stats = if verify { Some(frontier_hooks::adlib::verify(&mut native)) } else {
        frontier_hooks::adlib::install(&mut native);
        None
    };
    let slice = (native.dev.ips / 100.0) as u64;
    let (mut s1, mut s2) = (0u8, 0u8);
    let (mut a, mut b) = (vec![0u8; 256000], vec![0u8; 256000]);
    let (mut frame_diff, mut audio_diff) = (None, None);
    let (mut samples, mut nonzero) = (0u64, 0u64);
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
        if a != b && frame_diff.is_none() {
            frame_diff = Some(native.now());
        }
        let now = native.now();
        native.dev.adlib.advance(now);
        let now = plain.now();
        plain.dev.adlib.advance(now);
        let (x, y) = (std::mem::take(&mut native.dev.adlib.samples), std::mem::take(&mut plain.dev.adlib.samples));
        if x != y && audio_diff.is_none() {
            audio_diff = Some(native.now());
        }
        samples += x.len() as u64;
        nonzero += x.iter().filter(|&&s| s != 0).count() as u64;
    }
    println!("{:.1} virtual s in {:.1} host s; native driver calls {}", native.now(), t0.elapsed().as_secs_f64(), native.hook_calls);
    println!("{samples} samples, {nonzero} non-zero");
    if let Some(st) = stats {
        let st = st.borrow();
        println!("shadow verify: {} calls, {} mismatches, {} declined; per export {:?}", st.calls, st.mismatches, st.declined, st.per_export);
        for e in &st.examples {
            println!("  {e:x?}");
        }
    }
    match frame_diff {
        None => println!("frames identical to the original driver"),
        Some(t) => println!("frames first differ at {t:.2} s"),
    }
    match audio_diff {
        None => println!("audio identical to the original driver, sample for sample"),
        Some(t) => println!("audio first differs at {t:.2} s"),
    }
}
