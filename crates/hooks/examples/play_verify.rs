//! Shadow-verify ported routines while replaying a scripted game: every call of a port in the
//! chosen tables runs the port and the original and compares them; the original's result is kept,
//! so the run is the unported game's run. The event syntax is the coverage runner's
//! (`crates/dos/examples/coverage.rs`, docs/COVERAGE.md), so its scenarios can be reused.
//!
//! Usage: cargo run --release -p frontier-hooks --example play_verify -- --secs N [events...]
//!   T:SC tap, T:SC+ / T:SC- hold and release, T:@X,Y[,r] click, T:~DX,DY mickeys,
//!   T:b1+ T:b1- T:b2+ T:b2- buttons, T:shot (frame to game-data/derived/captures/play-T.png).
//! Example (a hyperspace jump, tools/coverage/run.sh):
//!   --secs 80 20:39 24:02 33:3e 34:@129,18 35.5:41 … 70:42

use frontier_dos::{Machine, Stop, Vfs};

#[derive(Clone, Copy, Debug)]
enum Event {
    Tap(u8),
    Down(u8),
    Up(u8),
    Move(i32, i32),
    Place(i32, i32),
    Button(u16, bool),
    Shot,
}

/// The coverage runner's event syntax (a copy of its parser).
fn parse_event(s: &str) -> Option<Vec<(f64, Event)>> {
    let (t, what) = s.split_once(':')?;
    let t: f64 = t.parse().ok()?;
    let ev = match what {
        "cover" => return Some(vec![]),
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

fn main() {
    let mut secs = 80.0;
    let mut data = "game-data/original".to_string();
    let mut events = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).expect("--secs N"),
            "--data" => data = args.next().expect("--data DIR"),
            _ => events.extend(parse_event(&a).unwrap_or_else(|| panic!("bad event {a}"))),
        }
    }
    events.sort_by(|a: &(f64, Event), b| a.0.partial_cmp(&b.0).unwrap());
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir(&data).expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let tables = [
        ("FRONTIER.EXE", frontier_game::rng::PORTS),
        ("EL2M02.OVL", frontier_game::galaxy::PORTS_M02),
        ("EL2M09.OVL", frontier_game::names::PORTS_M09),
        ("EL2M00.OVL", frontier_game::system::PORTS_M00),
    ];
    let stats: Vec<_> = tables.iter().map(|(module, ports)| (*module, frontier_hooks::verify_ports(&mut m, module, ports))).collect();
    let slice = (m.dev.ips / 100.0) as u64;
    while m.now() < secs {
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
                Event::Shot => {
                    let mut rgba = vec![0u8; 256000];
                    m.frame_rgba(&mut rgba);
                    let p = format!("game-data/derived/captures/play-{t:06.2}.png");
                    std::fs::write(p, frontier_dos::png::encode_rgba(320, 200, &rgba)).unwrap();
                }
            }
            events.remove(0);
        }
        if m.run(slice) != Stop::Budget {
            println!("stopped at {:.2} s", m.now());
            break;
        }
    }
    println!("{:.1} virtual s", m.now());
    for (module, st) in &stats {
        let st = st.borrow();
        println!("{module}: {} calls verified, {} mismatches, {} declined, unported {:?}", st.calls, st.mismatches, st.declined, st.unported);
        println!("  per routine: {:?}", st.per_routine);
        for e in &st.examples {
            println!("  {e:x?}");
        }
    }
}
