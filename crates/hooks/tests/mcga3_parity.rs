//! Parity: every `EL2MCGA3` port in `frontier_game::video` against the original routine, run on
//! the reference runtime inside the booted game (real DS, tables, back buffer, `int 0` handler).
//! Registers, every byte written (the frame buffer included), the VGA port accesses with their
//! timing, the VGA state and the instruction count are compared. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{DS, EAX, EBP, EBX, ECX, EDI, EDX, ES, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::video::{self, EXPORTS};
use frontier_hooks::state;
use frontier_hooks::video::{compare, record_original, run_port, Shadow};

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
    // Past start-up: the driver initialised (back buffer, EL2G.GRF) and the intro drawing.
    m.run(50_000_000);
    assert!(m.image_segment("EL2MCGA3.OVL").is_some(), "EL2MCGA3 not loaded");
    Some(m)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as u16
    }
    fn below(&mut self, n: u16) -> u16 {
        self.next() % n
    }
    fn dword(&mut self) -> u32 {
        (self.next() as u32) << 16 | self.next() as u32
    }
    /// A coordinate: mostly on screen, sometimes just off it, sometimes anything.
    fn coord(&mut self, size: u16) -> u16 {
        match self.below(10) {
            0 => self.next(),
            1 => self.below(64).wrapping_sub(32),
            2 => size.wrapping_sub(16).wrapping_add(self.below(48)),
            3 => self.below(size + 200).wrapping_sub(100),
            _ => self.below(size),
        }
    }
}

/// Scratch stack segment, clear of everything the routines touch.
const STACK: u16 = 0x9000;

/// Inputs for one call of export `name`; returns false if this export has no generator.
fn setup(m: &mut Machine, rng: &mut Rng, name: &str, ds: u16) -> bool {
    let target = if rng.below(2) == 0 { 0xA000 } else { m.cpu.mem.rw(lin(ds, video::BACK_BUFFER as u32)) };
    match name {
        "triangle" | "triangle_clipped" | "quad" | "quad_clipped" => {
            let inside = name == "triangle" || name == "quad";
            let (w, h) = (video::WIDTH, video::VIEW_HEIGHT);
            let c = |r: &mut Rng, s: u16| if inside { r.below(s) } else { r.coord(s) };
            let v = [c(rng, w), c(rng, h), c(rng, w), c(rng, h), c(rng, w), c(rng, h)];
            for (r, val) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            let (x3, y3) = (c(rng, w), c(rng, h));
            m.cpu.mem.ww(lin(ds, video::QUAD_X3 as u32), x3);
            m.cpu.mem.ww(lin(ds, video::QUAD_Y3 as u32), y3);
            m.cpu.s[ES] = rng.next();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            // Stale edge starts are read when an edge is horizontal.
            for a in [video::START_A, video::START_B] {
                m.cpu.mem.wd(lin(ds, a as u32), rng.dword());
            }
            true
        }
        "line" => {
            // Rows are not clipped: keep them on the screen (or a little off it, which writes
            // past the frame within the segment, as the original does).
            let v = [rng.coord(video::WIDTH), rng.below(200), rng.coord(video::WIDTH), rng.below(200)];
            for (r, val) in [EAX, EBX, ECX, EDX].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            if rng.below(8) == 0 {
                // Short and degenerate lines.
                m.cpu.r[ECX] = (m.cpu.r[ECX] & 0xFFFF_0000) | (v[0].wrapping_add(rng.below(5)).wrapping_sub(2)) as u32;
                m.cpu.r[EDX] = (m.cpu.r[EDX] & 0xFFFF_0000) | (v[1].wrapping_add(rng.below(5)).wrapping_sub(2)) as u32;
            }
            m.cpu.r[ESI] = rng.dword();
            m.cpu.r[EBP] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "disc" | "disc_clipped" => {
            let size = match rng.below(6) {
                0 => rng.below(8),
                1 => rng.next(),
                2 => 0x3e0 + rng.below(32),
                _ => rng.below(200),
            };
            m.cpu.r[EBX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::WIDTH) as u32;
            m.cpu.r[ECX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::VIEW_HEIGHT) as u32;
            if name == "disc" && rng.below(2) == 0 {
                // The unchecked entry: keep the disc touching the view.
                m.cpu.r[ECX] = (m.cpu.r[ECX] & 0xFFFF_0000) | rng.below(video::VIEW_HEIGHT) as u32;
            }
            m.cpu.r[EDX] = (rng.dword() & 0xFFFF_0000) | size as u32;
            m.cpu.r[EAX] = rng.dword();
            m.cpu.r[ESI] = rng.dword();
            m.cpu.r[EBP] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "blob" => {
            // Start at a shape: the byte after a terminator (or the table start).
            let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
            let base = lin(seg, video::BLOB_TABLE as u32);
            let mut starts = vec![0u16];
            for i in 0..0x1c0u16 {
                if (m.cpu.mem.rb(base + i as u32) as i8) < 0 {
                    starts.push(i + 1);
                }
            }
            starts.pop();
            let bp = starts[rng.below(starts.len() as u16) as usize];
            m.cpu.r[EBP] = (rng.dword() & 0xFFFF_0000) | bp as u32;
            m.cpu.r[EBX] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::WIDTH) as u32;
            m.cpu.r[EDI] = (rng.dword() & 0xFFFF_0000) | rng.coord(video::VIEW_HEIGHT) as u32;
            m.cpu.r[EAX] = rng.dword();
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "fill_rows" => {
            // The caller's frame starts after the return address the call pushes (SP = 7FFCh).
            let bp = 0x7FFCu16;
            let at = |o: u16| lin(STACK, bp.wrapping_add(o) as u32);
            let inside = rng.below(2) == 0;
            let rows = match rng.below(8) {
                0 => rng.below(4),
                _ => 1 + rng.below(video::VIEW_HEIGHT),
            };
            m.cpu.mem.ww(at(video::ROWS_COUNT), rows);
            let top = if rng.below(4) == 0 { rng.next() % 200 } else { rng.below(rows.max(1)) };
            m.cpu.mem.ww(at(video::ROWS_TOP), top);
            m.cpu.mem.ww(at(video::ROWS_INSIDE), if inside { 1 + rng.below(5) } else { 0 });
            for y in 0..video::VIEW_HEIGHT {
                let end = [0, 0x2a0, 0x540, 0x7e0][if rng.below(5) == 0 { 0 } else { rng.below(4) } as usize];
                m.cpu.mem.ww(at(4 + 2 * y), end);
                for k in 1..8 {
                    let v = if inside { rng.below(video::WIDTH) } else { rng.coord(video::WIDTH) };
                    m.cpu.mem.ww(at(4 + 2 * y + k * video::ROWS_STRIDE), v);
                }
            }
            for r in [EAX, EBX, ECX, EDX, ESI] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.wb(lin(ds, video::COLOUR as u32), rng.next() as u8);
            true
        }
        "fill_masks" | "fill_masks_args" => {
            // SP = 7FFCh after the call; with arguments, 8 words above the return address.
            let sp = 0x7FFCu16;
            let bp = if name == "fill_masks_args" { sp + 16 } else { sp };
            for i in 0..8 {
                m.cpu.mem.ww(lin(STACK, (sp + 4 + 2 * i) as u32), rng.next());
            }
            for w in [0x7bcu16, 0x7c0, 0x7c4, 0x7c8, 0x7cc, 0x7d0, 0x7d4, 0x7d8] {
                m.cpu.mem.ww(lin(ds, w as u32), rng.next());
            }
            let at = |o: u16| lin(STACK, bp.wrapping_add(o) as u32);
            let rows = match rng.below(8) {
                0 => rng.below(3),
                1 => 160 + rng.below(16),
                _ => 1 + rng.below(video::VIEW_HEIGHT),
            };
            m.cpu.mem.ww(at(6), rows);
            for i in 0..0x40u16 {
                m.cpu.mem.wb(at(8 + i), rng.next() as u8);
            }
            for y in 0..176u16 {
                let list = 0x4a + 0x80 * y;
                let k = if rng.below(4) == 0 { 0 } else { rng.below(10) };
                m.cpu.mem.ww(at(list), 4 * k);
                for j in 0..40u16 {
                    let v = match rng.below(4) {
                        0 => 0,
                        1 => 2 * rng.below(24),
                        _ => rng.coord(video::WIDTH),
                    };
                    m.cpu.mem.ww(at(list + 2 + 2 * j), v);
                }
                m.cpu.mem.ww(at(0x544a + 2 * y), 2 * rng.below(24));
            }
            m.cpu.mem.ww(at(0x544a + 0x14e), 2 * rng.below(24));
            for r in [EAX, EBX, ECX, EDX, ESI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            true
        }
        "glyph" | "text_width" | "char_width" | "fill_rect" | "fill_rect_masked" | "pixel" | "target_back"
        | "target_screen" | "clear_view_7f" | "clear_view_0" => {
            for r in [EAX, EBX, ECX, EDX, ESI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            let set = |m: &mut Machine, r: usize, v: u16| m.cpu.r[r] = (m.cpu.r[r] & 0xFFFF_0000) | v as u32;
            match name {
                "glyph" => {
                    set(m, EAX, rng.below(0x60) + if rng.below(4) == 0 { 0 } else { 0x20 });
                    set(m, ECX, rng.below(312));
                    set(m, EDX, if rng.below(8) == 0 { rng.next() } else { rng.below(192) });
                }
                "text_width" => {
                    // A string in scratch memory, DS pointing at it.
                    let len = rng.below(24);
                    for i in 0..len {
                        let c = if rng.below(8) == 0 { rng.next() as u8 } else { 0x20 + rng.below(0x60) as u8 };
                        m.cpu.mem.wb(lin(0x8000, (0x100 + i) as u32), c.max(1));
                    }
                    m.cpu.mem.wb(lin(0x8000, (0x100 + len) as u32), 0);
                    set(m, ESI, 0x100);
                }
                "fill_rect" | "fill_rect_masked" => {
                    let x0 = rng.below(320);
                    let x1 = if rng.below(16) == 0 { x0.wrapping_sub(1) } else { x0 + rng.below(320 - x0) };
                    let y0 = rng.below(200);
                    let mut y1 = if rng.below(16) == 0 { rng.below(200) } else { y0 + rng.below(200 - y0) };
                    if x1 == x0.wrapping_sub(1) {
                        // Width 0 fills 65536 bytes a row: keep it to a row or two.
                        y1 = y0 + rng.below(2);
                    }
                    set(m, EAX, x0);
                    set(m, EBX, x1);
                    m.cpu.r[ECX] = (m.cpu.r[ECX] & !0xFF) | y0 as u32;
                    m.cpu.r[EDX] = (m.cpu.r[EDX] & !0xFF) | y1 as u32;
                }
                "pixel" => {
                    set(m, EAX, rng.below(320));
                    m.cpu.r[EBX] = (m.cpu.r[EBX] & !0xFF) | rng.below(200) as u32;
                }
                _ => {}
            }
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            true
        }
        "save_under" | "restore_under" | "set_pointer" | "set_pointer@068c" | "set_pointer@0698" | "set_pointer@06b9"
        | "copy_screen" | "copy_screen@34b0" | "copy_screen@34c8" | "copy_panel" | "copy_panel@34cd"
        | "copy_panel@34e6" => {
            let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
            for r in [EAX, EBX, ECX, EDX, ESI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            // Random words on the stack the pieces pop.
            for i in 0..8u32 {
                m.cpu.mem.ww(lin(STACK, 0xFF00 + 2 * i), rng.next());
            }
            let back = m.cpu.mem.rw(lin(ds, video::BACK_BUFFER as u32));
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            m.cpu.mem.ww(lin(ds, 0x09e4), if target == 0xA000 { back } else { 0xA000 });
            m.cpu.s[ES] = target;
            // The pointer: header at CS:0106, position, saved background.
            let hdr = [rng.below(4) as u8, 1 + rng.below(40) as u8];
            m.cpu.mem.wb(lin(seg, 0x106), hdr[0]);
            m.cpu.mem.wb(lin(seg, 0x107), hdr[1]);
            m.cpu.mem.ww(lin(seg, 0x108), 1 + rng.below(40));
            m.cpu.mem.ww(lin(seg, 0x10a), rng.below(40).wrapping_sub(20));
            m.cpu.mem.ww(lin(seg, 0x10c), rng.below(40).wrapping_sub(20));
            m.cpu.mem.ww(lin(ds, video::POINTER_X as u32), rng.coord(video::WIDTH));
            m.cpu.mem.ww(lin(ds, video::POINTER_Y as u32), rng.coord(200));
            m.cpu.mem.ww(lin(seg, 0x124), 4 * (1 + rng.below(12)));
            m.cpu.mem.wb(lin(seg, 0x126), 1 + rng.below(40) as u8);
            let saved = if rng.below(4) == 0 { 0xffff } else { rng.below(64000 - 48 * 320) };
            m.cpu.mem.ww(lin(STACK, 0), saved);
            if name == "set_pointer" && rng.below(3) == 0 {
                let cur = m.cpu.mem.rw(lin(ds, video::POINTER_BITMAP as u32));
                m.cpu.r[EBX] = (m.cpu.r[EBX] & 0xFFFF_0000) | cur as u32;
            }
            true
        }
        "colour" | "colour_entry" | "colours_tick" | "upload_colours" | "nothing" | "set_palette" | "set_palette@081d"
        | "load_high_colours" | "load_high_colours@1432" | "load_colours" | "load_colours@34f0" => {
            for r in [EAX, EBX, ECX, EDX, ESI, EDI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            let set = |m: &mut Machine, r: usize, v: u16| m.cpu.r[r] = (m.cpu.r[r] & 0xFFFF_0000) | v as u32;
            for i in 0..0x20u32 {
                m.cpu.mem.ww(lin(STACK, 0xFF00 + 2 * i), rng.next());
            }
            // Disturb the allocator: some slots get other states, colours and entries.
            if rng.below(2) == 0 {
                for _ in 0..rng.below(40) {
                    let n = rng.below(0xff) as u32;
                    let a = lin(ds, video::SLOTS as u32 + 4 * n);
                    m.cpu.mem.wb(a, [1, 2, 2, 3, 4, 5, 6][rng.below(7) as usize]);
                    m.cpu.mem.wb(a + 1, 0x80 | rng.next() as u8);
                    let key = rng.next() & 0xfff;
                    m.cpu.mem.ww(a + 2, key);
                    if rng.below(2) == 0 {
                        m.cpu.mem.wb(lin(ds, video::COLOUR_MAP as u32 + key as u32), n as u8);
                    }
                }
            }
            match name {
                "colour" => set(m, EBX, rng.next()),
                "colour_entry" => set(m, EDI, rng.below(0x100)),
                "colours_tick" => m.cpu.r[EAX] = (m.cpu.r[EAX] & !0xFF) | (rng.below(4) == 0) as u32,
                "set_palette" => m.cpu.r[EAX] = (m.cpu.r[EAX] & !0xFF) | rng.below(4) as u32,
                "set_palette@081d" => m.cpu.mem.ww(lin(STACK, 0xFF00), rng.below(4)),
                _ => {}
            }
            true
        }
        "textured_triangle" | "textured_quad" => {
            // A cached bitmap (a valid header), so the fill runs to the end.
            let cache = m.cpu.mem.rw(lin(ds, video::CACHE_SEGMENT as u32));
            let cached: Vec<u16> = (0..0x400u16)
                .filter(|&i| {
                    let slot = m.cpu.mem.rw(lin(cache, 0xf000 + 2 * i as u32));
                    slot != 0xffff && m.cpu.mem.rb(lin(cache, slot as u32)) <= 3 && m.cpu.mem.rb(lin(cache, slot as u32 + 1)) > 0
                })
                .collect();
            if cached.is_empty() {
                return false;
            }
            let id = cached[rng.below(cached.len() as u16) as usize];
            let (w, h) = (video::WIDTH, video::VIEW_HEIGHT);
            let small = rng.below(3) == 0;
            let c = |r: &mut Rng, s: u16| if small { r.below(s) } else if r.below(6) == 0 { r.coord(s) } else { r.below(s) };
            let v = [c(rng, w), c(rng, h), c(rng, w), c(rng, h), c(rng, w), c(rng, h)];
            for (r, val) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
                m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
            }
            m.cpu.mem.ww(lin(ds, video::QUAD_X3 as u32), c(rng, w));
            m.cpu.mem.ww(lin(ds, video::QUAD_Y3 as u32), c(rng, h));
            let turn = if name == "textured_quad" { rng.below(8) << 12 } else { rng.next() & 0xf000 };
            m.cpu.r[EDI] = (rng.dword() & 0xFFFF_0000) | (id | turn) as u32;
            m.cpu.mem.ww(lin(ds, video::TARGET as u32), target);
            true
        }
        _ if name.starts_with("flip") || name.starts_with("init") => {
            for r in [EAX, EBX, ECX, EDX, ESI, EDI, EBP] {
                m.cpu.r[r] = rng.dword();
            }
            for i in 0..0x20u32 {
                m.cpu.mem.ww(lin(STACK, 0xFF00 + 2 * i), rng.next());
            }
            let f = m.cpu.mem.rb(lin(STACK, 0xfa06 as u32));
            m.cpu.mem.wb(lin(STACK, 0xfa06), (f & !2) | (rng.below(2) as u8) << 1);
            if name == "init@05d1" {
                // The cache segment DOS gave, or a failure.
                let cache = m.cpu.mem.rw(lin(ds, video::CACHE_SEGMENT as u32));
                m.cpu.r[EAX] = (m.cpu.r[EAX] & 0xFFFF_0000) | cache as u32;
                m.cpu.flags = (m.cpu.flags & !1) | (rng.below(8) == 0) as u32;
            }
            true
        }
        _ => false,
    }
}

#[test]
fn mcga3_ports_match_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let cases: usize = std::env::var("MCGA3_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(3000);
    let only = std::env::var("MCGA3_ONLY").ok();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut shadow = Shadow::default();
    let mut failures = Vec::new();
    for &(off, slot, name, port) in EXPORTS {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let (mut mismatches, mut declined, mut ran) = (0, 0, 0);
        for _ in 0..cases {
            for r in [EDI] {
                m.cpu.r[r] = rng.dword();
            }
            m.cpu.r[ESP] = if name.starts_with("fill_") { 0x8000 } else { 0xFF00 };
            if name.contains('@') {
                // A piece after a call out: the stack holds what the code before it pushed.
                m.cpu.r[ESP] = 0xFF04;
            }
            m.cpu.s[DS] = ds;
            m.cpu.s[SS] = STACK;
            m.cpu.flags &= !frontier_dos::cpu::DF;
            if !setup(&mut m, &mut rng, name, ds) {
                break;
            }
            if name == "text_width" {
                m.cpu.s[DS] = 0x8000;
            }
            m.enter(seg, off);
            let entry = state(&m.cpu);
            let out = match run_port(&m.cpu, &m.dev.vga, m.dev.ips, &mut shadow, port) {
                Ok(o) => o,
                Err(_) => {
                    declined += 1;
                    continue;
                }
            };
            ran += 1;
            let mut orig_writes = std::collections::BTreeMap::new();
            let until = (out.cpu.cs, out.cpu.ip, out.cpu.r[ESP] as u16);
            let diff = match record_original(&mut m.cpu, &mut m.dev, 50_000_000, until) {
                Some(orig) => {
                    let d = compare(&out, &shadow, &orig, &m.cpu.mem, &m.dev.vga);
                    orig_writes = orig.writes;
                    d
                }
                None => Some(format!("the original did not return; the port claims {} steps", out.steps)),
            };
            if let Some(d) = diff {
                if std::env::var_os("MCGA3_DIFF").is_some() {
                    let staged: std::collections::BTreeMap<u32, u8> = shadow.writes().collect();
                    let mut diffs = Vec::new();
                    for (&a, &(before, after)) in &orig_writes {
                        let got = staged.get(&a).copied().unwrap_or(before);
                        if got != after { diffs.push((a, after, got)); }
                    }
                    for (&a, &v) in &staged {
                        if !orig_writes.contains_key(&a) && v != m.cpu.mem.rb(a) && !(a >= 0x90000 && a < 0x90000 + 0xff00) { diffs.push((a, m.cpu.mem.rb(a), v)); }
                    }
                    let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
                    let cs = |o: u32| m.cpu.mem.rw(lin(seg, o));
                    eprintln!("DIFF {name}: {} bytes {:x?} | cs 357a {:x} 357c {:x} 357f {:x} 3581 {:x} 353c {:x} 353e {:x} 1fb1 {:x}", diffs.len(), &diffs[..diffs.len().min(12)], cs(0x357a), m.cpu.mem.rb(lin(seg, 0x357c)), cs(0x357f), cs(0x3581), cs(0x353c), cs(0x353e), m.cpu.mem.rb(lin(ds, 0x1fb1)));
                }
                if mismatches == 0 && std::env::var_os("MCGA3_DEBUG").is_some() {
                    let mut ws: Vec<(u32, u8)> = shadow.writes().filter(|w| w.0 >= 0xA0000 && w.0 < 0xB0000 || w.0 >= 0xE0000).collect();
                    ws.sort();
                    eprintln!("port frame writes: {} first {:x?}", ws.len(), &ws[..ws.len().min(8)]);
                    let o: Vec<_> = orig_writes.iter().filter(|w| *w.0 >= 0xA0000 && *w.0 < 0xB0000 || *w.0 >= 0xE0000).take(8).collect();
                    eprintln!("orig frame writes: {} first {:x?}", orig_writes.iter().filter(|w| *w.0 >= 0xA0000 && *w.0 < 0xB0000 || *w.0 >= 0xE0000).count(), o);
                }
                if mismatches < 3 {
                    failures.push(format!("{name} (EL2MCGA3:{off:04x}, slot {slot:04x}): input {entry:x?}\n  {d}"));
                }
                mismatches += 1;
            }
        }
        eprintln!("{name}: {ran} compared, {declined} declined, {mismatches} mismatches");
        if mismatches > 0 {
            failures.push(format!("  {name}: {mismatches}/{ran} mismatches"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Compare the port of the entry the CPU is at with the original, which then carries on.
fn compare_here(m: &mut Machine, shadow: &mut Shadow, name: &str) -> Result<(u16, u16, u16), String> {
    let port = EXPORTS.iter().find(|e| e.2 == name).unwrap().3;
    let entry = state(&m.cpu);
    let out = run_port(&m.cpu, &m.dev.vga, m.dev.ips, shadow, port).map_err(|d| format!("{name}: declined {d:?}"))?;
    let until = (out.cpu.cs, out.cpu.ip, out.cpu.r[ESP] as u16);
    let rec = record_original(&mut m.cpu, &mut m.dev, 50_000_000, until);
    match rec {
        Some(orig) => match compare(&out, shadow, &orig, &m.cpu.mem, &m.dev.vga) {
            None => Ok(until),
            Some(d) => {
                let h: Vec<u8> = (0..8).map(|i| m.cpu.mem.rb(lin(entry.es, entry.r[ESI] as u16 as u32 + i))).collect();
                Err(format!("{name}: input {entry:x?} header {h:02x?}\n  {d}"))
            }
        },
        None => Err(format!("{name}: the original did not reach the port's end {until:x?}")),
    }
}

/// Run the machine (DOS and EMS services included) until `CS:IP` is `seg:off`.
fn run_to(m: &mut Machine, seg: u16, off: u16) {
    for _ in 0..1_000_000 {
        if m.cpu.s[frontier_dos::cpu::CS] == seg && m.cpu.ip as u16 == off {
            return;
        }
        m.run(1);
    }
    panic!("did not reach {seg:04x}:{off:04x}, at {:04x}:{:04x}", m.cpu.s[frontier_dos::cpu::CS], m.cpu.ip);
}

/// The bitmap path end to end: the blitter, the cache lookup, and on a miss the three pieces
/// between the `int 67h` calls, each compared with the original, the EMS driver run in between.
#[test]
fn sprite_chain_matches_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let cases: usize = std::env::var("MCGA3_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(1500);
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut shadow = Shadow::default();
    // Bitmaps with a plausible directory entry (EMS page below 30h).
    let ids: Vec<u16> = (0..0x400u16)
        .filter(|&i| {
            let page = m.cpu.mem.rw(lin(ds, 0x5813 + 4 * i as u32 + 2));
            let off = m.cpu.mem.rw(lin(ds, 0x5813 + 4 * i as u32));
            page < 0x30 && off < 0x4000
        })
        .collect();
    assert!(ids.len() > 10, "no bitmap directory");
    let (mut misses, mut failures) = (0, Vec::new());
    for _ in 0..cases {
        let id = ids[rng.below(ids.len() as u16) as usize];
        // Skip cached entries that are not bitmaps (flags above 3): their sizes wrap the stack.
        let cache = m.cpu.mem.rw(lin(ds, video::CACHE_SEGMENT as u32));
        let slot = m.cpu.mem.rw(lin(cache, 0xf000 + 2 * id as u32));
        if slot != 0xffff && m.cpu.mem.rb(lin(cache, slot as u32)) > 3 {
            continue;
        }
        // A target descriptor in the stack segment, as the mouse pointer's at SS:0fd59h.
        let d = 0xF000u16;
        let target = if rng.below(2) == 0 { 0xA000 } else { m.cpu.mem.rw(lin(ds, video::BACK_BUFFER as u32)) };
        let fields = [0, target, rng.below(40), rng.below(40), 240 + rng.below(80), 120 + rng.below(80), rng.below(8)];
        for (i, v) in fields.into_iter().enumerate() {
            m.cpu.mem.ww(lin(STACK, (d + 2 * i as u16) as u32), v);
        }
        for r in [EAX, ESI, EBP] {
            m.cpu.r[r] = rng.dword();
        }
        let set = |m: &mut Machine, r: usize, v: u16| m.cpu.r[r] = (m.cpu.r[r] & 0xFFFF_0000) | v as u32;
        set(&mut m, EBX, id);
        set(&mut m, ECX, rng.coord(video::WIDTH));
        set(&mut m, EDX, rng.coord(200));
        set(&mut m, EDI, if rng.below(16) == 0 { 0 } else { d });
        m.cpu.r[ESP] = 0xE000;
        m.cpu.s[DS] = ds;
        m.cpu.s[SS] = STACK;
        m.cpu.flags &= !(frontier_dos::cpu::DF | frontier_dos::cpu::IF);
        m.enter(seg, 0x02d3);
        let mut step = |m: &mut Machine, name: &str| compare_here(m, &mut shadow, name);
        let r = (|| -> Result<(), String> {
            let end = step(&mut m, "blit")?;
            if (end.0, end.1) == (0xF000, 0x0f00) {
                // Returned to the host: the bitmap was cached.
                return Ok(());
            }
            misses += 1;
            for (resume, name) in [(0x01b4, "cache@01b4"), (0x01be, "cache@01be"), (0x0244, "cache@0244")] {
                run_to(&mut m, seg, resume);
                step(&mut m, name)?;
            }
            run_to(&mut m, seg, 0x02dc);
            step(&mut m, "blit@02dc")?;
            Ok(())
        })();
        if let Err(e) = r {
            if failures.len() < 4 {
                failures.push(e);
            }
        }
    }
    eprintln!("sprite chain: {cases} blits, {misses} cache misses, {} failures", failures.len());
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The textured fills with bitmaps not yet cached: the fill up to the cache's first `int 67h`,
/// the original's cache code run on the machine, then the fill's code after the lookup.
#[test]
fn textured_miss_chain_matches_original() {
    let Some(mut m) = booted() else { return };
    let seg = m.image_segment("EL2MCGA3.OVL").unwrap();
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    let cases: usize = std::env::var("MCGA3_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(200);
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut shadow = Shadow::default();
    let cache = m.cpu.mem.rw(lin(ds, video::CACHE_SEGMENT as u32));
    let ids: Vec<u16> = (0..0x400u16)
        .filter(|&i| {
            let page = m.cpu.mem.rw(lin(ds, 0x5813 + 4 * i as u32 + 2));
            page < 0x30 && m.cpu.mem.rw(lin(cache, 0xf000 + 2 * i as u32)) == 0xffff
        })
        .collect();
    let (mut misses, mut failures) = (0, Vec::new());
    for case in 0..cases {
        if ids.is_empty() {
            break;
        }
        let id = ids[rng.below(ids.len() as u16) as usize];
        let quad = case % 2 == 1;
        let (entry, resume, name) = if quad { (0x44d1, 0x45b3, "textured_quad") } else { (0x3635, 0x36c0, "textured_triangle") };
        let v = [rng.below(320), rng.below(168), rng.below(320), rng.below(168), rng.below(320), rng.below(168)];
        for (r, val) in [EAX, EBX, ECX, EDX, ESI, EBP].into_iter().zip(v) {
            m.cpu.r[r] = (rng.dword() & 0xFFFF_0000) | val as u32;
        }
        m.cpu.mem.ww(lin(ds, video::QUAD_X3 as u32), rng.below(320));
        m.cpu.mem.ww(lin(ds, video::QUAD_Y3 as u32), rng.below(168));
        m.cpu.r[EDI] = (rng.dword() & 0xFFFF_0000) | (id | rng.below(8) << 12) as u32;
        m.cpu.r[ESP] = 0xFF00;
        m.cpu.s[DS] = ds;
        m.cpu.s[SS] = STACK;
        // Interrupts off: the game's timer handler must not run on this scratch stack.
        m.cpu.flags &= !(frontier_dos::cpu::DF | frontier_dos::cpu::IF);
        m.enter(seg, entry);
        let r = (|| -> Result<(), String> {
            let end = compare_here(&mut m, &mut shadow, name)?;
            let ems = (m.cpu.mem.rw(0x67 * 4 + 2), m.cpu.mem.rw(0x67 * 4));
            if (end.0, end.1) != ems {
                // Drawn from the cache, or (a directory entry that is no bitmap) the texel
                // buffer overran the return address: nothing more to compare.
                return Ok(());
            }
            misses += 1;
            run_to(&mut m, seg, resume);
            compare_here(&mut m, &mut shadow, &format!("{name}@{resume:04x}"))?;
            Ok(())
        })();
        if let Err(e) = r {
            if failures.len() < 4 {
                failures.push(e);
            }
        }
    }
    eprintln!("textured miss chain: {cases} fills, {misses} cache misses, {} failures", failures.len());
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
