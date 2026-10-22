//! Bridges native ports into the reference runtime. Each original routine that has a proven
//! port (a parity test in `tests/`) is replaced, at its address, by the Rust implementation.
//! As ports accumulate, less and less of the original runs (docs/RUNTIME.md).

pub mod adlib;
pub mod video;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use frontier_dos::cpu::{Cpu, CS, DS, ES, FS, GS, SS};
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
    math3::Cpu { r: cpu.r, cs: cpu.s[CS], ip: cpu.ip as u16, ds: cpu.s[DS], es: cpu.s[ES], ss: cpu.s[SS], fs: cpu.s[FS], gs: cpu.s[GS], flags: cpu.flags }
}

pub(crate) fn apply(cpu: &mut Cpu, s: &math3::Cpu) {
    cpu.r = s.r;
    cpu.s[CS] = s.cs;
    cpu.ip = s.ip as u32;
    cpu.s[DS] = s.ds;
    cpu.s[ES] = s.es;
    cpu.s[SS] = s.ss;
    cpu.s[FS] = s.fs;
    cpu.s[GS] = s.gs;
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
    record_with(cpu, budget, run_original)
}

/// [`record_original`] for a near routine (one that ends in `RET`), run by [`run_original_near`].
pub fn record_original_near(cpu: &mut Cpu, budget: u64) -> Option<Original> {
    record_with(cpu, budget, run_original_near)
}

/// [`record_original`] that stops where a port stopped: when the original reaches `CS:IP` =
/// `cs:ip` with `SP` = `sp`. Covers ports that return (the caller's return address) and ports
/// that end by entering original code (a tail call into a routine that is not ported).
pub fn record_original_until(cpu: &mut Cpu, budget: u64, cs: u16, ip: u16, sp: u16) -> Option<Original> {
    let mut run = |cpu: &mut Cpu, budget: u64| run_original_until(cpu, budget, cs, ip, sp);
    record_with_dyn(cpu, budget, &mut run)
}

/// Run original code until `CS:IP` = `cs:ip` with `SP` = `sp` (after at least one instruction).
pub fn run_original_until(cpu: &mut Cpu, budget: u64, cs: u16, ip: u16, sp: u16) -> Option<u64> {
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
            Err(_) => return None,
        }
        if cpu.icount - start > budget || io.0 {
            return None;
        }
        if cpu.s[CS] == cs && cpu.ip as u16 == ip && cpu.r[frontier_dos::cpu::ESP] as u16 == sp {
            return Some(cpu.icount - start);
        }
    }
}

fn record_with(cpu: &mut Cpu, budget: u64, run: fn(&mut Cpu, u64) -> Option<u64>) -> Option<Original> {
    let mut run = |cpu: &mut Cpu, budget: u64| run(cpu, budget);
    record_with_dyn(cpu, budget, &mut run)
}

fn record_with_dyn(cpu: &mut Cpu, budget: u64, run: &mut dyn FnMut(&mut Cpu, u64) -> Option<u64>) -> Option<Original> {
    cpu.mem.write_log = Some(Vec::new());
    let steps = run(cpu, budget);
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
    install_ports(m, "EL2MATH3.OVL", math3::PORTS);
}

/// Replace each routine in `ports` (offsets in `module`'s load image, Ghidra segment `1000`) with
/// its native port. A call is left to the original when the port declines it or when an
/// interrupt is due before the original would have returned.
pub fn install_ports(m: &mut Machine, module: &str, ports: &'static [(u16, u16, &'static str, Port)]) {
    for &(offset, _slot, _name, port) in ports {
        m.hook(
            module,
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let out = run_port(cpu, port);
                if cpu.icount.saturating_add(out.steps) > limit {
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
    /// Calls left to the runtime because the port does not handle that input yet.
    pub unported: BTreeMap<&'static str, u64>,
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

/// The far-call effect of the instruction at `CS:IP`: +1 for `CALLF` (and for a near `CALL`
/// right after `PUSH CS`, the far-call idiom at `EL2MATH3:2a29`), −1 for `RETF`, else 0.
pub(crate) fn far_call_depth(cpu: &Cpu) -> i32 {
    let mut a = frontier_dos::mem::lin(cpu.s[CS], cpu.ip);
    let mut op = cpu.mem.rb(a);
    while matches!(op, 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67) {
        a += 1;
        op = cpu.mem.rb(a);
    }
    match op {
        0x9A => 1,
        0xFF if (cpu.mem.rb(a + 1) >> 3) & 7 == 3 => 1,
        0xE8 if cpu.mem.rb(a.wrapping_sub(1)) == 0x0E => 1,
        0xCA | 0xCB => -1,
        _ => 0,
    }
}

/// Run the original routine at the CPU's current position until the `RETF` that leaves it
/// (counting the far calls it makes). Breakpoints of other hooked entries it reaches are passed.
/// Returns the instruction count, or `None` if it did I/O, faulted or ran past `budget`.
pub fn run_original(cpu: &mut Cpu, budget: u64) -> Option<u64> {
    let start = cpu.icount;
    let mut depth = 0;
    cpu.skip_breakpoint = true;
    let mut io = NoIo(false);
    loop {
        let d = far_call_depth(cpu);
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
        depth += d;
        if depth < 0 {
            return Some(cpu.icount - start);
        }
    }
}

/// Run the near routine the CPU is at (just called: the return offset is on top of the stack)
/// until it returns to that offset in the same code segment with the stack back where it was.
/// Returns the instruction count, or `None` if it did I/O, faulted or ran past `budget`.
pub fn run_original_near(cpu: &mut Cpu, budget: u64) -> Option<u64> {
    let start = cpu.icount;
    let (cs, sp) = (cpu.s[CS], cpu.r[frontier_dos::cpu::ESP] as u16);
    let ret = cpu.mem.rw(frontier_dos::mem::lin(cpu.s[SS], sp as u32));
    let done_sp = sp.wrapping_add(2);
    cpu.skip_breakpoint = true;
    let mut io = NoIo(false);
    loop {
        match cpu.step(&mut io) {
            Ok(()) => {}
            Err(frontier_dos::cpu::Event::Breakpoint(_)) => {
                cpu.skip_breakpoint = true;
                continue;
            }
            Err(_) => return None,
        }
        if cpu.icount - start > budget || io.0 {
            return None;
        }
        if cpu.s[CS] == cs && cpu.ip as u16 == ret && cpu.r[frontier_dos::cpu::ESP] as u16 == done_sp {
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
    verify_ports(m, "EL2MATH3.OVL", math3::PORTS)
}

/// Shadow verification of any port table, as [`verify_math3`] does for `EL2MATH3`. An entry
/// whose vector slot is 0 is a near routine (no slot; it ends in `RET`) or may end by entering
/// original code; it is recorded with [`record_original_until`] up to where the port stopped.
pub fn verify_ports(m: &mut Machine, module: &str, ports: &'static [(u16, u16, &'static str, Port)]) -> Rc<RefCell<VerifyStats>> {
    let stats = Rc::new(RefCell::new(VerifyStats::default()));
    for &(offset, slot, name, port) in ports {
        let stats = stats.clone();
        m.hook(
            module,
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let input = regs_from(cpu);
                let out = run_port(cpu, port);
                if out.steps == math3::DECLINE {
                    *stats.borrow_mut().unported.entry(name).or_default() += 1;
                    return false;
                }
                if cpu.icount + out.steps > limit {
                    stats.borrow_mut().declined += 1;
                    return false;
                }
                let diff = match if slot == 0 { record_original_until(cpu, 10_000_000, out.cpu.cs, out.cpu.ip, out.cpu.x(math3::ESP)) } else { record_original(cpu, 10_000_000) } {
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
    install_ports(m, "FRONTIER.EXE", frontier_game::rng::PORTS);
    install_ports(m, "EL2M02.OVL", frontier_game::galaxy::PORTS_M02);
    install_ports(m, "EL2M09.OVL", frontier_game::names::PORTS_M09);
    adlib::install(m);
    video::install(m);
}
