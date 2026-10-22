# Random number generators

This covers the main program's pseudo-random number generators: their state, their seeding, the
three entry points, who calls them, and how the native port was proven. Evidence: the Ghidra
12.1.4 disassembly listing (`tools/ghidra/ExportDisasm.java`), a byte scan of every shipped file
for far calls through the generators' vector slots, and execution on the reference runtime
([RUNTIME.md](RUNTIME.md)): fuzzed calls and shadow verification during play. The galaxy and
star-system generator is not covered here. Its search started and is recorded under
*Open questions*.

## State and seeding

Two of the routines share one 64-bit state in the main data segment.

| Address | Size | Name here | Seed |
|---|---|---|---|
| `DS:1fd9` | dword | `s0` | `12345678h` |
| `DS:1fdd` | dword | `s1` | `89ABCDEFh` |

**Observed:** the seed is stored at `1000:0c1c` (inside `modules_init_all`, [OVERLAYS.md](OVERLAYS.md)
§4) and again at `1000:0c6d`. **Measured:** after 5 M instructions of start-up, the state on the
runtime is a short sequence of draws from that seed, within 12 steps of the two generators
(`seed_is_where_start_up_puts_it`).

## Entry points

All three end in `RETF` and are reached through the main vector table. Overlays call them with
`lcall [slot]`. The main program calls them with near calls after `PUSH CS` (`1000:1309`,
`1000:16c0` and others).

| Slot | Address | Port | What it does (observed) |
|---|---|---|---|
| `DS:0124` | `1000:069b` | `random_port` | `random(n)`: `AX` = n. Advances the state (`s0 += s1; s1 = rol(s1, 5); s0 += s1; s0 = ror(s0, 16)`), then `AX = (n × low16(s0)) >> 16`, a value in `0..n`. `EBX` and `EDX` are pushed and popped, so they are kept whole. 15 instructions |
| `DS:012c` | `1000:0645` | `random32_port` | 32-bit draw: `EDX = ror(s1 xor 1234567Fh, 7)`, `s0 += EDX`, `EAX = ror(old s0, 19)`, `s1 = EAX`. Returns `EAX`, and leaves `EDX` changed. 9 instructions |
| `DS:0128` | `1000:066a` | `mix_port` | Stateless mixer of the 32-bit pairs `CX:BX` and `SI:DI`: `CX:BX += SI:DI`, `SI:DI = rol(SI:DI, 5)`, `CX:BX += SI:DI`, then `XCHG CX,BX`, `MUL BX` and `XCHG AX,DX`. `AX` gets the high word and `DX` the low word of `AX × new BX`. 23 instructions |

The `random(n)` scaling takes the high word of a 16 × 16 product, so values are evenly spread
without a division. `EL2ADLIB` uses slot `0124` to pick random songs ([AUDIO.md](AUDIO.md)).

## Callers

Far calls through the slots in each shipped file (byte pattern `FF 1E` + slot):

| File | `random` (`0124`) | `mix` (`0128`) | `random32` (`012c`) |
|---|---|---|---|
| `EL2M02` (galaxy map) | 2 | 0 | 6 |
| `EL2M03` (cockpit) | 0 | 0 | 13 |
| `EL2M07` (commodities) | 14 | 0 | 21 |
| `EL2M09` (people's names) | 0 | 8 | 0 |
| `EL2M11` | 14 | 0 | 16 |
| `EL2M12` | 2 | 3 | 0 |
| `EL2M14` (equipment) | 1 | 0 | 5 |
| `EL2M15`, `EL2M16`, `EL2M17` | 2, 1, 1 | 0 | 3, 3, 1 |
| `EL2ADLIB`, `EL2ROL` | 1, 1 | 0 | 0 |

`FRONTIER.EXE` reaches them with near calls (18 sites between `1000:1309` and `1000:2569`, for
example). `EL2M09` uses `mix` to pick names (first names such as "Brian" and "Chris" in its
strings) from register-pair seeds, so names are a function of their seed, not of the shared state
(*inferred*).

## How parity was proven

1. **Fuzzing** (`crates/hooks/tests/rng_parity.rs`): 20,000 calls per routine, inside the booted
   game, with random registers and random state. Registers, every byte written (the state) and
   the instruction count are compared with the original. All match.
2. **Shadow verification in play** (`cargo run -p frontier-hooks --example rng_verify -- … 60`):
   60 virtual seconds of intro, menu, start at Ross 154 and the cockpit. Each call runs the port
   and the original and compares them. **798 calls (113 `random`, 25 `mix`, 660 `random32`),
   0 mismatches.**

The ports are installed in the game by `frontier_hooks::install_all`.

## Open questions

- **Galaxy and system generation.** `EL2M02` is the galaxy map module. It holds the catalogue
  of named nearby stars (Alpha Centauri, Ross 128, …). Its handler `+14` prints sector
  coordinates relative to `1718h, 1524h`, which is Sol's sector (*inferred*). `EL2M02:2537`
  sets up a system from an index in `AX` and a sector in `BX`, then calls `EL2M06` and `EL2M16`
  through their handler `+20`. `EL2M06:3eb9` (handler `+20`) looks a 32-bit system id up in a
  table of hand-made systems (30h-byte records from `EL2M06:01f3`). Otherwise it picks a template
  by squared distance from Sol's sector (thresholds `1FA4h`, `E1h`, `64h`). The procedural
  generator for systems that are not in the table is not found yet. The coverage-discovery work
  (galaxy map runs) should name it.
- Whether any caller depends on the flags these routines leave (`OR EAX,EAX` at the end of
  `random32` sets ZF and SF for the caller). Flags are not modelled.
