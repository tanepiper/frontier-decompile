//! Shadow-verify the main program's random number generators during play: every call of a ported
//! generator runs the port and the original and compares them; the original's result is kept.
//!
//! Usage: cargo run --release -p frontier-hooks --example rng_verify -- [game-dir] [seconds]

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
    let second = m.dev.ips as u64;
    let mut stage = 0;
    while m.now() < secs {
        if m.run(second / 10) != Stop::Budget {
            println!("stopped");
            break;
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
    let st = stats.borrow();
    println!("{:.1} virtual s: {} calls verified, {} mismatches, {} declined", m.now(), st.calls, st.mismatches, st.declined);
    println!("per routine: {:?}", st.per_routine);
    for e in &st.examples {
        println!("  {e:x?}");
    }
}
