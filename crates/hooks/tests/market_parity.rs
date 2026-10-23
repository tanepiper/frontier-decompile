//! Parity: the bulletin board's advert generator (`EL2M07:0b03`, `frontier_game::market`) against
//! the original, in the game's own state in the cockpit at Ross 154, with modules 2, 6, 7 and 19
//! resident. Skips when game-data/ is absent.

use std::path::Path;

use frontier_dos::cpu::{CS, DS, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::{Machine, Vfs};
use frontier_game::math3::DECLINE;
use frontier_hooks::{compare, record_original_until, run_port};

struct X(u64);
impl X {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as u32
    }
}

/// Play into the cockpit, then load the modules the generator calls through their own module
/// thunks (handler `+04`), with the game's registers saved around each call.
fn in_cockpit() -> Option<(Machine, u16)> {
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
    let second = m.dev.ips as u64;
    assert_eq!(m.run(20 * second), frontier_dos::Stop::Budget);
    m.key(0x39);
    m.key(0xB9);
    assert_eq!(m.run(4 * second), frontier_dos::Stop::Budget);
    m.key(0x02);
    m.key(0x82);
    assert_eq!(m.run(9 * second), frontier_dos::Stop::Budget);
    let ds = m.images().iter().find(|i| i.name == "FRONTIER.EXE").unwrap().seg + 0x1615;
    for (slot, name) in [(19u16, "EL2M19.OVL"), (2, "EL2M02.OVL"), (6, "EL2M06.OVL"), (7, "EL2M07.OVL")] {
        if m.image_segment(name).is_some() {
            continue;
        }
        let (r, sg, ip, fl) = (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags);
        let si = slot * 8;
        let hdr_off = m.cpu.mem.rw(lin(ds, 0x0396 + si as u32));
        let hdr_seg = m.cpu.mem.rw(lin(ds, 0x0398 + si as u32));
        let off = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 4));
        let seg = m.cpu.mem.rw(lin(hdr_seg, hdr_off as u32 + 6));
        m.cpu.s[DS] = ds;
        m.cpu.r[ESI] = si as u32;
        m.cpu.r[0] = 0;
        m.call(seg, off, 50_000_000).expect("load module");
        (m.cpu.r, m.cpu.s, m.cpu.ip, m.cpu.flags) = (r, sg, ip, fl);
    }
    for name in ["EL2M19.OVL", "EL2M02.OVL", "EL2M06.OVL", "EL2M07.OVL"] {
        if m.image_segment(name).is_none() {
            eprintln!("skipping: {name} not resident after loading");
            return None;
        }
    }
    Some((m, ds))
}

/// `EL2M07:0b03` with random generator states, room (`DS:43a3` − `43a5`) across the kinds'
/// thresholds, allegiances 0–2 in `DS:2243`, a partly filled advert table, and the current system
/// or a random one in `DS:208b`.
#[test]
fn adverts_match_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let port = frontier_game::market::BOARD_M07[0].3;
    let here = m.cpu.mem.rd(lin(ds, 0x208b));
    let mut x = X(0xADAD_0B03_5EED_0001);
    let (mut compared, mut declined, mut stored, mut failures) = (0, 0, 0, Vec::new());
    let mut kinds = std::collections::BTreeMap::<u16, u32>::new();
    for i in 0..3000 {
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.ww(lin(ds, 0x43a3), (x.next() % 0x700) as u16);
        m.cpu.mem.ww(lin(ds, 0x43a5), (x.next() % 0x200) as u16);
        m.cpu.mem.ww(lin(ds, 0x2243), (x.next() % 3) as u16);
        m.cpu.mem.wd(lin(ds, 0x208b), if i % 2 == 0 { here } else { x.next() });
        m.cpu.mem.wd(lin(ds, 0x07d4), x.next());
        // Slots 0..k used, the rest free; sometimes all full.
        let used = if i % 10 == 0 { 25 } else { x.next() % 25 };
        for k in 0..26 {
            m.cpu.mem.wd(lin(ds, 0x43a7 + 0x18 * k), if k < used { 1 + x.next() % 0xffff } else { 0 });
        }
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.flags &= !0x0400;
        m.cpu.push16(0x0afc);
        m.cpu.s[CS] = m07;
        m.cpu.ip = 0x0b03;
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        if let Some(k) = (0..25).find(|&k| out.writes.contains_key(&lin(ds, 0x43a7 + 0x18 * k))) {
            let a = lin(ds, 0x43a7 + 0x18 * k);
            *kinds.entry(out.writes[&a] as u16 | (out.writes[&(a + 1)] as u16) << 8).or_default() += 1;
            stored += 1;
        }
        let diff = match record_original_until(&mut m.cpu, 5_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 4 {
                failures.push(format!("case {i}: {d}"));
            }
        }
    }
    eprintln!("advert: {compared} compared ({stored} stored an advert), {declined} declined, {} mismatches", failures.len());
    assert!(compared > 2000, "too few cases compared");
    eprintln!("advert kinds stored: {kinds:x?}");
    assert!(stored > 200, "too few adverts stored");
    assert!(kinds.len() == 4, "not every kind of advert was stored");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `EL2M07:101a` (a station's list entry, through `103c`) for list kinds 0, 1, 9 and 12, random
/// allegiances, `DS:2239` values, list lengths and generator states.
#[test]
fn station_lists_match_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let port = frontier_game::market::BOARD_M07.iter().find(|p| p.2 == "station_list").unwrap().3;
    let mut x = X(0x101A_103C_5EED_0002);
    let (mut compared, mut added, mut failures) = (0, 0, Vec::new());
    for i in 0..4000 {
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.ww(lin(ds, 0x07d0), [0u16, 1, 9, 0x0c][i % 4]);
        m.cpu.mem.ww(lin(ds, 0x2243), (x.next() % 3) as u16);
        m.cpu.mem.ww(lin(ds, 0x2239), (x.next() % 8) as u16);
        m.cpu.mem.wd(lin(ds, 0x07cc), x.next());
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        // A station record from DS:224d (1DAh bytes each) and its list at +1A0h.
        let si = 0x224d + 0x1da * (x.next() % 4);
        let count = x.next() % 16;
        m.cpu.mem.ww(lin(ds, si + 0x1d8), count as u16);
        m.cpu.r[ESI] = si;
        m.cpu.r[7] = si + 0x1a0 + 4 * count.min(13);
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.push16(0x0fd9);
        m.cpu.s[CS] = m07;
        m.cpu.ip = 0x101a;
        let out = run_port(&m.cpu, port);
        compared += 1;
        if out.writes.contains_key(&lin(ds, si + 0x1d8)) {
            added += 1;
        }
        let diff = match record_original_until(&mut m.cpu, 100_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 4 {
                failures.push(format!("case {i}: {d}"));
            }
        }
    }
    eprintln!("station_list: {compared} compared ({added} added an entry), {} mismatches", failures.len());
    assert!(added > 1000, "too few entries added");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `EL2M07:0923` (a station event, through the jump table at `03a1` and the writer `1506`) with
/// random generator states, station flags, event counts, commodity flags and stocks, the four
/// dwords at `DS:4603`, allegiances, and the current system or a random one.
#[test]
fn station_events_match_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let port = frontier_game::market::BOARD_M07.iter().find(|p| p.2 == "event").unwrap().3;
    let here = m.cpu.mem.rd(lin(ds, 0x208b));
    let mut x = X(0x0923_03A1_5EED_0003);
    let (mut compared, mut declined, mut failures) = (0, 0, Vec::new());
    let mut kinds = std::collections::BTreeMap::<u16, u32>::new();
    for i in 0..6000 {
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.ww(lin(ds, 0x2243), (x.next() % 3) as u16);
        m.cpu.mem.ww(lin(ds, 0x223b), (x.next() % 4) as u16);
        m.cpu.mem.wd(lin(ds, 0x208b), if i % 2 == 0 { here } else { x.next() });
        for a in [0x07d4u32, 0x07d8, 0x07cc, 0x07c4, 0x07c8, 0x07e4] {
            m.cpu.mem.wd(lin(ds, a), x.next());
        }
        for k in 0..4 {
            m.cpu.mem.wd(lin(ds, 0x4603 + 4 * k), if x.next() % 3 == 0 { x.next() } else { 0 });
        }
        for k in 0..31 {
            let b = x.next() as u8;
            m.cpu.mem.wb(lin(ds, 0x2219 + k), if i % 3 == 0 { b & 0x7f } else { b });
        }
        let si = 0x224d + 0x1da * (x.next() % 4);
        for k in 0..31 {
            m.cpu.mem.ww(lin(ds, si + 4 * k), if x.next() % 4 == 0 { 0 } else { x.next() as u16 });
            m.cpu.mem.ww(lin(ds, si + 4 * k + 2), x.next() as u16);
        }
        m.cpu.mem.wb(lin(ds, si + 0x7d), (x.next() & 0x1f) as u8);
        let count = x.next() % 20;
        m.cpu.mem.ww(lin(ds, si + 0x19e), count as u16);
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        m.cpu.r[ESI] = si;
        m.cpu.r[7] = si + 0x7e + 0x10 * count.min(17);
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.push16(0x0a2d);
        m.cpu.s[CS] = m07;
        m.cpu.ip = 0x0923;
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        let di = m.cpu.r[7] & 0xffff;
        if let (Some(&lo), Some(&hi)) = (out.writes.get(&lin(ds, di)), out.writes.get(&lin(ds, di + 1))) {
            *kinds.entry(lo as u16 | (hi as u16) << 8).or_default() += 1;
        }
        let diff = match record_original_until(&mut m.cpu, 5_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 6 {
                failures.push(format!("case {i}: {d}"));
            }
        }
    }
    eprintln!("event: {compared} compared, {declined} declined, {} mismatches", failures.len());
    eprintln!("event strings written: {kinds:x?}");
    assert!(compared > 4000, "too few cases compared");
    assert!(kinds.len() >= 14, "too few kinds of event written");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `EL2M07:0949`, the whole market builder, for the current system and random ones, with random
/// generator states and 0 to 4 stations (the game's own record and copies of its object index).
#[test]
fn market_builder_matches_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let port = frontier_game::market::BOARD_M07.iter().find(|p| p.2 == "builder").unwrap().3;
    let here = m.cpu.mem.rd(lin(ds, 0x208b));
    let stations = m.cpu.mem.rw(lin(ds, 0x2247));
    let mut x = X(0x0949_0949_5EED_0004);
    let (mut compared, mut declined, mut failures) = (0, 0, Vec::new());
    for i in 0..300 {
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.wd(lin(ds, 0x07c0), if i % 3 == 0 { here } else { x.next() });
        // 0 to 4 stations; the extra records point at the real station's object.
        let count = if i % 10 == 9 { 0 } else { 1 + (x.next() % 4) as u16 };
        m.cpu.mem.ww(lin(ds, 0x2247), count);
        let obj = m.cpu.mem.rb(lin(ds, 0x224d + 0x7c));
        for k in 1..4 {
            m.cpu.mem.wb(lin(ds, 0x224d + 0x1da * k + 0x7c), obj);
        }
        for k in 0..4 {
            m.cpu.mem.wd(lin(ds, 0x4603 + 4 * k), if x.next() % 3 == 0 { x.next() } else { 0 });
        }
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.push16(0x1580);
        m.cpu.s[CS] = m07;
        m.cpu.ip = 0x0949;
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        let diff = match record_original_until(&mut m.cpu, 50_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 4 {
                failures.push(format!("case {i}: {d}"));
            }
        }
    }
    eprintln!("builder: {compared} compared ({stations} stations), {declined} declined, {} mismatches", failures.len());
    assert!(stations > 0, "no stations in the cockpit state");
    assert!(compared > 250, "too few cases compared");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Module 7's handler `+0c` (`EL2M07:1551`): `AX` = 2 (station count, the shift of `DS:4603`,
/// the whole builder), `AX` = 0Fh (the daily update) and other values, with random generator
/// states, advert tables and dates, against the original. The station count and the daily update
/// are also called on their own.
#[test]
fn arrival_and_daily_update_match_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let find = |name: &str| frontier_game::market::BOARD_M07.iter().find(|p| p.2 == name).unwrap().3;
    let mut x = X(0x1551_0E1A_5EED_0005);
    let (mut compared, mut declined, mut failures) = (0, 0, Vec::new());
    let date = m.cpu.mem.rd(lin(ds, 0x1fc5));
    let mut per: std::collections::BTreeMap<&str, u32> = Default::default();
    for i in 0..400 {
        let (name, entry, far, ax) = match i % 4 {
            0 => ("handler_0c", 0x1551u16, true, 2u32),
            1 => ("handler_0c", 0x1551, true, [0x0f, 0x0f, 0x0f, 7][(i / 4) % 4]),
            2 => ("daily", 0x0e1a, false, 0),
            _ => ("station_count", 0x0818, false, 0),
        };
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fc5), date.wrapping_add(x.next() % 64));
        m.cpu.mem.wb(lin(ds, 0x2075), if i % 16 == 5 { 8 } else { 0 });
        for k in 0..4 {
            m.cpu.mem.wd(lin(ds, 0x4603 + 4 * k), if x.next() % 3 == 0 { x.next() } else { 0 });
        }
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        m.cpu.r[0] = (m.cpu.r[0] & 0xffff_0000) | ax;
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        if far {
            m.cpu.push16(0x1234);
        }
        m.cpu.push16(0x0abc);
        m.cpu.s[CS] = m07;
        m.cpu.ip = entry as u32;
        let out = run_port(&m.cpu, find(name));
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        *per.entry(name).or_default() += 1;
        let diff = match record_original_until(&mut m.cpu, 100_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 6 {
                failures.push(format!("case {i} {name} AX {ax:x}: {d}"));
            }
        }
    }
    eprintln!("handler_0c/daily/station_count: {compared} compared {per:?}, {declined} declined, {} mismatches", failures.len());
    assert!(compared > 350, "too few cases compared");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Module 7's handler `+20` (`EL2M07:15a4`, the docking bays) for every `BX` but 6, on the real
/// station object (`[DS:07e0]`) with random bays, bay counts, states, flags and ships.
#[test]
fn docking_bays_match_original() {
    let Some((mut m, ds)) = in_cockpit() else { return };
    let m07 = m.image_segment("EL2M07.OVL").unwrap();
    let port = frontier_game::market::BOARD_M07.iter().find(|p| p.2 == "handler_20").unwrap().3;
    // The station object: [56f7] + the word at DS:06d6 + 2·index, index = its record's byte +7Ch.
    let idx = m.cpu.mem.rb(lin(ds, 0x224d + 0x7c)) as u32;
    let (blk_off, blk_seg) = (m.cpu.mem.rw(lin(ds, 0x56f7)), m.cpu.mem.rw(lin(ds, 0x56f9)));
    let obj = blk_off.wrapping_add(m.cpu.mem.rw(lin(ds, 0x06d6 + 2 * idx)));
    let mut x = X(0x15A4_D0C0_5EED_0006);
    let (mut compared, mut declined, mut failures) = (0, 0, Vec::new());
    let mut per = [0u32; 10];
    for i in 0..5000 {
        let bx = [0u32, 1, 1, 1, 2, 3, 4, 5, 5, 7, 8, 9, 9, 0x10][i % 14];
        m.cpu.mem.wd(lin(ds, 0x1fd9), x.next());
        m.cpu.mem.wd(lin(ds, 0x1fdd), x.next());
        m.cpu.mem.ww(lin(ds, 0x07e0), obj);
        m.cpu.mem.ww(lin(ds, 0x07e2), blk_seg);
        let o = |k: u32| lin(blk_seg, obj as u32 + k);
        let bays = x.next() % 4;
        m.cpu.mem.ww(o(0xa4), bays as u16);
        for k in 0..4 {
            m.cpu.mem.wb(o(0xa6 + k), if x.next() % 2 == 0 { 0 } else { (x.next() % 8) as u8 });
        }
        m.cpu.mem.wb(o(0x9f), [0u8, 1, 3, 0xff][(x.next() % 4) as usize]);
        m.cpu.mem.wb(o(0xa0), (x.next() % 8) as u8);
        let f = m.cpu.mem.rb(o(0x118));
        m.cpu.mem.wb(o(0x118), (f & !0x60) | (x.next() as u8 & 0x60));
        for k in 0..9 {
            m.cpu.mem.ww(o(2 * k), x.next() as u16);
        }
        m.cpu.mem.ww(lin(ds, 0x07d8), (x.next() % 4) as u16);
        m.cpu.mem.wb(lin(ds, 0x2053), (x.next() % 8) as u8);
        m.cpu.mem.ww(lin(ds, 0x07d0), [0u16, 1, 9][(x.next() % 3) as usize]);
        for r in 0..8 {
            m.cpu.r[r] = x.next();
        }
        m.cpu.r[0] = (m.cpu.r[0] & 0xffff_0000) | if bx == 8 { [0, 1, 9][(x.next() % 3) as usize] } else { x.next() % 8 };
        m.cpu.r[3] = (m.cpu.r[3] & 0xffff_0000) | bx;
        m.cpu.r[ESP] = 0xFFF0;
        m.cpu.s[SS] = 0xA000;
        m.cpu.s[DS] = ds;
        m.cpu.push16(0x1234);
        m.cpu.push16(0x0abc);
        m.cpu.s[CS] = m07;
        m.cpu.ip = 0x15a4;
        let out = run_port(&m.cpu, port);
        if out.steps == DECLINE {
            declined += 1;
            continue;
        }
        compared += 1;
        per[(bx as usize).min(9)] += 1;
        let diff = match record_original_until(&mut m.cpu, 1_000_000, out.cpu.cs, out.cpu.ip, out.cpu.r[4] as u16) {
            Some(orig) => compare(&out, &orig, &m.cpu.mem),
            None => Some("the original did not return".into()),
        };
        if let Some(d) = diff {
            if failures.len() < 6 {
                failures.push(format!("case {i} BX {bx}: {d}"));
            }
        }
    }
    eprintln!("handler_20: {compared} compared {per:?}, {declined} declined, {} mismatches", failures.len());
    assert!(compared > 4500, "too few cases compared");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
