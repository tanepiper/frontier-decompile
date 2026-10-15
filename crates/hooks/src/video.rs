//! The native video driver ([`frontier_game::video`]) in the runtime: hooks at the exports of
//! `EL2MCGA3.OVL`, a recorder for the original's memory writes and VGA port accesses, and
//! shadow verification (docs/VIDEO.md).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use frontier_dos::cpu::{Cpu, CS};
use frontier_dos::devices::{Devices, Vga};
use frontier_dos::mem::Memory;
use frontier_dos::Machine;
use frontier_game::math3;
use frontier_game::video::{self, Ctx, Decline, Port, EXPORTS, IF};

use crate::{apply, far_call_depth, state};

pub const OVERLAY: &str = "EL2MCGA3.OVL";

/// A VGA port access: (instructions executed in the call before it, port, value).
pub type Access = (u64, u16, u8);

/// Memory writes staged over runtime memory: a 1 MB shadow with a bitmap of the bytes written,
/// reused from call to call (only the 64-byte blocks written are cleared).
pub struct Shadow {
    data: Vec<u8>,
    written: Vec<u64>,
    /// Blocks with a byte written, in first-write order.
    blocks: Vec<u32>,
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow { data: vec![0; 0x10_0000], written: vec![0; 0x10_0000 / 64], blocks: Vec::new() }
    }
}

impl Shadow {
    fn clear(&mut self) {
        for &b in &self.blocks {
            self.written[b as usize] = 0;
        }
        self.blocks.clear();
    }

    /// The bytes written: (address, value).
    pub fn writes(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.blocks.iter().flat_map(move |&b| {
            let w = self.written[b as usize];
            (0..64u32).filter(move |i| w & (1 << i) != 0).map(move |i| {
                let a = b << 6 | i;
                (a, self.data[a as usize])
            })
        })
    }
}

struct Staged<'a> {
    mem: &'a Memory,
    shadow: &'a mut Shadow,
}

impl math3::Bus for Staged<'_> {
    fn read8(&mut self, a: u32) -> u8 {
        let a = a & 0xF_FFFF;
        if self.shadow.written[(a >> 6) as usize] & (1 << (a & 63)) != 0 {
            self.shadow.data[a as usize]
        } else {
            self.mem.rb(a)
        }
    }

    fn write8(&mut self, a: u32, v: u8) {
        let a = a & 0xF_FFFF;
        let w = &mut self.shadow.written[(a >> 6) as usize];
        if *w == 0 {
            self.shadow.blocks.push(a >> 6);
        }
        *w |= 1 << (a & 63);
        self.shadow.data[a as usize] = v;
    }
}

/// VGA ports seen through a copy of the VGA state, at the virtual time of each access.
pub struct StagedVga<'a> {
    base: &'a Vga,
    vga: Option<Vga>,
    start: u64,
    ips: f64,
    pub accesses: Vec<Access>,
}

impl<'a> StagedVga<'a> {
    pub fn new(base: &'a Vga, start: u64, ips: f64) -> Self {
        StagedVga { base, vga: None, start, ips, accesses: Vec::new() }
    }
    fn vga(&mut self) -> &mut Vga {
        self.vga.get_or_insert_with(|| self.base.clone())
    }
}

impl video::Io for StagedVga<'_> {
    fn inb(&mut self, steps: u64, port: u16) -> u8 {
        let now = (self.start + steps) as f64 / self.ips;
        let v = if (0x3C0..=0x3DF).contains(&port) { self.vga().inb(port, now) } else { 0xFF };
        self.accesses.push((steps, port, v));
        v
    }
    fn outb(&mut self, steps: u64, port: u16, v: u8) {
        if (0x3C0..=0x3DF).contains(&port) {
            self.vga().outb(port, v);
        }
        self.accesses.push((steps, port | 0x8000, v));
    }
}

/// What a port did to a call, staged.
pub struct Outcome {
    pub cpu: math3::Cpu,
    pub steps: u64,
    /// The VGA state after it, if it touched a port.
    pub vga: Option<Vga>,
    pub accesses: Vec<Access>,
}

/// Run `port` on the CPU's state (at the export's entry) without changing anything; writes go
/// to `shadow`.
pub fn run_port(cpu: &Cpu, vga: &Vga, ips: f64, shadow: &mut Shadow, port: Port) -> Result<Outcome, Decline> {
    shadow.clear();
    let mut bus = Staged { mem: &cpu.mem, shadow };
    let mut io = StagedVga::new(vga, cpu.icount, ips);
    let mut x = Ctx { c: state(cpu), bus: &mut bus, io: &mut io, steps: 0 };
    port(&mut x)?;
    let (c, steps) = (x.c, x.steps);
    Ok(Outcome { cpu: c, steps, vga: io.vga, accesses: io.accesses })
}

/// Apply a port's staged effect.
pub fn commit(cpu: &mut Cpu, dev: &mut Devices, shadow: &Shadow, out: Outcome) {
    for (a, v) in shadow.writes() {
        cpu.mem.wb(a, v);
    }
    apply(cpu, &out.cpu);
    cpu.flags = (cpu.flags & !IF) | (out.cpu.flags & IF);
    cpu.icount += out.steps;
    if let Some(v) = out.vga {
        dev.vga = v;
    }
}

/// Recorder of the original's port accesses, on the real devices at the right virtual time.
struct Recorder<'a> {
    dev: &'a mut Devices,
    steps: u64,
    accesses: Vec<Access>,
}

impl frontier_dos::cpu::Io for Recorder<'_> {
    fn inb(&mut self, port: u16) -> u8 {
        let v = self.dev.inb(port);
        self.accesses.push((self.steps, port, v));
        v
    }
    fn outb(&mut self, port: u16, v: u8) {
        self.dev.outb(port, v);
        self.accesses.push((self.steps, port | 0x8000, v));
    }
}

/// The original's effect on one call.
pub struct Original {
    pub cpu: math3::Cpu,
    pub steps: u64,
    /// Every byte it wrote: (value before, value after).
    pub writes: BTreeMap<u32, (u8, u8)>,
    pub accesses: Vec<Access>,
}

/// Run the original routine the CPU is at until the `RETF` that leaves it, on the real devices,
/// recording memory writes and port accesses. `None` if it faulted, called a host service or
/// ran past `budget`; memory is then restored.
pub fn record_original(cpu: &mut Cpu, dev: &mut Devices, budget: u64) -> Option<Original> {
    let start = cpu.icount;
    cpu.mem.write_log = Some(Vec::new());
    cpu.skip_breakpoint = true;
    let ips = dev.ips;
    let mut rec = Recorder { dev, steps: 0, accesses: Vec::new() };
    let mut depth = 0;
    let ok = loop {
        let d = far_call_depth(cpu);
        rec.steps = cpu.icount - start;
        rec.dev.now = cpu.icount as f64 / ips;
        match cpu.step(&mut rec) {
            Ok(()) => {}
            Err(frontier_dos::cpu::Event::Breakpoint(_)) => {
                cpu.skip_breakpoint = true;
                continue;
            }
            Err(e) => {
                if std::env::var_os("HOOKS_DEBUG").is_some() {
                    eprintln!("record_original: {e:?} at {:04x}:{:04x}", cpu.s[CS], cpu.ip);
                }
                break false;
            }
        }
        if cpu.icount - start > budget {
            break false;
        }
        depth += d;
        if depth < 0 {
            break true;
        }
    };
    let log = cpu.mem.write_log.take().unwrap_or_default();
    let mut writes = BTreeMap::new();
    for &(a, old) in &log {
        writes.entry(a).or_insert((old, 0));
    }
    for (&a, w) in writes.iter_mut() {
        w.1 = cpu.mem.rb(a);
    }
    if !ok {
        for (&a, &(old, _)) in &writes {
            cpu.mem.wb(a, old);
        }
        return None;
    }
    let accesses = rec.accesses;
    Some(Original { cpu: state(cpu), steps: cpu.icount - start, writes, accesses })
}

/// The first difference between a port's effect and the original's for the same call, given
/// memory as the original left it and the VGA state after the original (`vga`). Registers,
/// segments, `IF`, the return address, every byte written except the dead stack below the final
/// `SS:SP`, the port accesses in order and time, the VGA state and the instruction count.
pub fn compare(port: &Outcome, shadow: &Shadow, orig: &Original, mem: &Memory, vga: &Vga) -> Option<String> {
    let strip = |c: &math3::Cpu| math3::Cpu { flags: c.flags & IF, ..*c };
    if strip(&port.cpu) != strip(&orig.cpu) {
        return Some(format!("state: original {:x?}, port {:x?}", orig.cpu, port.cpu));
    }
    let (lo, hi) = (math3::lin(orig.cpu.ss, 0), math3::lin(orig.cpu.ss, orig.cpu.x(math3::ESP)));
    let dead = |a: u32| a >= lo && a < hi;
    let staged: BTreeMap<u32, u8> = shadow.writes().collect();
    for &a in orig.writes.keys().chain(staged.keys()) {
        if dead(a) {
            continue;
        }
        let unchanged = mem.rb(a);
        let (before, want) = orig.writes.get(&a).copied().unwrap_or((unchanged, unchanged));
        let got = staged.get(&a).copied().unwrap_or(before);
        if want != got {
            return Some(format!("memory {a:05x}: original {want:02x}, port {got:02x}"));
        }
    }
    if port.accesses != orig.accesses {
        let n = port.accesses.iter().zip(&orig.accesses).take_while(|(a, b)| a == b).count();
        return Some(format!(
            "port access #{n} of {}/{}: original {:x?}, port {:x?}",
            orig.accesses.len(),
            port.accesses.len(),
            orig.accesses.get(n),
            port.accesses.get(n)
        ));
    }
    if let Some(v) = &port.vga {
        if v != vga {
            return Some("VGA state differs".into());
        }
    }
    if port.steps != orig.steps {
        return Some(format!("steps: original {}, port {}", orig.steps, port.steps));
    }
    None
}

/// Replace the driver's ported exports with the native port.
pub fn install(m: &mut Machine) {
    for &(offset, _slot, _name, port) in EXPORTS {
        let mut shadow = Shadow::default();
        m.hook_dev(
            OVERLAY,
            offset,
            Box::new(move |cpu: &mut Cpu, dev: &mut Devices, limit: u64| {
                let Ok(out) = run_port(cpu, &dev.vga, dev.ips, &mut shadow, port) else {
                    return false;
                };
                if cpu.icount.saturating_add(out.steps) > limit {
                    return false;
                }
                commit(cpu, dev, &shadow, out);
                true
            }),
        );
    }
}

/// Results of [`verify`].
#[derive(Default, Debug)]
pub struct VerifyStats {
    pub calls: u64,
    /// Calls left to the runtime because an interrupt or the slice end fell inside them.
    pub declined: u64,
    /// Calls left to the runtime because the port does not handle that input: (export, reason).
    pub unported: BTreeMap<(&'static str, &'static str), u64>,
    pub mismatches: u64,
    pub per_export: BTreeMap<&'static str, u64>,
    /// The first few mismatches: (export, input state, difference).
    pub examples: Vec<(String, math3::Cpu, String)>,
}

/// Shadow verification: at every call of a ported export compute the port's effect, then run
/// the original and compare. The original's results are kept, so the game runs exactly as
/// unported. Calls the port would decline are left to the runtime, as in [`install`].
pub fn verify(m: &mut Machine) -> Rc<RefCell<VerifyStats>> {
    let stats = Rc::new(RefCell::new(VerifyStats::default()));
    for &(offset, _slot, name, port) in EXPORTS {
        let stats = stats.clone();
        let mut shadow = Shadow::default();
        m.hook_dev(
            OVERLAY,
            offset,
            Box::new(move |cpu: &mut Cpu, dev: &mut Devices, limit: u64| {
                let input = state(cpu);
                let out = match run_port(cpu, &dev.vga, dev.ips, &mut shadow, port) {
                    Ok(o) => o,
                    Err(Decline(why)) => {
                        *stats.borrow_mut().unported.entry((name, why)).or_default() += 1;
                        return false;
                    }
                };
                if cpu.icount.saturating_add(out.steps) > limit {
                    stats.borrow_mut().declined += 1;
                    return false;
                }
                let diff = match record_original(cpu, dev, 50_000_000) {
                    Some(orig) => compare(&out, &shadow, &orig, &cpu.mem, &dev.vga),
                    None => Some("original faulted, called a host service or did not return".into()),
                };
                let mut st = stats.borrow_mut();
                st.calls += 1;
                *st.per_export.entry(name).or_default() += 1;
                if let Some(d) = diff {
                    st.mismatches += 1;
                    if st.examples.len() < 8 {
                        st.examples.push((name.to_string(), input, d));
                    }
                }
                true
            }),
        );
    }
    stats
}
