# Generated names

This covers the name generator in `EL2M09` (planets, colonies, people and catalogue-style
designations) and the main program's text expander it copies words with. Evidence: the Ghidra
12.1.4 disassembly listings, and execution on the reference runtime: fuzzed calls and shadow
verification during play. The words themselves stay in the user's `EL2M09.OVL`.

## Entry: `EL2M09:0f92` (module 9, handler `+20`)

Inputs: `AX` = kind, `CX:BX` = 32-bit seed, `SS:DI` = output. Output: a 0-terminated name at
`SS:DI`, with `BP` just past it. `[DS:07e0]`–`07ef` are pushed and popped around the work
(`EL2M09:0fc0`). Every random choice is a call of the stateless mixer `mix` (`DS:0128`,
[RNG.md](RNG.md)), which carries the seed forward in `CX:BX` and `SI:DI`. Names are therefore a
function of the kind and the seed only.

| Kind | Seed preparation | Name (observed) |
|---|---|---|
| 0–5 | `SI:DI = (BX:CX) + (CX:BX)`, rotated left 7 | A first word, then a last word. Kind 1, or an even `BL` for kinds 0/3/5: word `random(69h)`. Kind 2, or an odd `BL`: word `E0h + random(3Ch)`. The first word's terminator becomes a space. Kind 4 skips the first word |
| | | Last word (`EL2M09:1032`): when `BL` < 19h, word `4Fh + random(10)` + `s`, then `on` unless `BH` bit 3 is set. Otherwise word `59h + random(8Ah)` |
| 6–14 | `CX:BX += BX:CX`, `SI:DI = BX:CX` rotated left 7, added, rotated left 4, added | A last word into a 1Eh-byte stack buffer, published in `[DS:07e4]`. Then template word `EL2M09:0f61[kind] + random(EL2M09:0f73[kind])` |
| 15 | as 6–14 | two letters, `-`, three digits (`AB-123`) |
| 16 | as 6–14 | `+` and a space are written, then three digits **overwrite them**, then a letter (`123A`). That is what the original does |
| 17 and up | as 6–14 | two letters, ` * `, one digit (`AB * 1`) |

Letters are `'A' + random(26)` (`EL2M09:11a7`) and digits `'0' + random(10)` (`EL2M09:11b5`).

## The text expander: `1000:e969` (slot `DS:005c`)

`EL2M09:11c3` doubles `AX`, sets `ES:SI` to `EL2M09:00a3` and calls slot `DS:005c`. That copies
string `AX` of the table (`SI += ES:[SI + 2·AX]`, offsets relative to the table) to `SS:DI`:

| Byte | Effect |
|---|---|
| 01h–7Fh, 80h–FCh | copied |
| 00h | copied, then return (`DI` is past it) |
| FDh | writes `1Eh` and the next byte (a 1-byte control code) |
| FEh | writes `1Fh` and the next word (a 2-byte control code) |
| FFh | the next word names another string, expanded through slot `DS:0014` (recursive). **Not ported** |

## Port and proof

`frontier_game::names::name_port` is register- and memory-exact and returns the original's
instruction count. It declines when a copied word contains an `FFh` escape, and the original runs
in that case.

- **Fuzzing** (`crates/hooks/tests/galaxy_parity.rs`): 20,000 calls with kinds 0–31 and random
  seeds, in the booted game with `EL2M09` loaded through its module thunk. **14,274 compared,
  0 mismatches**, 5,726 declined (`FFh` escapes).
- **Shadow verification in play** (`rng_verify` example, 60 s with the galaxy map): 5 calls,
  0 mismatches.

## Open questions

- The nested string expansion (`FFh`, slot `DS:0014`): the main program's text system. It is
  needed for kinds 6–14 and for most of the game's text.
- What each kind is used for (planets, people, ships). The callers in `EL2M00` (`EL2M00:4db8`
  for system bodies, [GALAXY.md](GALAXY.md)) and the bulletin board give the answer.
