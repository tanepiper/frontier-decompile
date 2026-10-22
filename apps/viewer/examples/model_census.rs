//! Which models the original draws, measured on the reference runtime.
//!
//! Usage: cargo run --release -p frontier-viewer --example model_census -- [game-dir] [seconds] [keys...]
//!
//! Boots the user's `FRONTIER.EXE` headless, watches the model lookup `1000:18b5` (slot
//! `DS:0050`: `BX` = model number × 2 into the table at `[DS:5701]:[DS:56ff]`) without
//! replacing it, and every second writes the frame to
//! `game-data/derived/captures/models-NN.png` and prints the models looked up in that second
//! (`main:n` for the default table in `FRONTIER.EXE`, `seg:base:n` for a module's table).
//! Objects drawn through `1000:3810` are printed first, as `obj:n` (their model number at `+5Ah`).
//! `keys` are `T:SC` pairs: at virtual second T press and release set-1 scancode SC (hex), or
//! `T:@X,Y` to move the mouse to X,Y (driver coordinates) and click the left button.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use frontier_dos::{Machine, Stop, Vfs};

#[derive(Clone, Copy)]
enum Input {
    Key(u8),
    Click(i32, i32),
}

/// Lookups per (table segment, base, model number) in the current second.
type Seen = Rc<RefCell<BTreeMap<(u16, u16, u16), u32>>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().cloned().unwrap_or_else(|| "game-data/original".into());
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    // `T:SC` presses scancode SC (hex); `T:@X,Y` clicks the left button at driver X,Y.
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
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut vfs = Vfs::new();
    for (name, data) in frontier_platform::files::read_dir(Path::new(&dir)) {
        vfs.insert(&name, data);
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    let seen: Seen = Rc::default();
    let s2 = seen.clone();
    m.hook(
        "FRONTIER.EXE",
        0x18b5,
        Box::new(move |cpu, _| {
            let ds = cpu.s[3] as u32;
            let seg = cpu.mem.rw((ds << 4) + 0x5701);
            let base = cpu.mem.rw((ds << 4) + 0x56ff);
            let bx = cpu.r[3] as u16;
            *s2.borrow_mut().entry((seg, base, bx / 2)).or_default() += 1;
            false // observe only: the original runs
        }),
    );
    // Every command fetch of the mesh interpreter (`1000:b207`): ES:SI at the three LODSW sites.
    // Consecutive fetches give each command's length (written to models-commands.txt).
    let fetches: Rc<RefCell<Vec<(u16, u16)>>> = Rc::default();
    for site in [0xb21e, 0xb243, 0xb268] {
        let f2 = fetches.clone();
        m.hook(
            "FRONTIER.EXE",
            site,
            Box::new(move |cpu, _| {
                let mut f = f2.borrow_mut();
                if f.len() < 4_000_000 {
                    f.push((cpu.s[0], cpu.r[6] as u16));
                }
                false
            }),
        );
    }
    // Objects drawn by `1000:3810` (slot `DS:00d8`): ES:SI is the object, its model number × 2
    // is the word at `+5Ah`. Printed as `obj:n` beside the lookups.
    let objects: Seen = Rc::default();
    let o2 = objects.clone();
    m.hook(
        "FRONTIER.EXE",
        0x3810,
        Box::new(move |cpu, _| {
            let es = cpu.s[0] as u32;
            let si = cpu.r[6] as u16 as u32;
            let n = cpu.mem.rw((es << 4) + si + 0x5a);
            *o2.borrow_mut().entry((0, 0, n / 2)).or_default() += 1;
            false
        }),
    );
    // The colour double word `[BP-9Eh]` a face adds when its colour has bit 4 set (`1000:5211`):
    // low word a colour, high word a texture entry. Printed as `dw:value(model offset)`.
    let colours: Seen = Rc::default();
    // The sites that add it: line 5211, triangle 554c, quad 5cf5, polygon 4c1f / 4f12, and the
    // second halves of the mirrored triangle and quad, 568b / 5e88.
    for site in [0x5211, 0x554c, 0x5cf5, 0x4c1f, 0x4f12, 0x568b, 0x5e88] {
        let c2 = colours.clone();
        m.hook(
            "FRONTIER.EXE",
            site,
            Box::new(move |cpu, _| {
                let ss = cpu.s[2] as u32;
                let bp = cpu.r[5] as u16;
                let dw_lo = cpu.mem.rw((ss << 4) + bp.wrapping_sub(0x9e) as u32);
                let dw_hi = cpu.mem.rw((ss << 4) + bp.wrapping_sub(0x9c) as u32);
                let model = cpu.mem.rw((ss << 4) + bp.wrapping_sub(4) as u32);
                *c2.borrow_mut().entry((dw_hi, dw_lo, model)).or_default() += 1;
                false
            }),
        );
    }
    let out = Path::new("game-data/derived/captures");
    std::fs::create_dir_all(out).unwrap();
    let slice = (m.dev.ips / 100.0) as u64;
    let (mut next, mut shot) = (1.0, 0);
    loop {
        let st = m.run(slice);
        while let Some(&(t, input)) = keys.first() {
            if m.now() < t {
                break;
            }
            match input {
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
        if m.now() >= next {
            let main = m.image_segment("FRONTIER.EXE").unwrap_or(0);
            let mut rgba = vec![0u8; 320 * 200 * 4];
            m.frame_rgba(&mut rgba);
            std::fs::write(out.join(format!("models-{shot:03}.png")), frontier_dos::png::encode_rgba(320, 200, &rgba)).unwrap();
            let list: Vec<String> = seen
                .borrow()
                .iter()
                .map(|(&(seg, base, n), c)| {
                    let table =
                        if seg == main.wrapping_add(0x412d) && base == 4 { "main".to_string() } else { format!("{seg:04x}:{base:x}") };
                    format!("{table}:{n}×{c}")
                })
                .collect();
            let objs: Vec<String> = objects.borrow().iter().map(|(&(_, _, n), c)| format!("obj:{n}×{c}")).collect();
            let dws: Vec<String> = colours.borrow().iter().map(|(&(hi, lo, md), c)| format!("dw:{hi:04x}{lo:04x}({md:04x})×{c}")).collect();
            println!("{shot:3} {:6.1}s  {}  |  {}  |  {}", m.now(), objs.join(" "), list.join(" "), dws.join(" "));
            colours.borrow_mut().clear();
            seen.borrow_mut().clear();
            objects.borrow_mut().clear();
            shot += 1;
            next += 1.0;
        }
        if st != Stop::Budget || m.now() >= secs {
            // (segment, command offset, length to the next fetch, word) for the agent to read.
            let f = fetches.borrow();
            let mut lines = String::new();
            let mut seen = std::collections::BTreeSet::new();
            for w in f.windows(2) {
                let ((s0, o0), (s1, o1)) = (w[0], w[1]);
                if seen.insert((s0, o0)) {
                    let word = m.cpu.mem.rw(((s0 as u32) << 4) + o0 as u32);
                    let len = if s0 == s1 { o1.wrapping_sub(o0) as i16 as i32 } else { 0 };
                    lines.push_str(&format!("{s0:04x} {o0:04x} {word:04x} {len}\n"));
                }
            }
            std::fs::write(out.join("models-commands.txt"), lines).unwrap();
            println!("stop {st:?} at {:.1}s", m.now());
            break;
        }
    }
}
