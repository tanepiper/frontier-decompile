# Audio

This covers how Frontier makes sound: the 11 sound vector slots, the three driver overlays, how
the AdLib driver drives the OPL2 chip, how the Roland driver uses the `.ROL` MIDI files, and the
runtime's OPL2 model and audio output. Evidence:

- **Static:** Ghidra 12.1.4 disassembly of `EL2ADLIB.OVL`, `EL2ROL.OVL`, `EL2SPK.OVL` and the IRQ 0
  handler in `FRONTIER.EXE` (`tools/ghidra/ExportDisasm.java`, `tools/ghidra/ExportRange.java`
  for code analysis did not reach), plus byte scans of every shipped file.
- **Observed:** the original `EL2ADLIB` running on the reference runtime ([RUNTIME.md](RUNTIME.md))
  against the OPL2 model in `crates/opl`, with every register write logged
  (`crates/dos/examples/audio.rs`).

Not yet done: comparing captured audio with DOSBox (see *Verification* below).

Addresses as in [OVERLAYS.md](OVERLAYS.md): `1000:xxxx` main program, `DS:xxxx` main data
segment, `EL2ADLIB:xxxx` offsets in the overlay's load image (CS-relative inside the driver).

## 1. The sound vector slots, `DS:032c`–`0357`

The chosen driver exports 11 far pointers into these slots ([OVERLAYS.md](OVERLAYS.md) §2). All
three drivers implement the same interface. Roles below are from `EL2ADLIB`; argument meanings
marked *inferred* come from how the driver uses them.

| Slot | `EL2ADLIB` | Role |
|---|---|---|
| `032c` | `0152` | Continuous engine/thrust sound. BX = volume (0–63), CX = pitch (*inferred*). Patches two bytes of the effect's instrument (`EL2ADLIB:66ea` level, `66f0` pitch) and starts effect `21h` if it is not running. It stops by itself (`22h`) 200 ticks (≈1 s) after the last call. Only while `DS:4636` ≠ 0 and no music is playing (`DS:53a3` < 0) |
| `0330` | `01b6` | Stop the music unless `DS:463f` ≠ 0 (falls through to `0354`) |
| `0334` | `01c0` | Initialise: reset the chip (`0920`), set `DS:53a3` = −1 (no song), set bit 7 of `DS:8920` (enables the tick) and clear the effect timers. Called once from `load_sound_driver` (`1000:ee82`) |
| `0338` | `01d2` | Play song AX. AX ≥ 0 plays song AX if music is on (`DS:463b`). AX < 0 picks a random song: up to 40 draws from `random(6)` or `random(7)` (main slot `DS:0124`) until `DS:4640 + n` ≠ 0, else the first enabled one. Records the song in `DS:53a3` and queues command `32h + n` |
| `033c` | `022a` | Shut down: reset the chip, clear bit 7 of `DS:8920`. Called at exit (`1000:07e5`) |
| `0340` | `0233` | Sound effect AX (game numbering). Mapped through the table at `EL2ADLIB:0095` to a driver command; `FFh` entries are silent. Effect `14h` plays only if `DS:463a` ≠ 0. Needs `DS:4636` ≠ 0 |
| `0344` | `0280` | Effect BL (0–14) at the volume from `EL2ADLIB:00c7` (`32h` … `01h`, falling) through `0348`. Only when the dword `DS:5709` ≠ 0 |
| `0348` | `025b` | Effect AX with a distance/volume in BX (`BX >> 4`, *inferred*). Same mapping as `0340` |
| `034c` | `029e` | **Tick.** Called from the IRQ 0 handler, see §2. Re-entry guarded by `EL2ADLIB:0092` |
| `0350` | `033b` | Silence everything: queue command 0, clear the effect timers |
| `0354` | `036f` | Stop the music: queue command `23h`, `DS:53a3` = −1 |

Call sites (byte scan for `lcall [slot]`): the main program calls `0330`, `0334`, `0338`, `033c`,
`0340`, `0344` (11 sites), `0348`, `034c`, `0350`, `0354`. The overlays call them too: `EL2M03`
(flight: 15 × `0340`, `032c`, `0330`, `0338`, `0348`), `EL2M08` (title: `0338`, `0330`), `EL2M10`,
`EL2M11`, `EL2M12`, `EL2M14`, `EL2M15`, `EL2M05`, `EL2M07`, `EL2M16`. `DS:4636`, `463a`, `463b`, `463f` and
`4640`–`4646` look like the in-game sound options (effects on, music on, per-song enables;
*inferred*, the options screen is not traced).

## 2. Timing

`1000:ce5e` programs PIT channel 0 with reload `170Bh` (5899): IRQ 0 runs at
1,193,182 / 5899 = **202.27 Hz**. The handler (`1000:ceae`) calls the sound tick through
`DS:034c` on every interrupt while bit 7 of `DS:8920` is set, then chains to the BIOS handler
every time a `170Ah` accumulator at `DS:09fa` carries (≈18.2 Hz, so the BIOS clock keeps time).

Inside the tick (`EL2ADLIB:029e`):

1. The effect timers at `EL2ADLIB:00bd`–`00c5` count down once per interrupt. When one reaches
   zero the driver queues the matching "stop" command (`19h`, `0Eh`, `0Ch`, `1Eh`, `22h`).
   Effects `18h`/`1Dh` run 400 ticks (1.98 s), `0Dh`/`0Bh` 600 ticks (2.97 s).
2. A fractional divider at `EL2ADLIB:0093` subtracts 327 and adds 910 on borrow, so the
   sequencer runs on 327 of every 910 ticks: **72.68 Hz**. Then:
   `08b3` drains the command queue, `0974` steps all 9 voices, `1494` advances the tempo counter.

## 3. `EL2ADLIB`: an OPL2 sequencer with its own music

`EL2ADLIB.OVL` is 27,101 bytes. Its code ends near `EL2ADLIB:17dd`; the remaining ~20 KB is the
music and effect data. **The AdLib driver never opens a file**: no `int 21h` after its TSR stub,
and it does not read the `.ROL` files. The AdLib music is embedded in the overlay.

| Address | Role |
|---|---|
| `EL2ADLIB:089f` | Queue a command byte (16-entry ring at `059c`, write index `0598`) |
| `EL2ADLIB:08b3` | Drain the queue. Command n → pointer table at `CS:[1871]` (= `2229h`) → a stream whose first word is the voice (low byte) and priority (high byte). The command starts on that voice only if its priority ≥ the voice's current priority (`DI+08h`) |
| `EL2ADLIB:084a` | 10 voice records of `43h` bytes at `05ac`, `05ef`, … `0807` (voices 0–8 plus a spare) |
| `EL2ADLIB:085e` | Voice → operator register offset: `00 01 02 08 09 0A 10 11 12` (the standard OPL2 channel map) |
| `EL2ADLIB:0867` | 12 F-numbers for one octave: 308, 327, 346, 367, 388, 412, 436, 462, 489, 519, 549, 582 |
| `EL2ADLIB:0920` | Chip reset: `01h` = `20h` (waveform select on), `08h` = 0, `BDh` = 0 (melodic mode), all levels `3Fh` (silent) |
| `EL2ADLIB:0974` | Per-voice sequencer step |
| `EL2ADLIB:1304` | Note → F-number and block (`A0h`/`B0h`), with pitch bend/vibrato from tables at `0382` |
| `EL2ADLIB:13cc`, `1404` | Key on (`B0h` bit 5) / key off |
| `EL2ADLIB:14da`, `1514`, `1554` | Operator levels (`40h`/`43h`): instrument level + voice volume + effect volume, clamped to 0–63 |
| `EL2ADLIB:15d3` | Load an instrument: 11 bytes → `20h`, `23h`, `C0h`, `E0h`, `E3h`, `40h`, `43h`, `60h`, `63h`, `80h`, `83h` |
| `EL2ADLIB:164e` | Silence a voice (fastest release, key off) |
| `EL2ADLIB:1472` | Register write: index to `388h`, 4 reads of `388h` as a delay, data to `389h`, 23 reads of `388h` |
| `EL2ADLIB:17cb` | 16-bit pseudo-random step (`+9248h`, rotate right 3) for random note variation |

Command numbers: `00h` silence, `01h`–`23h` effects (`21h`/`22h` engine start/stop, `23h` stop
music), `32h`–`3Dh` songs (12 entries; `32h`–`38h` are the 7 songs `0338` picks at random).

**Chip use (observed and static agree):** only ports `388h`/`389h`; only OPL2 registers (no
`105h`, no second bank, so OPL2, not OPL3); waveform select enabled; melodic mode (`BDh` = 0 at
reset, and no write sets the rhythm bits in 8 s of intro). The status port is read only as a delay;
the **timers are not used** and the driver does not detect the card.

## 4. `EL2ROL`: MT-32 MIDI from the `.ROL` files

`EL2ROL.OVL` (6,614 bytes) is a MIDI player for a Roland LAPC-1 / MT-32 on an MPU-401:

- Ports `330h` (data) and `331h` (status/command), polled (`EL2ROL:155f`–`15c2`); UART mode
  (*inferred* from the byte stream it writes).
- At initialisation (`EL2ROL:1078`) it reads **nine** files, named in the overlay: `hallking.rol`,
  `elitet.rol`, `babayaga.rol`, `barem.rol`, `jupalt.rol`, `valkries.rol`, `bluedan.rol`,
  `ggofkiev.rol`, `intro.rol`. Each is loaded whole into EMS (`int 67h`, `EL2ROL:1604`).
  Despite the extension they are **Standard MIDI Files** (`MThd`), not AdLib Visual Composer
  `.ROL`.
- It sends two SysEx blocks at start (`EL2ROL:15c5`, from `0fb2` and `0fad`), *inferred* to be
  MT-32 set-up. Music uses MIDI channels 1–7; effects play as short sequences on channels 8–10
  (`B7h`–`B9h` controller writes at `EL2ROL:1223`, `133f`), so they depend on the MT-32's sounds.

The songs are different arrangements from the AdLib ones: separate data written for a
different synthesiser, not a conversion of the same files.

`EL2SPK.OVL` (2,182 bytes) plays effects only, on the PC speaker through PIT channel 2 and port
`61h`.

## 5. The reimplementation

Per [DEVIATIONS.md](DEVIATIONS.md) the game has **one audio output** and no driver choice. The
reference is the AdLib driver, the only one whose music and effects come entirely from the
original files. The runtime runs `EL2ADLIB` unchanged against an OPL2 model. The sound selection
`DS:8920` = 2 is supplied by `setup_skip`.

### OPL2 model (`crates/opl`)

An original Rust implementation of the YM3812 as documented by datasheets and published die
analyses. No emulator source was used. It runs at the chip's own rate, 3,579,545 / 72 =
49,716 Hz, and models the log-sine and exponent ROMs (generated from their defining formulas),
19-bit phase accumulators, the multiplier table, vibrato (8 steps, 6.1 Hz), tremolo (210 steps,
3.7 Hz, 1.0 or 4.8 dB), the envelope generator's global counter and per-rate step patterns,
exponential attack, KSL and KSR, the four OPL2 waveforms, feedback, both connections, rhythm mode
with the noise generator, and the two timers with the status register. Output is the 16-bit
digital sum; the YM3014 DAC's quantisation is not modelled.

### Runtime wiring (`crates/dos/src/audio.rs`)

Ports `388h`/`389h`, and the Sound Blaster mirror `228h`/`229h`. Before every access the chip is
advanced to the current virtual time, so each write lands on the sample it would have hit on
hardware, to within one sample (20 µs). Samples depend only on executed instructions, so a run is
reproducible bit for bit (`crates/dos/tests/audio.rs`).

### Output (`apps/game/src/audio.rs`)

One mixer, stereo at the device rate. A windowed-sinc resampler (64 taps, Kaiser window, cut-off
at 91 % of the output Nyquist frequency) converts 49,716 Hz to 44.1/48 kHz without aliasing. The
ratio is nudged by up to ±0.5 % to keep about 80 ms queued. Natively the backend is cpal
(CoreAudio, WASAPI, ALSA); in the browser it is WebAudio, with stereo buffers scheduled back to
back on one `AudioContext`, resumed by the first click or key press.

### Observed on the runtime

`cargo run --release -p frontier-dos --example audio -- game-data/original 30` (30 virtual
seconds of boot and intro):

| t (s) | Event |
|---|---|
| < 0.01 | `exec EL2ADLIB.OVL` between `EL2MATH3` and `EL2MCGA3`, as in `startup_init` step 7 |
| < 0.01 | Reset writes (from sample 2 of the chip's output) `01h`=`20h`, `08h`=0, `BDh`=0, then `3Fh` to every level register |
| ~4–20 | The intro music: key-ons from 4 s, with level rising steadily from 4 s to 20 s (a fade-in, *inferred*) |
| 30 | 5,020 register writes; peak sample ±4,566 |

With the 27 KB driver resident, the module loader now runs out of conventional memory one
module earlier: `EL2M17` first fails with DOS error 8 and loads after an eviction. The original's
loader is built for this ([OVERLAYS.md](OVERLAYS.md) §4).

## Verification

- `crates/opl` unit tests: ROM end points, sine pitch and peak, release to silence, timer status,
  determinism; resampler gain, rate and alias rejection.
- `crates/dos/tests/audio.rs` (skips without `game-data/`): the driver loads, its reset sequence
  is the first thing written, more than 50 key-ons in the first 8 s, non-silent output, and two
  runs give identical register logs and sample hashes.
- **DOSBox comparison: not done yet.** DOSBox Staging records its mixer output with a hotkey only,
  and that could not be automated here. To capture a reference: start `tools/dosbox/run.sh`, answer
  **2**, and toggle audio recording around the intro with the *Record audio* hotkey (WAV lands in
  the `capture/` folder). Then compare with `game-data/derived/audio/boot.wav` from the example
  above. Expect the same notes and timbres; timing differs slightly because DOSBox runs at a fixed
  25,000 cycles and the runtime at 25 M instructions per second.

## Higher-quality options (not pursued without a decision)

| Option | What it would take | Trade-off |
|---|---|---|
| **MT-32 through `EL2ROL`** | An MPU-401 UART on `330h`/`331h` plus an MT-32 emulation (an LA-synthesis implementation and the user's MT-32 control and PCM ROMs) | The richest original sound and the game's own MT-32 arrangements. Needs ROM files the user must supply; a large new synthesiser; effects become MIDI sequences, so they sound different |
| **General MIDI rendering of the `.ROL` MIDI files** | A GM synthesiser and a SoundFont | Modern-sounding, but **not 1:1**: the songs were arranged for MT-32 patches and SysEx, and GM instruments would be substitutes. Needs a SoundFont download and a DEVIATIONS.md row |
| **OPL2 (current)** | Done | Exactly what an AdLib/Sound Blaster player heard, from the original files alone |

## Open questions

- The meaning of `DS:4636`, `463a`, `463b`, `463f`, `4640`–`4646` (sound options) and of
  `DS:5709`.
- The voice record layout (`43h` bytes) and the sequence byte code at `EL2ADLIB:0974`. These are
  needed for a native port of the sequencer, with the original as the parity oracle (identical
  register write logs).
- Whether the intro's fade-in is in the song data or driven by the game (`032c`/`0348` volumes).
