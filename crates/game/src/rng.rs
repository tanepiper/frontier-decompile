//! The main program's pseudo-random number generators (docs/RNG.md).
//!
//! Two generators share one 64-bit state in the main data segment, `DS:1fd9` (low dword, called
//! `s0` here) and `DS:1fdd` (`s1`), seeded with `12345678h` and `89ABCDEFh` (observed at
//! `1000:0c1c` and `1000:0c6d`). A third routine is a stateless mixer of 32-bit register pairs.
//! All three are reached through the main vector table: `DS:0124`, `DS:0128` and `DS:012c`.
//!
//! The ports are register- and memory-exact and return the original's instruction count, like the
//! `EL2MATH3` ports ([`crate::math3::Port`]). Parity: `crates/hooks/tests/rng_parity.rs`.

use crate::math3::{lin, Bus, Cpu, Port, EAX, EBX, ECX, EDI, EDX, ESI};

/// Offset of the generator state's low dword in the main data segment.
pub const STATE_S0: u16 = 0x1fd9;
/// Offset of the generator state's high dword.
pub const STATE_S1: u16 = 0x1fdd;
/// Seed values stored by `1000:0c1c` and `1000:0c6d` (observed).
pub const SEED: (u32, u32) = (0x1234_5678, 0x89AB_CDEF);

/// One step of the `random(n)` generator on the state (`1000:069f`–`06bc`): returns the new
/// state. `s0 += s1; s1 = rol(s1, 5); s0 += s1; s0 = ror(s0, 16)`.
pub fn step(s0: u32, s1: u32) -> (u32, u32) {
    let mut ebx = s0.wrapping_add(s1);
    let edx = s1.rotate_left(5);
    ebx = ebx.wrapping_add(edx);
    (ebx.rotate_right(16), edx)
}

/// `random(n)` on a state, as the game uses it: advances the state and returns
/// `(n × low16(new s0)) >> 16`, a value in `0..n` (0 when `n` is 0).
pub fn random(state: &mut (u32, u32), n: u16) -> u16 {
    *state = step(state.0, state.1);
    ((n as u32 * (state.0 & 0xFFFF)) >> 16) as u16
}

/// One step of the 32-bit generator (`1000:0645`): returns (new state, value returned in `EAX`).
pub fn step32(s0: u32, s1: u32) -> ((u32, u32), u32) {
    let edx = (s1 ^ 0x1234_567F).rotate_right(7);
    let eax = s0.rotate_right(19);
    ((s0.wrapping_add(edx), eax), eax)
}

/// Port of `1000:069b` (slot `DS:0124`), *observed*: `AX = random(AX)`. Advances the state with
/// [`step`], then `MUL BX` with the low word of the new `s0` and returns the high word of the
/// product in `AX`. `EBX` and `EDX` are pushed and popped, so they are preserved whole; the high
/// word of `EAX` is untouched. 15 instructions.
pub fn random_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (a0, a1) = (lin(c.ds, STATE_S0), lin(c.ds, STATE_S1));
    let (s0, s1) = step(bus.read32(a0), bus.read32(a1));
    bus.write32(a0, s0);
    bus.write32(a1, s1);
    let p = c.x(EAX) as u32 * (s0 & 0xFFFF);
    c.set_x(EAX, (p >> 16) as u16);
    c.retf(bus);
    15
}

/// Port of `1000:0645` (slot `DS:012c`), *observed*: the 32-bit generator. `EDX = ror(s1 xor
/// 1234567Fh, 7)`, `s0 += EDX`, `EAX = ror(old s0, 19)`, `s1 = EAX`; returns `EAX` (the
/// original's closing `OR EAX,EAX` only sets flags). 9 instructions.
pub fn random32_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (a0, a1) = (lin(c.ds, STATE_S0), lin(c.ds, STATE_S1));
    let (s0, s1) = (bus.read32(a0), bus.read32(a1));
    let edx = (s1 ^ 0x1234_567F).rotate_right(7);
    let ((n0, n1), eax) = step32(s0, s1);
    bus.write32(a0, n0);
    bus.write32(a1, n1);
    c.r[EAX] = eax;
    c.r[EDX] = edx;
    c.retf(bus);
    9
}

/// Port of `1000:066a` (slot `DS:0128`), *observed*: a stateless mixer of the 32-bit pairs
/// `CX:BX` and `SI:DI`. `CX:BX += SI:DI`; `SI:DI = rol(SI:DI, 5)` (five `SHL DI; RCL SI;
/// ADC DI,0`); `CX:BX += SI:DI`; then `XCHG CX,BX`, `MUL BX` (`AX` × the new `BX`) and
/// `XCHG AX,DX`, so `AX` is the high and `DX` the low word of the product. 23 instructions.
pub fn mix_port(c: &mut Cpu, bus: &mut dyn Bus) -> u64 {
    let (cx, bx, si, di) = (c.x(ECX), c.x(EBX), c.x(ESI), c.x(EDI));
    let a = (cx as u32) << 16 | bx as u32;
    let b = (si as u32) << 16 | di as u32;
    let a = a.wrapping_add(b);
    let b = b.rotate_left(5);
    let a = a.wrapping_add(b);
    let (new_cx, new_bx) = (a as u16, (a >> 16) as u16); // after XCHG CX,BX
    let p = c.x(EAX) as u32 * new_bx as u32;
    c.set_x(ECX, new_cx);
    c.set_x(EBX, new_bx);
    c.set_x(ESI, (b >> 16) as u16);
    c.set_x(EDI, b as u16);
    c.set_x(EAX, (p >> 16) as u16);
    c.set_x(EDX, p as u16);
    c.retf(bus);
    23
}

/// Every ported routine of the main program's generators: (`FRONTIER.EXE` offset in segment
/// `1000`, vector slot, name, port).
pub const PORTS: &[(u16, u16, &str, Port)] = &[
    (0x069b, 0x0124, "random", random_port),
    (0x066a, 0x0128, "mix", mix_port),
    (0x0645, 0x012c, "random32", random32_port),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_stays_in_range() {
        let mut st = SEED;
        for n in [1u16, 6, 7, 100, 0xFFFF] {
            for _ in 0..1000 {
                assert!(random(&mut st, n) < n);
            }
        }
        assert_eq!(random(&mut st, 0), 0);
    }
}
