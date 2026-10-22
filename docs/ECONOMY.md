# System data and the economy

This covers what makes each star system different in trade terms: the per-system data in `EL2M06`,
and, as it is traced, the market built in `EL2M07` when the player arrives. Evidence: the Ghidra
12.1.4 disassembly listings, the coverage of a hyperspace jump ([COVERAGE.md](COVERAGE.md)), and
execution on the reference runtime (fuzzed calls against the original). The market builder itself
(`EL2M07:0949`) is ported with everything it calls.

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

## The market on arrival: `EL2M07:0949` (ported)

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

`market::builder_port` runs the whole of step 4 natively, as the sections below describe. For each
station it also sets `DS:2249` to the station object's dword `+76h` with its halves swapped. It
sets flag bit 4 unless the object's byte `+118h` has bit 5 set. The object comes through the
main program's lookup `CALLF [00b4]` (`1000:085a`): `[07ec]` + the word at `DS:06d6` + 2·index.
Parity (`market_builder_matches_original`). The test makes 300 whole builds for the current
system and random ones, with 0 to 4 stations and random generator states. **0 mismatches.**
Steps 1 and 2 are ported too, so module 7's handler `+0c` runs natively from start to end
(`market::handler_0c_port`).

## Handler `+0c`, the station count and the daily update (ported)

- **Handler `+0c` (`EL2M07:1551`)** saves `AX`–`DX`. With `AX` = 2 it counts the stations, shifts
  `DS:4603`, `4607`, `460b` up into `4607`, `460b`, `460f` (0 into `4603`), sets `DS:07c0` =
  `DS:208b` and runs the builder. With `AX` = 0Fh it runs the daily update unless byte
  `DS:2075` = 8. Other values return at once.
- **The station count (`EL2M07:0818`, callback `0831`)** walks the object block `[DS:56fb]`
  through the main program's iterator (`CALLF [0000]` = `1000:0cff`). The iterator visits the
  73h slot bytes from the top. A slot that is non-zero and has bit 5 clear gets its object from
  `1000:084f`, `[56f7]` + the word at `DS:06d6` + 2·slot. An object with bit 4 of `+118h` set is a
  station. Its byte `+5Dh` goes to `+7Ch` of the next station record, and `DS:2247` counts up to
  18.
- **The daily update (`EL2M07:0e1a`)**:
  1. It removes the first advert whose deadline is before the date + 4, and moves the later
     ones up.
  2. It makes up to three new adverts, each when `random(25)` ≤ `DS:43a1`.
  3. For each station:
     - Every price and stock moves to (new + 3·old)/4 in 32-bit arithmetic, from a new draw of
       the formulas.
     - When `random(7)` < `DS:2239`, a random event is cleared, with its once-only flag for
       9826h–9828h, and a new one ([`event_port`]) is written in its place.
     - `random(14)` picks a list entry to remove.
     - `random(9)` < `DS:223d`, `random(9)` < `DS:223f` and `random(27)` < `DS:2247` each add an
       entry, of kinds 0, 1 and 9.

Parity (`arrival_and_daily_update_match_original`). The test makes 400 calls: handler `+0c` with
`AX` = 2, 0Fh and 7, the daily update and the station count on their own, with random generator
states, dates up to 63 days ahead, the `DS:2075` = 8 case and random `DS:4603` dwords. **0
mismatches.** With one instruction miscounted in the daily update, the test fails.

## Price and stock: `EL2M07:0855`, `EL2M07:08c0` (ported)

Each commodity has an 8-byte block of coefficients at `CS:BP` (`EL2M07:02a9` + 8·k) and a flag
byte in `CL` (from the system record's `+0Ch` block). The system factors are word tables in
`EL2M07`, indexed by `DS:07bc`, `07c8` and `07c0` (set from the system data). All products are
Q15 (`IMUL` then doubled).

- **Price** (`market::price_port`):
  1. `p = [BP+2] · F[0261][07bc] · F[0251][07c8]`, negated if `CL` bit 3.
  2. `+ [BP+0]`, doubled if `CL` bit 7.
  3. `· F[0275][07c0]`, doubled with saturation to 7FFFh.
  4. `· (3E00h + random32 & 3FFh)`: a random factor of about 0.97–1.0.
  5. Doubled twice, saturating to `7FFF:FFFF`. The result is in `DX:AX`.
- **Stock** (`market::stock_port`):
  1. `s = [BP+6] · F[0251][07c8]`, negated unless `CL` bit 3.
  2. `+ [BP+4]`, `· F[0285][07bc]`, divided by 8 if `CL` bit 7.
  3. `· F[0299][07c0]` into `BX`.
  4. `DX = BX + |BX| · (4000h + random32 & 3FFFh)`, or 0 if that is negative.

Parity (`market_formulas_match_original`): 10,000 calls of each with random commodity blocks,
flags, factor indices and generator states, compared with the original. **0 mismatches.**

## Bulletin-board adverts: `EL2M07:0b03` (ported)

After the markets, the builder fills the bulletin board. It sets `DS:43a1` = 8·`DS:07c4` + 8 (from
the system data, `AL` = 2), `DS:43a5` = `DS:07d4` and `DS:43a3` = `DS:07d8`, and calls
`EL2M07:0b03` `DS:43a1` + 1 times. Each call makes at most one advert (`market::advert_port`,
*observed*):

1. `random32` picks one of 16 entries of the jump table at `EL2M07:059b`.
2. `DS:07d8` = `DS:43a3` − `DS:43a5`, or 0 when negative. Each kind needs it above a threshold
   (the "room", meaning *unknown*).

| Entries | Routine | Needs `DS:07d8` above | Advert |
|---|---|---|---|
| 0–7 | `0ce9` | – | string 9853h: one of 8 strings from 9957h |
| 8, 9 | `0cf4` | 10h | 9853h: one of 4 from 995Fh |
| 10, 11 | `0d0d` | 51h | 9853h: one of 4 from 9963h |
| 12, 13 | `0bb0` | 100h | 9856h: string 99A5h + 0–7, reward from `EL2M07:067b`; the target's record byte `+4` at least 3 |
| 14 | `0c19` | 271h | 9854h: 994Fh + 0–7, reward from `068b` |
| 15 | `0c81` | 510h | 9855h: 994Fh + 0–7, reward from `069b` |

- **Delivery and passenger adverts** (`EL2M07:0d1d`, *inferred* from the strings). The
  destination comes from module 2's nearby-system routine: a system within `random(3)` sectors
  of the current one (`DS:208b`). Its record (module 6, `AL` = 1) must have byte `+4` at least 3,
  and its allegiance (byte `+0Ah` >> 6) must equal the current system's (`DS:2243`). The deadline
  is the date `DS:1fc5` + 10 + `random(14)` days. The reward is
  50·(2·`r3` + the chosen string − 9957h + 3), with `r3` the sector range.
- **Military adverts** (`EL2M07:0b28`, *inferred*). The target is a system within `random(3)`
  sectors of the capital of allegiance `DS:2243`, from the three ids at `EL2M07:066f`. The deadline
  is `DS:1fc5` + 14 + (a `random32` draw & 3Fh) days. The reward comes from three 8-word tables
  (4,000–10,000, 10,000–17,000 and 15,000–25,000).
- **The record** (`EL2M07:0dd3`). The first free slot of the 25 slots of 18h bytes from `DS:43a7`
  (first dword 0) gets the string word, then the dwords `DS:07c0` (the system), `07c4` (the
  deadline), `07c8` (the reward), `07cc` (the text) and `07dc` (a `random32` draw). A zero dword
  follows (the next slot's first). When all 25 slots are used, nothing is written.

Parity (`crates/hooks/tests/market_parity.rs`, `adverts_match_original`). The test plays into
the cockpit, then loads modules 2, 6, 7 and 19. It makes 3,000 calls with random generator
states, room values, allegiances, partly filled tables, and the current system or a random one.
360 calls stored an advert of each of the four kinds. All were compared with the original:
**0 mismatches.** A port with one instruction miscounted fails the test.

## Station lists: `EL2M07:101a` (ported)

Each station has a record of 1DAh bytes from `DS:224d`. Its list of up to 14 entries of 4 bytes
starts at `+1A0h`, with the count at `+1D8h` (*observed*). The builder calls `EL2M07:101a` with
`DS:07d0` = the list's kind: (`DS:2247` − 1)/2 + 1 times for kind 9 (none when `DS:2247` is 0),
then 2·`DS:223d` + 1 times for kind 0 and 2·`DS:223f` + 1 times for kind 1. The daily update (`EL2M07:0e1a`) adds at most one of each. What
the lists hold is *unknown*; ships at the station (*inferred*) would fit.

1. A `random32` draw with bit 3 of `AL` set adds nothing. A full list (14 entries) adds nothing.
2. Otherwise `EL2M07:103c` picks the entry's first word. For kinds 0 and 1 the allegiance
   selects the row: `EL2M07:0567` + 4·(2·`DS:2243` + kind). For other kinds the row is
   `0567` + 4·kind. The row holds a scale and a table offset.
3. A `random32` draw gives `r` (`DS:07bc`). Kind 12 takes `DS:07cc` instead. For kind 1 with
   `DS:2239` below 4, `r` = `r`/2 + high(`DS:2239` · (`DS:07c0`/8)).
4. The entry is the table's word at 2·high(scale·`r`), and the second word is `r`.

Parity (`station_lists_match_original`). The test makes 4,000 calls for kinds 0, 1, 9 and 12, with
random allegiances, `DS:2239` values, list lengths and generator states. 1,734 calls added an
entry. All matched the original: **0 mismatches.**

## Station events: `EL2M07:0923` (ported)

Each station has up to 18 events of 16 bytes from `+7Eh`, counted in `+19Eh` (*observed*). The
builder calls `EL2M07:0923` 4·`DS:2239` + 1 times per station. A `random32` draw picks one of 32
entries of the jump table at `EL2M07:03a1`. The draw's `EDX` goes to `DS:07c0`. The writer
`EL2M07:1506` stores the string word, the words `DS:07c4`, `07c8` and `07cc`, and the dwords
`DS:07c0` and `07e4`. Strings `9819h`–`982Ch` are the event texts (*inferred*).

| Entries | Routine | Event |
|---|---|---|
| 0–4 | `12dc` | 9828h, once per station (flag bit 0 of `+7Dh`), when a commodity byte at `DS:2219` has bit 7 set. Its data is a dword of the station object (`ES:[obj+76h]` rotated left by 17), found through `CALLF [00b4]` (`1000:085a`) |
| 5 | `132b` | 9828h, once (bit 1), with `DS:07c4` = FFFFFFFFh |
| 6–9 | `139f` | 9826h for allegiance 2 (bit 2), 9827h for allegiance 1 (bit 3) |
| 10 | `10b2` | the newest non-zero dword of `DS:460f`, `460b`, `4607`, `4603` (cleared), then `111b` |
| 11, 12 | `118e` | a `random32` draw, then `111b`: 981Ch or 981Dh to a system one sector away, reward 40·(draw & 3 + 2) |
| 13–23 | `11ac` | up to four tries for a system within `random(3)` sectors whose record byte `+4` is at least 3: 9819h or 981Ah, rewards from `EL2M07:03e1`, `0401`, `0421` |
| 24–26 | `135e` | 9829h: the `random(12)`-th commodity with no stock, at twice its price (none if that overflows) |
| 27 | `14e4` | 982Bh or 982Ch, `DS:07c4` = 99C0h + draw & 7. This entry is reached only through the table |
| 28, 29 | `13cd` | up to four tries as in `11ac`: 981Eh–9821h, reward 1000·range + `EL2M07:06ab`[k], text 999Dh + `06ad`[k] |
| 30, 31 | `14d0` | 9822h–9825h |

Parity (`station_events_match_original`). The test makes 6,000 calls in the cockpit with random
generator states, station flags, event counts, commodity flags, stocks, the dwords at
`DS:4603`, allegiances, and the current system or a random one. 18 event strings were written.
All matched the original: **0 mismatches.**

## Open questions

- The meaning of each byte of the system record (economy, government, population, tech level are
  likely, *inferred*), and of the commodity tables in `EL2M07`.
- What the station lists (kinds 0, 1, 9) and the dwords at `DS:4603`–`460f` hold.
- What `DS:43a3` and `DS:43a5` (the "room" that gates each advert kind) measure.
