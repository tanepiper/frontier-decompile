//! Parity: the native AdLib driver (`frontier_game::adlib`) against the original
//! `EL2ADLIB.OVL` in the booted game. Every song is played for 30 virtual seconds, every effect
//! is started through each export, and engine, jukebox and shutdown paths are exercised. At
//! each call the port and the original start from the same state; registers, every byte
//! written, every OPL port write with the instruction count at which it happens, and the total
//! instruction count are compared. The original's results are kept between calls. Skips when
//! game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{CS, DS, EAX, EBX, ECX, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::adlib::EXPORTS;
use frontier_hooks::adlib::{compare, record_original, run_port};

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
    while m.now() < 1.0 {
        m.run(1_000_000);
    }
    assert!(m.image_segment("EL2ADLIB.OVL").is_some(), "EL2ADLIB not loaded");
    Some(m)
}

const STACK: u16 = 0xA000;

struct Harness {
    m: Machine,
    seg: u16,
    ds: u16,
    calls: u64,
    outs: u64,
    ops: u128,
    failures: Vec<String>,
}

impl Harness {
    fn export(name: &str) -> (u16, fn(&mut frontier_game::adlib::Driver)) {
        let e = EXPORTS.iter().find(|e| e.2 == name).unwrap();
        (e.0, e.3)
    }

    /// Call export `name` with AX, BX, CX on both, compare, keep the original's result.
    fn call(&mut self, name: &str, ax: u16, bx: u16, cx: u16, context: &str) {
        let (off, f) = Self::export(name);
        let cpu = &mut self.m.cpu;
        cpu.r[EAX] = ax as u32;
        cpu.r[EBX] = bx as u32;
        cpu.r[ECX] = cx as u32;
        cpu.s[SS] = STACK;
        cpu.r[ESP] = 0xFFF0;
        cpu.s[DS] = self.ds;
        cpu.push16(0x1234);
        cpu.push16(0x5678);
        cpu.s[CS] = self.seg;
        cpu.ip = off as u32;
        let port = run_port(cpu, f);
        let Some(orig) = record_original(cpu, 50_000_000) else {
            self.failures.push(format!("{context}: {name}: original did not return"));
            return;
        };
        self.calls += 1;
        self.outs += port.outs.len() as u64;
        self.ops |= port.ops;
        if let Some(d) = compare(&port, &orig, &cpu.mem) {
            if self.failures.len() < 12 {
                self.failures.push(format!("{context}: {name}(ax={ax:04x} bx={bx:04x} cx={cx:04x}): {d}"));
            }
        }
    }

    fn ticks(&mut self, n: usize, context: &str) {
        for i in 0..n {
            if self.failures.len() >= 12 {
                return;
            }
            self.call("tick", 0, 0, 0, &format!("{context} tick {i}"));
        }
    }

    fn set_ds(&mut self, off: u16, v: u8) {
        self.m.cpu.mem.wb(lin(self.ds, off as u32), v);
    }
}

#[test]
fn adlib_port_matches_original() {
    let Some(m) = booted() else { return };
    let seg = m.image_segment("EL2ADLIB.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let mut h = Harness { m, seg, ds, calls: 0, outs: 0, ops: 0, failures: Vec::new() };
    // Sound options on (effects, effect 14h, music), every song enabled, range effects active.
    for off in [0x4636, 0x463a, 0x463b] {
        h.set_ds(off, 1);
    }
    h.set_ds(0x463f, 0);
    for n in 0..12 {
        h.set_ds(0x4640 + n, 1);
    }
    h.set_ds(0x5709, 1);

    // What the intro left playing.
    h.ticks(2000, "intro");
    // Every song, 30 s each.
    for song in 0..12u16 {
        h.call("stop_music", 0, 0, 0, "songs");
        h.call("play_song", song, 0, 0, "songs");
        h.ticks(6000, &format!("song {song}"));
    }
    // Every effect through each export, with time to play out.
    h.call("silence", 0, 0, 0, "effects");
    for fx in 0..0x30u16 {
        h.call("effect", fx, 0, 0, "effects");
        h.ticks(150, &format!("effect {fx:02x}"));
        h.call("effect_with_volume", fx, 0x0100, 0, "effects");
        h.call("effect_with_volume", fx, 0, 0, "effects");
        h.ticks(150, &format!("effect {fx:02x} with volume"));
        h.call("effect_at_range", fx, (fx % 16) as u16, 0, "effects");
        h.ticks(150, &format!("effect {fx:02x} at range"));
    }
    // The effects with long streams (driver commands 01h and 17h), played out.
    for fx in [0x14u16, 0x1c] {
        h.call("silence", 0, 0, 0, "long effects");
        h.call("effect", fx, 0, 0, "long effects");
        h.ticks(6000, &format!("long effect {fx:02x}"));
    }
    // Retriggered timed effects, the engine at several volumes and pitches.
    for fx in [0x18u16, 0x0d, 0x0b, 0x1d] {
        h.call("effect_with_volume", fx, 1, 0, "timed");
        h.ticks(100, "timed");
        h.call("effect_with_volume", fx, 1, 0, "timed");
        h.ticks(700, "timed");
    }
    for (bx, cx) in [(0u16, 0xAEu16), (20, 0x100), (63, 0x200), (200, 0x50), (40, 0x400)] {
        h.call("engine", 0, bx, cx, "engine");
        h.ticks(60, "engine");
    }
    h.ticks(400, "engine off");
    // Jukebox: random songs back to back.
    h.set_ds(0x463f, 1);
    h.call("stop_music_unless_jukebox", 0, 0, 0, "jukebox");
    h.set_ds(0x463f, 0);
    h.call("stop_music_unless_jukebox", 0, 0, 0, "jukebox");
    h.set_ds(0x463f, 1);
    h.call("play_song", 0xFFFF, 0, 0, "jukebox");
    h.call("play_song", 0xFFFE, 0, 0, "jukebox");
    h.ticks(20000, "jukebox");
    // Music off, a disabled song, and the shutdown / init pair.
    h.set_ds(0x463b, 0);
    h.call("play_song", 3, 0, 0, "music off");
    h.set_ds(0x463b, 1);
    h.set_ds(0x4643, 0);
    h.call("stop_music", 0, 0, 0, "disabled");
    h.call("play_song", 3, 0, 0, "disabled");
    h.ticks(200, "disabled");
    h.call("shutdown", 0, 0, 0, "shutdown");
    h.call("init", 0, 0, 0, "init");
    h.ticks(200, "after init");

    // Ops no shipped stream reaches in these runs (12h slide off, 1Eh volume), in a stream
    // placed in free memory of the driver's segment and started through song 11's command.
    let stream: [u8; 12] = [0x00, 0x7f, 0x91, 0x40, 0x00, 0x20, 0x9e, 0x08, 0x92, 0x34, 0x10, 0x88];
    let base = 0xF000u16;
    for (i, b) in stream.iter().enumerate() {
        h.m.cpu.mem.wb(lin(h.seg, base as u32 + i as u32), *b);
    }
    h.m.cpu.mem.wb(lin(h.seg, base as u32 + 12), 0x00);
    let table = h.m.cpu.mem.rw(lin(h.seg, 0x0588));
    h.m.cpu.mem.ww(lin(h.seg, table as u32 + 0x3d * 2), base);
    h.call("stop_music", 0, 0, 0, "synthetic");
    h.set_ds(0x4640 + 11, 1);
    h.call("play_song", 11, 0, 0, "synthetic");
    h.ticks(300, "synthetic");

    assert!(h.failures.is_empty(), "{} calls, mismatches:\n{}", h.calls, h.failures.join("\n"));
    let seen: Vec<String> = (0..0x71).filter(|i| h.ops & (1u128 << i) != 0).map(|i| format!("{i:02x}")).collect();
    eprintln!("{} driver calls, {} port writes matched; byte-code ops covered: {}", h.calls, h.outs, seen.join(" "));
    assert!(h.outs > 100_000, "too little music played to be a meaningful comparison");
}
