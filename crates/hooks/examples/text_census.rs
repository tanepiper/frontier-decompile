//! Which text ids the native text dispatcher declines during a scripted game: every call of
//! `1000:e89d` runs the port on a copy (the original always runs), and declined ids are counted by
//! id (formatters by their target). For finding what to port next in the text system.
//!
//! Usage: cargo run --release -p frontier-hooks --example text_census -- --secs N [events...]
//! (the event syntax of play_verify / the coverage runner)

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use frontier_dos::cpu::Cpu;
use frontier_dos::{Machine, Stop, Vfs};
use frontier_game::math3::DECLINE;

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
    let mut events = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).expect("--secs N"),
            _ => events.extend(parse_event(&a).unwrap_or_else(|| panic!("bad event {a}"))),
        }
    }
    events.sort_by(|a: &(f64, Event), b| a.0.partial_cmp(&b.0).unwrap());
    let mut vfs = Vfs::new();
    for e in std::fs::read_dir("game-data/original").expect("game dir").flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let declined: Rc<RefCell<BTreeMap<u16, u64>>> = Rc::default();
    let calls: Rc<RefCell<u64>> = Rc::default();
    {
        let (declined, calls) = (declined.clone(), calls.clone());
        m.hook(
            "FRONTIER.EXE",
            0xe89d,
            Box::new(move |cpu: &mut Cpu, _limit: u64| {
                *calls.borrow_mut() += 1;
                let id = cpu.r[0] as u16;
                let out = frontier_hooks::run_port(cpu, frontier_game::text::MAIN_TEXT[0].3);
                if out.steps == DECLINE {
                    *declined.borrow_mut().entry(id).or_default() += 1;
                }
                false
            }),
        );
    }
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
                Event::Shot => {}
            }
            events.remove(0);
        }
        if m.run(slice) != Stop::Budget {
            break;
        }
    }
    let d = declined.borrow();
    let mut v: Vec<_> = d.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    println!("{} dispatcher calls, {} declined", calls.borrow(), d.values().sum::<u64>());
    for (id, n) in v.iter().take(25) {
        println!("  id {id:04x}: {n}");
    }
}
