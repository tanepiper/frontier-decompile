# System data and the economy

This covers what makes each star system different in trade terms: the per-system data in `EL2M06`,
and, as it is traced, the market built in `EL2M07` when the player arrives. Evidence: the Ghidra
12.1.4 disassembly listings, the coverage of a hyperspace jump ([COVERAGE.md](COVERAGE.md)), and
execution on the reference runtime (fuzzed calls against the original). The market builder itself
(`EL2M07:0949`) is traced below but not ported yet.

## Per-system data: `EL2M06:3eb9` (module 6, handler `+20`)

Ported as `frontier_game::sysdata::system_data_port`. Input: the 32-bit system id in `DS:07c0`
(sector `y` from bit 19, `x` in bits 6–18, the star's index below, [GALAXY.md](GALAXY.md)) and a
selector in `AL`.

**The record.** Hand-made systems have a 30h-byte record each in `EL2M06`, from `EL2M06:01f3`,
found by id. The table ends with id 0. Every other system takes one of seven **template records**
by its squared distance from Sol's sector. The distance is
`d = (|x − 1718h| + j)² + (|y − 1524h| + j)²`, in 16 bits, with a jitter `j` = `id & 7`:

| `d` at least | Record | (*inferred*) |
|---|---|---|
| 1FA4h | `EL2M06:00a3` | the frontier |
| E1h | `00d3` | |
| 64h | `0103` | |
| 31h | `0133` | |
| 19h | `0163` | |
| 9 | `0193` | |
| below | `01c3` | the core around Sol |

Every system at a given distance therefore shares the same record. The bytes grow towards the core.

**The selector:**

| `AL` | Result |
|---|---|
| 0 | `DS:07c0`/`07c4` = the record's words `+2Ch`/`+2Eh`; `DS:07bc`, `07c8`, `07cc`, `07d0`, `07d4`, `07d8` = its bytes `+5` to `+0Ah` |
| 1 | `AX` = byte `+4`, `BX` = byte `+0Bh`, `DS:07c4` = byte `+0Ah`, `ES:DI` = `ES:SI` = the record + 0Ch (a block of bytes the market builder copies) |
| other | `DS:07bc`/`DS:07c4` = the low and `DS:07c0` the high nibble of byte `+2Bh`; `DS:07c8` = byte `+0Ah` >> 6; `DS:07d8`/`07d4` = `DS:20f1`/`20f3`, swapped unless the low nibble is the larger (or they are equal and `DS:07c8` = 2); when the high nibble is larger, `DS:07c4` takes it |

Parity: `crates/hooks/tests/galaxy_parity.rs`, `system_data_matches_original`. There were 6,000
calls with random ids (a quarter of them from the hand-made table), random registers and every
selector. Every register, every byte written and the instruction count were compared: **0
mismatches.**

## The market on arrival: `EL2M07` (traced, not ported)

On a hyperspace arrival, module 7's handler `+0c` with `AX` = 2 does the following:

1. It calls `EL2M07:0818`, which counts the stations through the main program's object iterator
   (slot `DS:0000`, callback `EL2M07:0831`).
2. It rotates the four dwords at `DS:4603`–`460f`.
3. It sets `DS:07c0` from `DS:208b` (the current system, *inferred*).
4. It builds the market in `EL2M07:0949`. This reads the system data three times through module 6
   (`AL` = 0, 1, 2), copies 31 bytes of the record's `+0Ch` block, and computes each of up to 31
   commodities' price and stock (`EL2M07:0855`, `08c0`). These use tables at `EL2M07:0251`–`0299`
   and draws from `random32` (slot `DS:012c`, [RNG.md](RNG.md)), so a market depends on the shared
   generator's state. The builder then makes random events (`EL2M07:0923`, a jump table at
   `EL2M07:03a1`) and the per-station lists (`EL2M07:101a`).

## Open questions

- The meaning of each byte of the system record (economy, government, population, tech level are
  likely, *inferred*), and of the commodity tables in `EL2M07`.
- The market builder and its helpers, to be ported next.
