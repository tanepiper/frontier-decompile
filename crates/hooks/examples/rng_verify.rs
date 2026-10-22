//! Shadow-verify the main program's random number generators during play: every call of a ported
//! generator runs the port and the original and compares them; the original's result is kept.
//!
//! Usage: cargo run --release -p frontier-hooks --example rng_verify -- [game-dir] [seconds] [T:SC ...]
//! `T:SC` presses set-1 scancode SC (hex) at virtual second T. Also verifies the galaxy ports.

use frontier_dos::{Machine, Stop, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir(&dir).expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let stats = frontier_hooks::verify_ports(&mut m, "FRONTIER.EXE", frontier_game::rng::PORTS);
    let galaxy = frontier_hooks::verify_ports(&mut m, "EL2M02.OVL", frontier_game::galaxy::PORTS_M02);
    let mut keys: Vec<(f64, u8)> = args.iter().skip(2).filter_map(|k| {
        let (t, sc) = k.split_once(':')?;
        Some((t.parse().ok()?, u8::from_str_radix(sc, 16).ok()?))
    }).collect();
    let second = m.dev.ips as u64;
    let mut stage = 0;
    while m.now() < secs {
        if m.run(second / 10) != Stop::Budget {
            println!("stopped");
            break;
        }
        while let Some(&(t, sc)) = keys.first() {
            if m.now() < t {
                break;
            }
            m.key(sc);
            m.key(sc | 0x80);
            keys.remove(0);
        }
        // Space at 20 s (skip intro), start option 1 at 24 s (docs/RUNTIME.md).
        if stage == 0 && m.now() >= 20.0 {
            m.key(0x39);
            m.key(0xB9);
            stage = 1;
        }
        if stage == 1 && m.now() >= 24.0 {
            let (dx, dy) = (124 - m.mouse.x, 90 - m.mouse.y);
            m.mouse_move(dx, dy);
            m.mouse_button(1, true);
            m.mouse_button(1, false);
            stage = 2;
        }
    }
    let g = galaxy.borrow();
    println!("galaxy: {} calls verified, {} mismatches, {} declined, unported {:?}", g.calls, g.mismatches, g.declined, g.unported);
    for e in &g.examples {
        println!("  {e:x?}");
    }
    let mut rgba = vec![0u8; 256000];
    m.frame_rgba(&mut rgba);
    std::fs::write("game-data/derived/captures/verify-last.png", frontier_dos::png::encode_rgba(320, 200, &rgba)).unwrap();
    let st = stats.borrow();
    println!("{:.1} virtual s: {} calls verified, {} mismatches, {} declined", m.now(), st.calls, st.mismatches, st.declined);
    println!("per routine: {:?}", st.per_routine);
    for e in &st.examples {
        println!("  {e:x?}");
    }
}
