//! Bridges native ports into the reference runtime. Each original routine that has a proven
//! port (a parity test in `tests/`) is replaced, at its address, by the Rust implementation.
//! As ports accumulate, less and less of the original runs (docs/RUNTIME.md).

use std::cell::RefCell;
use std::rc::Rc;

use frontier_dos::cpu::{Cpu, CS, DS, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP, SS};
use frontier_dos::mem::lin;
use frontier_dos::Machine;
use frontier_game::math3::{self, Regs};

/// Main data segment offset of the divide-overflow resume address (docs/MATH3.md).
pub const DIVIDE_RESUME: u32 = 0x09dc;

pub fn regs_from(cpu: &Cpu) -> Regs {
    Regs {
        ax: cpu.r[EAX] as u16,
        bx: cpu.r[EBX] as u16,
        cx: cpu.r[ECX] as u16,
        dx: cpu.r[EDX] as u16,
        si: cpu.r[ESI] as u16,
        di: cpu.r[EDI] as u16,
        bp: cpu.r[EBP] as u16,
    }
}

pub fn regs_to(cpu: &mut Cpu, r: &Regs) {
    cpu.set_reg(EAX, 2, r.ax as u32);
    cpu.set_reg(EBX, 2, r.bx as u32);
    cpu.set_reg(ECX, 2, r.cx as u32);
    cpu.set_reg(EDX, 2, r.dx as u32);
    cpu.set_reg(ESI, 2, r.si as u32);
    cpu.set_reg(EDI, 2, r.di as u32);
    cpu.set_reg(EBP, 2, r.bp as u32);
}

/// Replace the `EL2MATH3` routines in [`math3::PORTS`] with their native ports.
pub fn install_math3(m: &mut Machine) {
    for &(offset, _slot, _name, port) in math3::PORTS {
        m.hook(
            "EL2MATH3.OVL",
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let mut r = regs_from(cpu);
                let exit = port(&mut r);
                if cpu.icount + exit.steps > limit {
                    return false;
                }
                regs_to(cpu, &r);
                if let Some(v) = exit.resume {
                    let a = lin(cpu.s[DS], DIVIDE_RESUME);
                    cpu.mem.ww(a, v);
                }
                cpu.retf();
                cpu.icount += exit.steps;
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
    /// The first few mismatches: (routine, input, original, port).
    pub examples: Vec<(String, Regs, Regs, Regs)>,
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

/// Shadow verification: at each call of a ported `EL2MATH3` routine, compute the port's result,
/// then run the original to its return and compare registers, `DS:09dc` and the instruction
/// count. The original's results are kept, so the game is exactly the unported game. Calls the
/// port would decline (an interrupt due inside) are left to the runtime, as in [`install_math3`].
pub fn verify_math3(m: &mut Machine) -> Rc<RefCell<VerifyStats>> {
    let stats = Rc::new(RefCell::new(VerifyStats::default()));
    for &(offset, _slot, name, port) in math3::PORTS {
        let stats = stats.clone();
        m.hook(
            "EL2MATH3.OVL",
            offset,
            Box::new(move |cpu: &mut Cpu, limit: u64| {
                let input = regs_from(cpu);
                let mut port_out = input;
                let exit = port(&mut port_out);
                if cpu.icount + exit.steps > limit {
                    stats.borrow_mut().declined += 1;
                    return false;
                }
                let start = cpu.icount;
                // Run the original until it returns to the caller (SP above the return address).
                let (ss, sp) = (cpu.s[SS], cpu.r[ESP] as u16);
                let ret = (cpu.mem.rw(lin(ss, sp as u32)), cpu.mem.rw(lin(ss, sp as u32 + 2)));
                cpu.skip_breakpoint = true;
                let mut io = NoIo(false);
                let mut n = 0u32;
                loop {
                    match cpu.step(&mut io) {
                        Ok(()) => {}
                        // Falling into another hooked entry (EL2MATH3:1d41 → 1d43): run it too.
                        Err(frontier_dos::cpu::Event::Breakpoint(_)) => {
                            cpu.skip_breakpoint = true;
                            continue;
                        }
                        Err(_) => break,
                    }
                    if n > 1_000_000 {
                        break;
                    }
                    n += 1;
                    if cpu.r[ESP] as u16 == sp.wrapping_add(4) && (cpu.ip as u16, cpu.s[CS]) == ret {
                        break;
                    }
                }
                let orig = regs_from(cpu);
                let orig_resume = cpu.mem.rw(lin(cpu.s[DS], DIVIDE_RESUME));
                let mut st = stats.borrow_mut();
                st.calls += 1;
                let resume_ok = exit.resume.map_or(true, |v| v == orig_resume);
                let steps_ok = cpu.icount - start == exit.steps;
                if orig != port_out || !resume_ok || !steps_ok || io.0 {
                    st.mismatches += 1;
                    if st.examples.len() < 8 {
                        st.examples.push((format!("{name} steps {} port {}", cpu.icount - start, exit.steps), input, orig, port_out));
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
