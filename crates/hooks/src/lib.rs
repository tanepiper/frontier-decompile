//! Bridges native ports into the reference runtime. Each original routine that has a proven
//! port (a parity test in `tests/`) is replaced, at its address, by the Rust implementation.
//! As ports accumulate, less and less of the original runs (docs/RUNTIME.md).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use frontier_dos::cpu::{Cpu, CS, DS, ES, ESP, SS};
use frontier_dos::mem::Memory;
use frontier_dos::Machine;
use frontier_game::math3::{self, Port};

pub use frontier_game::math3::{Regs, DIVIDE_RESUME};

pub fn regs_from(cpu: &Cpu) -> Regs {
    Regs::load(&state(cpu))
}

pub fn regs_to(cpu: &mut Cpu, r: &Regs) {
    let mut s = state(cpu);
    r.store(&mut s);
    cpu.r = s.r;
}

/// The runtime CPU as a port sees it.
pub fn state(cpu: &Cpu) -> math3::Cpu {
    math3::Cpu { r: cpu.r, cs: cpu.s[CS], ip: cpu.ip as u16, ds: cpu.s[DS], es: cpu.s[ES], ss: cpu.s[SS], flags: cpu.flags }
}

fn apply(cpu: &mut Cpu, s: &math3::Cpu) {
    cpu.r = s.r;
    cpu.s[CS] = s.cs;
    cpu.ip = s.ip as u32;
    cpu.s[DS] = s.ds;
    cpu.s[ES] = s.es;
    cpu.s[SS] = s.ss;
}

/// Runtime memory seen through a port: reads go to memory, writes are held back until
/// [`Staged::commit`], so a port that declines leaves no trace.
pub struct Staged<'a> {
    mem: &'a Memory,
    pub writes: HashMap<u32, u8>,
}

impl<'a> Staged<'a> {
    pub fn new(mem: &'a Memory) -> Self {
        Staged { mem, writes: HashMap::new() }
    }
}

impl math3::Bus for Staged<'_> {
    fn read8(&mut self, a: u32) -> u8 {
        let a = a & 0xF_FFFF;
        match self.writes.get(&a) {
            Some(&v) => v,
            None => self.mem.rb(a),
        }
    }

    fn write8(&mut self, a: u32, v: u8) {
        self.writes.insert(a & 0xF_FFFF, v);
    }
}

/// What a port did to a call: the CPU after its `RETF`, the bytes it wrote, its instruction count.
pub struct Outcome {
    pub cpu: math3::Cpu,
    pub writes: HashMap<u32, u8>,
    pub steps: u64,
}

/// Run `port` on the current state of `cpu` without changing anything.
pub fn run_port(cpu: &Cpu, port: Port) -> Outcome {
    let mut s = state(cpu);
    let mut bus = Staged::new(&cpu.mem);
    let steps = port(&mut s, &mut bus);
    Outcome { cpu: s, writes: bus.writes, steps }
}

/// The original's effect on one call, recorded by [`record_original`].
pub struct Original {
    /// CPU after its `RETF`.
    pub cpu: math3::Cpu,
    pub steps: u64,
    /// Every byte it wrote: (value before, value after).
    pub writes: BTreeMap<u32, (u8, u8)>,
}

/// Run the original routine the CPU is at (see [`run_original`]) with the memory write log on.
pub fn record_original(cpu: &mut Cpu, budget: u64) -> Option<Original> {
    cpu.mem.write_log = Some(Vec::new());
    let steps = run_original(cpu, budget);
    let log = cpu.mem.write_log.take().unwrap_or_default();
    let mut writes = BTreeMap::new();
    for &(a, old) in &log {
        writes.entry(a).or_insert((old, 0));
    }
    for (&a, w) in writes.iter_mut() {
        w.1 = cpu.mem.rb(a);
    }
    if steps.is_none() {
        // Leave memory as it was, so the caller can carry on.
        for (&a, &(old, _)) in &writes {
            cpu.mem.wb(a, old);
        }
    }
    Some(Original { cpu: state(cpu), steps: steps?, writes })
}

/// Compare a port's [`Outcome`] with the original's ([`Original`]) for the same call, starting
/// from memory `mem` as it was before the call (or as the original left it, for addresses the
/// original did not write). Registers, segments, return address, instruction count and every
/// byte written by either are compared, except the dead stack below the final `SS:SP`.
/// Returns a description of the first difference.
pub fn compare(port: &Outcome, orig: &Original, mem: &Memory) -> Option<String> {
    let strip = |c: &math3::Cpu| math3::Cpu { flags: 0, ..*c };
    if strip(&port.cpu) != strip(&orig.cpu) {
        return Some(format!("state: original {:x?}, port {:x?}", orig.cpu, port.cpu));
    }
    if port.steps != orig.steps {
        return Some(format!("steps: original {}, port {}", orig.steps, port.steps));
    }
    let (stack_lo, stack_hi) = (math3::lin(orig.cpu.ss, 0), math3::lin(orig.cpu.ss, orig.cpu.x(math3::ESP)));
    let dead = |a: u32| a >= stack_lo && a < stack_hi;
    for &a in orig.writes.keys().chain(port.writes.keys()) {
        if dead(a) {
            continue;
        }
        let unchanged = mem.rb(a);
        let (before, want) = orig.writes.get(&a).copied().unwrap_or((unchanged, unchanged));
        let got = port.writes.get(&a).copied().unwrap_or(before);
        if want != got {
            return Some(format!("memory {a:05x}: original {want:02x}, port {got:02x}"));
        }
    }
    None
}

/// Replace the `EL2MATH3` routines in [`math3::PORTS`] with their native ports.
pub fn install_math3(m: &mut Machine) {
    for &(offset, _slot, _name, port) in math3::PORTS {
        m.hook(
            "EL2MATH3.OVL",
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let out = run_port(cpu, port);
                if cpu.icount + out.steps > limit {
                    return false;
                }
                apply(cpu, &out.cpu);
                for (&a, &v) in &out.writes {
                    cpu.mem.wb(a, v);
                }
                cpu.icount += out.steps;
                true
            }),
        );
    }
}

/// Results of shadow verification ([`verify_math3`]).
#[derive(Default, Debug)]
pub struct VerifyStats {
    pub calls: u64,
    /// Calls left to the runtime because an interrupt or the slice end fell inside them.
    pub declined: u64,
    pub mismatches: u64,
    /// Verified calls per routine name.
    pub per_routine: BTreeMap<&'static str, u64>,
    /// The first few mismatches: (routine, input registers, difference).
    pub examples: Vec<(String, Regs, String)>,
}

/// I/O is not expected inside the maths routines; any access is recorded as a mismatch.
struct NoIo(bool);
impl frontier_dos::cpu::Io for NoIo {
    fn inb(&mut self, _: u16) -> u8 {
        self.0 = true;
        0xFF
    }
    fn outb(&mut self, _: u16, _: u8) {
        self.0 = true;
    }
}

/// Run the original routine at the CPU's current position until it returns to its caller (the
/// return address on the stack at entry; the frame allocators return with a lower `SP`).
/// Breakpoints of other hooked entries it reaches are passed.
/// Returns the instruction count, or `None` if it did I/O, faulted or ran away.
pub fn run_original(cpu: &mut Cpu, budget: u64) -> Option<u64> {
    let (ss, sp) = (cpu.s[SS], cpu.r[ESP] as u16);
    let ret = (cpu.mem.rw(frontier_dos::mem::lin(ss, sp as u32)), cpu.mem.rw(frontier_dos::mem::lin(ss, sp as u32 + 2)));
    let start = cpu.icount;
    cpu.skip_breakpoint = true;
    let mut io = NoIo(false);
    loop {
        match cpu.step(&mut io) {
            Ok(()) => {}
            Err(frontier_dos::cpu::Event::Breakpoint(_)) => {
                cpu.skip_breakpoint = true;
                continue;
            }
            Err(e) => {
                if std::env::var_os("HOOKS_DEBUG").is_some() {
                    eprintln!("run_original: {e:?} at {:04x}:{:04x}", cpu.s[CS], cpu.ip);
                }
                return None;
            }
        }
        if cpu.icount - start > budget || io.0 {
            return None;
        }
        if (cpu.ip as u16, cpu.s[CS]) == ret {
            return Some(cpu.icount - start);
        }
    }
}

/// Shadow verification: at each call of a ported `EL2MATH3` routine, compute the port's result,
/// then run the original to its return and compare registers, memory and the instruction
/// count ([`compare`]). The original's results are kept, so the game is exactly the unported
/// game. Calls the port would decline (an interrupt due inside) are left to the runtime, as in
/// [`install_math3`].
pub fn verify_math3(m: &mut Machine) -> Rc<RefCell<VerifyStats>> {
    let stats = Rc::new(RefCell::new(VerifyStats::default()));
    for &(offset, _slot, name, port) in math3::PORTS {
        let stats = stats.clone();
        m.hook(
            "EL2MATH3.OVL",
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let input = regs_from(cpu);
                let out = run_port(cpu, port);
                if cpu.icount + out.steps > limit {
                    stats.borrow_mut().declined += 1;
                    return false;
                }
                let diff = match record_original(cpu, 10_000_000) {
                    Some(orig) => compare(&out, &orig, &cpu.mem),
                    None => Some("original did I/O, faulted or did not return".into()),
                };
                let mut st = stats.borrow_mut();
                st.calls += 1;
                *st.per_routine.entry(name).or_default() += 1;
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

/// Every native port available.
pub fn install_all(m: &mut Machine) {
    install_math3(m);
}
