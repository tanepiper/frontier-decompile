# Galaxy generation

This covers how the galaxy map module `EL2M02` decides which stars a sector holds: the density map
in `EL2M19`, the density function, the sector seed and the sector star list, and how the native
ports were proven. Evidence: the Ghidra 12.1.4 disassembly listings
(`tools/ghidra/ExportDisasm.java`), and execution on the reference runtime
([RUNTIME.md](RUNTIME.md)): fuzzed calls and shadow verification while the galaxy map is open.
Not traced yet: how a star's system (its planets, type and name) is generated from the star
list, and the star records' meaning beyond position and kind. See *Open questions*.

Addresses: `EL2M02:xxxx` and `EL2M19:xxxx` are offsets in those overlays' load images (Ghidra
`1000:xxxx`). `DS:xxxx` is the main data segment ([OVERLAYS.md](OVERLAYS.md)).

## Coordinates

The galaxy is **8192 × 8192 sectors**, 13-bit coordinates `x` and `y`. A 32-bit system id packs
`y` from bit 19 up, `x` in bits 6–18 and the star's index in the sector below that. This is
**observed** in `EL2M06:3f8f` and `EL2M02:3e6c`, which unpack it. Sol's sector is
`(1718h, 1524h)` = (5912, 5412): the map prints sectors relative to it (`EL2M02:2314`), and
`EL2M06` measures distances from it.

## The density map (`EL2M19`)

`EL2M19` is almost all data. Its only code is handler `+20` (`EL2M19:4126`), three
instructions that return the word at `CS:[BX+00a3h]` in `AX` and the word at `CS:[BX+0123h]` in
`DX`: a 2×2 block of bytes from a map 128 bytes wide starting at `EL2M19:00a3`. The map runs to
the handler, 4082h bytes (128 rows of 128 and two more bytes). One map cell covers 64 × 64
sectors. The map is read from the user's file at run time; nothing of it is in this repository.

## Density of a sector: `EL2M02:3b34`

Ported as `frontier_game::galaxy::sector_density_port`. Inputs `AX` = x, `BX` = y, `CX` =
noise selector. A near routine. A second entry at `EL2M02:3b2f` sets `CX = [DS:2199] − 1` and
falls into it.

1. `x` or `y` ≥ 2000h: density 0.
2. Map index `((y·8 & FE00h) + (x·8 >> 7)) >> 2`. The four corner bytes come through module slot
   19's handler `+20`, so `EL2M19` is loaded on first use, and the lock byte `DS:05a9` is held
   around the call.
3. Bilinear interpolation by the position inside the cell (`x·8 & 1FFh` and `y·8 & 1FFh`,
   shifted left 6, as signed 16-bit fractions). Signed 16 × 16 products are accumulated in
   `DS:07c0`. `AX` gets bits 8–23 of the result.
4. `CX` ≥ 12h: done. Otherwise hashed noise `N = EL2M02:1b92[((xfrac + AX) xor SI) >> 4 & FEh]`
   (128 words). With `CX` = 0, `AX = AX × N >> 16`. Otherwise
   `AX = AX × not(N × EL2M02:1c90[CX] >> 16) >> 16`.

## The sector seed: `EL2M02:3abe`

Ported as `seed_step` / `seed_step_port`. Two 32-bit words, `A` = `DS:2155:2153` and `B` =
`DS:2159:2157`. One step:

`B += A; A = rol(A, 3); A += B; B = rol(B, 5)`, returning the new `A` in `DX:AX`.

## The star list of a sector: `EL2M02:3c2f`

Ported as `sector_stars_port`. For the sector at `DS:2149` (x) and `DS:214d` (y), it fills the
caller's frame at `SS:BP`.

- **Catalogue sectors.** 47 hand-made sectors are listed as `x, y` word pairs at
  `EL2M02:00a2`–`015d`. Each has a range of stars (`EL2M02:0162`), star records (`0246`,
  byte-swapped words) and extras (`041a`). Their stars are copied, not generated.
- **Generated sectors.** The star count is the high byte of `sector_density(x, y, 0)` shifted
  right 2 (`density >> 10`), at most 62. The seed starts at `A = (x << 16) | y` and
  `B = (y << 16) | x`, and is stepped three times. Each star then takes one step: its record at
  `[BP+FAh]` + 4·n (after a `0900h, 0` header) is `DX` = `A.hi` with `DH` = 9, then `AX` =
  `A.lo >> 1` (arithmetic) with its low byte halved again. Its kind bytes at `[BP+7Ch]` + 2·n
  are `EL2M02:1cb2[B.lo & 1F]` and `EL2M02:1cd2[B.hi & 1F]`.
- `[BP+7Ah]` = stars + 1 (0 for an empty sector), `[BP+5Ah]` = 4. The incoming registers are saved
  at `[BP+14h]`–`[BP+1Eh]` and the sector at `[BP+76h]`/`[BP+78h]`.
- When bit 7 of `DS:215b` is set (always, on the map), the list is handed to the renderer:
  `[DS:07e4]` = list, `CALLF [DS:00f0]`. The port does the same set-up and ends by entering that
  routine with the original's return address pushed, so the original's tail runs next. It
  declines the empty catalogue sectors that name an extra model (`EL2M02:1cf2`), which the
  original then draws through `DS:00f4`.

## How parity was proven

1. **Fuzzing** (`crates/hooks/tests/galaxy_parity.rs`). In the booted game, with `EL2M02` and
   `EL2M19` loaded through the game's own module thunks: 20,000 calls of `seed_step` and
   `sector_density`, and 5,000 of `sector_stars`. A third of the `sector_stars` sectors come
   from the catalogue, and half have the renderer flag set. Every register, every byte written
   and the instruction count are compared, up to where the port stops (its return, or its entry
   into the renderer). All match.
2. **Shadow verification in play**
   (`cargo run -p frontier-hooks --example rng_verify -- … 60 32:3c 38:3c 44:3c 50:3c`, F2 four
   times in the cockpit, which opens the galaxy map). **9,691 calls verified, 0 mismatches.**
   1,211 calls were left to the original (catalogue sectors with an extra model), and one
   density call ran while `EL2M19` was not yet resident.

## Open questions

- From a star in the list to its system: the per-star seed, the star type, the name syllables,
  the planets. Handler `+20` (`EL2M02:3ddd`) and `EL2M02:2537` (system set-up through `EL2M06`
  and `EL2M16`) are the next entry points ([RNG.md](RNG.md) *Open questions*).
- What the star records' fields mean beyond position (the `09h` byte, the two kind bytes).
- The density map as a picture (the resource viewer can show `EL2M19:00a3` as a 128 × 128
  grey image).
