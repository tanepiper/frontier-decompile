# Star-system generation (`EL2M00`)

This covers the routines in `EL2M00` that build a star system's bodies when the player arrives:
the system seed, the body records, the classifiers that give each body its type, the adders for
small and large bodies, and naming. Evidence: the code coverage of a hyperspace jump
([COVERAGE.md](COVERAGE.md): `EL2M00:4990`–`4e3f` run only then), the Ghidra 12.1.4 disassembly
listings, and execution on the reference runtime (fuzzed calls against the original). Not
traced yet: the orbit builder `EL2M00:4e3f`, which calls `468a` for each body. See *Open questions*. Names of fields are *inferred* from how the code uses them.

## The system seed

`DS:2021` (`s0`) and `DS:2025` (`s1`). One step (`EL2M00:4df5`):

`s1 += s0; s0 = rol(s0, 3) + s1; s1 = rol(s1, 5); s0 += s1`.

This is the sector seed's step ([GALAXY.md](GALAXY.md)) with one more addition. The new values
are left in `EAX` and `EBX`.

## Body records

40h bytes each in `ES`, the current one at `ES:SI` and the next at `SI + 40h`.

| Offset | Written by | Meaning (*inferred*) |
|---|---|---|
| `+00h` | adders: `0039_0000h`; `4bda`: 0 | Kind and flags |
| `+02h` | `4bda`: 38h; `4abb` caps it at 47h | |
| `+04h` | adders: a seed dword | The body's own seed (its name is made from it) |
| `+08h` | adders: `DS:204b` | Parent index |
| `+0Ah`, `+0Ch` | classifiers; `4c32` (`EL2M00:3e39`) | Class and type (model number, 7Eh–8Eh) |
| `+0Eh`, `+10h` | `4d10`: 7FFFh, 22h | A float (mantissa, exponent): the parent's mass scale |
| `+12h`, `+1Eh` | `4c32` (`EL2M00:3e99`/`3e9b`) | |
| `+14h`, `+16h` | `4d10` | A float: the body's size |
| `+20h` | classifiers | Temperature (the habitable band is about D8h–139h) |
| `+22h`… | `4db8` | Name, 0-terminated |
| `+3Bh` | `4db8` sets bit 7 | "Named" |

Counters in the main data segment: `DS:204d` counts the bodies (at most 12h = 18), `DS:204e` the
large ones, `DS:202d` is a running index, `DS:204b` the parent index, and `DS:2049` is the
current body's parent value that the classifiers branch on.

## Routines

All are near routines (`RET`), ported in `frontier_game::system`. `4bad`, `4bff`, `4c27` and `4c2c`
are reached only through the classifier table at `EL2M00:0213`, so Ghidra had not made them
functions. `tools/ghidra/DisasmAt.java` makes them.

| Address | Port | What it does |
|---|---|---|
| `EL2M00:4df5` | `sys_step_port` | The seed step above (10 instructions) |
| `EL2M00:4e29` | `lookup_3c39_port` | `BH` ≥ 40h: `AX` = FFFFh; else `AX` = `EL2M00:3c39[(BX >> 5) & 1FEh]` |
| `EL2M00:4db8` | `body_name_port` | Name the body: module 9's handler `+20` ([NAMES.md](NAMES.md)) with kind `AX` and the body's seed, to `SS:FC08h`, then copied to `+22h` with `STOSB`. Declines (original runs) if `EL2M09` is not resident |
| `EL2M00:4bda` | `body_4bda_port` | Reset a body (`+0` = 0, `+2` = 38h); name it with kind 10 when the temperature is at least DCh and the parent value above 32h |
| `EL2M00:4c32` | `moons_port` | Up to 4 small bodies: count = high word of `BX × low16(s1)`, at most 3, plus one. A private copy of the seed in `DS:07c4`/`07c8` is stepped like `random(n)`. Template `EL2M00:1d63`, kind-6 names, type from `EL2M00:3e39` |
| `EL2M00:4d10` | `big_body_port` | One large body: template `19e3`, `1da3` or `1c63` by `AX`; size `sqrt((7FFFh·2^22h)² / parent) × 5EDBh` through the native `EL2MATH3` ports (`mul`, `div_norm`, `sqrt_float`); kind-7 name |
| `EL2M00:4990` | `classify_4990_port` | The temperate classifier: hot (above 139h: type 84h, warmed by `DS:07bc`/2), habitable band (seed-dependent: type 8Ah, 8Ch or 8Eh, kind-9 name, small and large bodies by parent value), warm (AAh–D7h: type 84h or the 8Ah path), cool (42h–7Bh: type 86h), else [`4abb`] |
| `EL2M00:4abb` | `classify_4abb_port` | The cold classifier: temperature `+= 14h`; type 88h for 111h–139h under a heavy parent, otherwise type 7Eh, with names and small and large bodies by thresholds |
| `EL2M00:4b69` | `classify_4b69_port` | Temperature 96h–1F4h: kind-10 name, small bodies from a parent value of 2Dh, a large body from 96h |
| `EL2M00:4bad` | `classify_4bad_port` | At least DCh hot under a parent value above 32h: kind-10 name; small bodies from 50h |
| `EL2M00:4bff` | `classify_4bff_port` | From the third body on: type AEh, class 32h, `+18h` = 3E2h, temperature 2C09h, `+02h` − 1 (a white dwarf, *inferred*) |
| `EL2M00:4c27`, `4c2c` | `classify_4c27_port`, `classify_4c2c_port` | One or two system-seed steps; a negative `s0` (the `SF` that `4df5` leaves) continues in `4bff` |
| `EL2M00:468a` | `setup_body_port` | Set up a body from its mass `DS:07bc`: the class is the first threshold in `EL2M00:00ff` above the mass (1, 5, 12, 50, 200, …, `0103_6640h` for stars, …). It selects `+18h`, `+0Ah`, the temperature, `DS:07e4`, the classifier `DS:07e8` and the type `+0Ch` from the tables at `01b7`, `00a3`, `0189`, `015b`, `0213`, `00d1`. Type 0 returns with ZF set. Stars are named after the system (`[DS:2029]`), with a letter for a companion. Other bodies get their own kind-10 name when the parent is named, otherwise the parent's name plus a number or letter (`DS:2031`–`2033` by `DS:2034`). Then `47d8` |
| `EL2M00:47d8` | `finish_body_port` | Mass `DS:07c0` × `DS:07c8` as a float, radius from the mass and the parent, seed, parent index, orbit `DS:07bc` as a float, fields from the seed. Below a temperature of 3E8h, it adds the heat of every body up the parent chain (records at `[DS:2041]:[DS:203f]`, their `+18h`, summed in float) to the temperature. Then `DS:204b` = `DS:202d` and `CALL [DS:07e8]` (one of the classifiers above). Returns `AX` = FFFFh, ZF clear |

## How parity was proven

`crates/hooks/tests/galaxy_parity.rs`, in the booted game with `EL2M00`, `EL2M09` and the maths
overlay resident: 20,000 calls of each routine with a random body record at `B000:0100`,
temperatures 0–23Fh, parent values 0–13Fh, random seeds, body counts up to the limit of 18 and
parent indices into the records. Every register, every byte written (the records, the counters,
the seeds, the names) and the instruction count are compared, through the nested calls (the
adders, the namer, the name generator, the maths routines). For `468a` and `47d8`, ZF and SF are
compared too, because `4e3f` tests ZF after the call. The body-setup cases build a parent chain
of records whose parent indices point strictly backwards. A chain that loops never ends in the
original, so the port gives up after 256 links and leaves such a call to the original.
**All match.**

## Open questions

- The orbit builder `EL2M00:4e3f` (663 bytes), which calls `468a` per body. It saves its whole
  working set (`DS:07c0`–`07e4`, the seeds, `DS:2031`) before generating children, so it is
  recursive. Also open: the handler that starts a system on arrival.
- What `+02h`, `+0Ah`, `+12h` and `+1Eh` hold. The renderer and the system map read them.
