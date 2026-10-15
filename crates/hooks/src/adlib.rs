//! The native AdLib driver ([`frontier_game::adlib`]) in the runtime: hooks at the 11 exports
//! of `EL2ADLIB.OVL`, a recorder for the original's port writes, and shadow verification
//! (docs/AUDIO.md).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use frontier_dos::audio::PortQueue;
use frontier_dos::cpu::{Cpu, CS, DS, EAX, EBX, ECX, EDI, EDX, ESI, ESP, SS};
use frontier_dos::mem::{lin, Memory};
use frontier_dos::Machine;
use frontier_game::adlib::{Driver, Host, Regs, EXPORTS};

pub const OVERLAY: &str = "EL2ADLIB.OVL";

/// A port write: (instructions executed in the call before the `OUT`, port, value).
pub type Out = (u64, u16, u8);

/// Runtime memory seen through the port: reads go to memory, writes and `OUT`s are held until
/// committed.
struct Staged<'a> {
    mem: &'a Memory,
    writes: HashMap<u32, u8>,
    outs: Vec<Out>,
}

impl Host for Staged<'_> {
    fn read8(&mut self, a: u32) -> u8 {
        let a = a & 0xF_FFFF;
        self.writes.get(&a).copied().unwrap_or_else(|| self.mem.rb(a))
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.writes.insert(a & 0xF_FFFF, v);
    }
    fn out8(&mut self, steps: u64, port: u16, v: u8) {
        self.outs.push((steps, port, v));
    }
}

pub fn regs(cpu: &Cpu) -> Regs {
    let r = |i: usize| cpu.r[i] as u16;
    Regs { ax: r(EAX), bx: r(EBX), cx: r(ECX), dx: r(EDX), si: r(ESI), di: r(EDI) }
}

fn set_regs(cpu: &mut Cpu, r: &Regs) {
    for (i, v) in [(EAX, r.ax), (EBX, r.bx), (ECX, r.cx), (EDX, r.dx), (ESI, r.si), (EDI, r.di)] {
        cpu.r[i] = (cpu.r[i] & 0xFFFF_0000) | v as u32;
    }
}

/// What the port does for one call, without changing anything.
pub struct Outcome {
    pub regs: Regs,
    pub writes: HashMap<u32, u8>,
    pub outs: Vec<Out>,
    /// Instructions, including the final `RETF`.
    pub steps: u64,
    /// Byte-code ops executed (bit n = op n).
    pub ops: u128,
}

/// Run export `f` of the port on the CPU's state (the CPU is at the export's entry).
pub fn run_port(cpu: &Cpu, f: fn(&mut Driver)) -> Outcome {
    let mut host = Staged { mem: &cpu.mem, writes: HashMap::new(), outs: Vec::new() };
    let mut d = Driver::new(&mut host, cpu.s[CS], cpu.s[DS], regs(cpu));
    f(&mut d);
    let (regs, steps, ops) = (d.r, d.steps, d.ops);
    Outcome { regs, writes: host.writes, outs: host.outs, steps, ops }
}

/// Records the original's port writes with the instruction count at which each runs.
struct Recorder {
    steps: u64,
    outs: Vec<Out>,
}

impl frontier_dos::cpu::Io for Recorder {
    fn inb(&mut self, port: u16) -> u8 {
        // Only status reads used as a delay.
        if port == 0x388 {
            0x06
        } else {
            0xFF
        }
    }
    fn outb(&mut self, port: u16, v: u8) {
        self.outs.push((self.steps, port, v));
    }
}

/// The original's effect on one call.
pub struct Original {
    pub regs: Regs,
    pub steps: u64,
    /// Bytes written: (before, after), not counting its own stack frame.
    pub writes: BTreeMap<u32, (u8, u8)>,
    pub outs: Vec<Out>,
}

/// Run the original export the CPU is at until it returns to its caller, recording memory
/// writes and port writes. `None` if it faulted or ran away.
pub fn record_original(cpu: &mut Cpu, budget: u64) -> Option<Original> {
    let (ss, sp) = (cpu.s[SS], cpu.r[ESP] as u16);
    let ret = (cpu.mem.rw(lin(ss, sp as u32)), cpu.mem.rw(lin(ss, sp as u32 + 2)));
    let frame = (lin(ss, sp.wrapping_sub(0x100) as u32), lin(ss, sp as u32));
    let start = cpu.icount;
    cpu.mem.write_log = Some(Vec::new());
    cpu.skip_breakpoint = true;
    let mut io = Recorder { steps: 0, outs: Vec::new() };
    let ok = loop {
        io.steps = cpu.icount - start;
        match cpu.step(&mut io) {
            Ok(()) => {}
            Err(frontier_dos::cpu::Event::Breakpoint(_)) => {
                cpu.skip_breakpoint = true;
                continue;
            }
            Err(_) => break false,
        }
        if cpu.icount - start > budget {
            break false;
        }
        if (cpu.ip as u16, cpu.s[CS]) == ret {
            break true;
        }
    };
    let log = cpu.mem.write_log.take().unwrap_or_default();
    if !ok {
        return None;
    }
    let mut writes = BTreeMap::new();
    for &(a, old) in &log {
        if a >= frame.0 && a < frame.1 {
            continue;
        }
        writes.entry(a).or_insert((old, 0));
    }
    for (&a, w) in writes.iter_mut() {
        w.1 = cpu.mem.rb(a);
    }
    Some(Original { regs: regs(cpu), steps: cpu.icount - start, writes, outs: io.outs })
}

/// The first difference between the port's and the original's effect, if any.
pub fn compare(port: &Outcome, orig: &Original, mem: &Memory) -> Option<String> {
    if port.regs != orig.regs {
        return Some(format!("registers: original {:x?}, port {:x?}", orig.regs, port.regs));
    }
    let data = |o: &[Out]| o.iter().map(|&(_, p, v)| (p, v)).collect::<Vec<_>>();
    if data(&port.outs) != data(&orig.outs) {
        let n = port.outs.iter().zip(&orig.outs).take_while(|(a, b)| (a.1, a.2) == (b.1, b.2)).count();
        return Some(format!(
            "port writes differ at #{n} of {}/{}: original {:x?}, port {:x?}",
            orig.outs.len(),
            port.outs.len(),
            orig.outs.get(n),
            port.outs.get(n)
        ));
    }
    for &a in orig.writes.keys().chain(port.writes.keys()) {
        let unchanged = mem.rb(a);
        let (before, want) = orig.writes.get(&a).copied().unwrap_or((unchanged, unchanged));
        let got = port.writes.get(&a).copied().unwrap_or(before);
        if want != got {
            return Some(format!("memory {a:05x}: original {want:02x}, port {got:02x}"));
        }
    }
    if let Some(i) = port.outs.iter().zip(&orig.outs).position(|(a, b)| a.0 != b.0) {
        return Some(format!("timing of port write #{i}: original {}, port {}", orig.outs[i].0, port.outs[i].0));
    }
    if port.steps != orig.steps {
        return Some(format!("steps: original {}, port {}", orig.steps, port.steps));
    }
    None
}

fn apply(cpu: &mut Cpu, out: &Outcome, queue: &PortQueue) {
    let start = cpu.icount;
    for (&a, &v) in &out.writes {
        cpu.mem.wb(a, v);
    }
    set_regs(cpu, &out.regs);
    // RETF.
    cpu.ip = cpu.pop16() as u32;
    cpu.s[CS] = cpu.pop16();
    cpu.icount += out.steps;
    queue.borrow_mut().extend(out.outs.iter().map(|&(s, p, v)| (start + s, p, v)));
}

/// Replace the driver's exports with the native port.
pub fn install(m: &mut Machine) {
    for &(offset, _slot, _name, f) in EXPORTS {
        let queue = m.dev.adlib.pending.clone();
        m.hook(
            OVERLAY,
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let out = run_port(cpu, f);
                if cpu.icount + out.steps > limit {
                    return false;
                }
                apply(cpu, &out, &queue);
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
    pub mismatches: u64,
    pub per_export: BTreeMap<&'static str, u64>,
    /// The first few mismatches: (export, input registers, difference).
    pub examples: Vec<(String, Regs, String)>,
}

/// Shadow verification: at every driver call compute the port's effect, then run the original
/// and compare. The original's results are kept (and its port writes reach the chip), so the
/// game sounds and runs exactly as unported. Calls the port would decline (an interrupt due
/// inside) are left to the runtime, as in [`install`].
pub fn verify(m: &mut Machine) -> Rc<RefCell<VerifyStats>> {
    let stats = Rc::new(RefCell::new(VerifyStats::default()));
    for &(offset, _slot, name, f) in EXPORTS {
        let stats = stats.clone();
        let queue = m.dev.adlib.pending.clone();
        m.hook(
            OVERLAY,
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let input = regs(cpu);
                let start = cpu.icount;
                let out = run_port(cpu, f);
                if cpu.icount + out.steps > limit {
                    stats.borrow_mut().declined += 1;
                    return false;
                }
                let diff = match record_original(cpu, 50_000_000) {
                    Some(orig) => {
                        queue.borrow_mut().extend(orig.outs.iter().map(|&(s, p, v)| (start + s, p, v)));
                        compare(&out, &orig, &cpu.mem)
                    }
                    None => Some("original faulted or did not return".into()),
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
