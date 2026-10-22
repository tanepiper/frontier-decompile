//! Parity: the main program's random number generators (`frontier_game::rng`) against the
//! original routines in `FRONTIER.EXE`, run on the reference runtime inside the booted game.
//! Registers, the generator state in `DS:1fd9`–`1fe0`, every byte written and the instruction
//! count are compared. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{DS, ES, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::rng::{self, PORTS, STATE_S0, STATE_S1};
use frontier_hooks::{compare, record_original, regs_from, run_port};

fn booted() -> Option<Machine> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/original");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: {} missing (run tools/extract-assets.sh)", dir.display());
        return None;
    };
    let mut vfs = Vfs::new();
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            vfs.insert(&e.file_name().to_string_lossy(), std::fs::read(e.path()).unwrap());
        }
    }
    let mut m = Machine::new(vfs);
    m.boot("FRONTIER.EXE").unwrap();
    m.run(5_000_000);
    Some(m)
}

struct X(u64);
impl X {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as u32
    }
}

#[test]
fn rng_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let main = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg;
    let ds = main + 0x1615;
    let mut x = X(0x2545_F491_4F6C_DD1D);
    let mut failures = Vec::new();
    for &(off, slot, name, port) in PORTS {
        let mut mismatches = 0;
        let cases = 20_000;
        for _ in 0..cases {
            for r in 0..8 {
                m.cpu.r[r] = x.next();
            }
            m.cpu.r[ESP] = 0xFFF0;
            m.cpu.s[DS] = ds;
            m.cpu.s[ES] = 0xB000;
            m.cpu.s[SS] = 0xA000;
            m.cpu.mem.wd(lin(ds, STATE_S0 as u32), x.next());
            m.cpu.mem.wd(lin(ds, STATE_S1 as u32), x.next());
            m.enter(main, off);
            let input = regs_from(&m.cpu);
            let out = run_port(&m.cpu, port);
            let diff = match record_original(&mut m.cpu, 10_000) {
                Some(orig) => compare(&out, &orig, &m.cpu.mem),
                None => Some("the original did not return".into()),
            };
            if let Some(d) = diff {
                if mismatches == 0 {
                    failures.push(format!("{name} (1000:{off:04x}, slot {slot:04x}): input {input:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{cases} mismatches"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The state the game holds after boot is the documented seed advanced by the draws made so far:
/// replaying `rng::step`/`step32` from the seed reaches it (bounded search), which ties the port's
/// state model to the running game.
#[test]
fn seed_is_where_start_up_puts_it() {
    let Some(m) = booted() else { return };
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let now = (m.cpu.mem.rd(lin(ds, STATE_S0 as u32)), m.cpu.mem.rd(lin(ds, STATE_S1 as u32)));
    // Search short mixed sequences of the two generators from the seed.
    let mut frontier = vec![rng::SEED];
    for _ in 0..12 {
        if frontier.contains(&now) {
            return;
        }
        let mut next = Vec::new();
        for &(a, b) in &frontier {
            next.push(rng::step(a, b));
            next.push(rng::step32(a, b).0);
        }
        next.sort();
        next.dedup();
        frontier = next;
    }
    assert!(frontier.contains(&now), "state after boot {now:08x?} is not within 12 draws of the seed");
}
