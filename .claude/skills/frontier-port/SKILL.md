---
name: frontier-port
description: Port traced Frontier: Elite II behaviour into the Rust reimplementation — data readers in crates/formats, deterministic logic in crates/game, rendering/input/app flow in apps/game. Use when implementing a feature whose original behaviour has been traced so the code mirrors the original 1:1.
---

# Port to Rust, 1:1

Precondition: the behaviour is traced (`frontier-trace`). If you are inventing behaviour, stop and
trace it.

| Kind | Location | Constraints |
|---|---|---|
| File/format readers | `crates/formats` | No dependencies; corpus test that skips without `game-data/` |
| Maths, RNG, generation, simulation | `crates/game` | No wgpu/winit/audio; deterministic; bit-exact where emulated |
| Rendering, input, screens, app loop | `apps/game` | Must also build for `wasm32-unknown-unknown` (no threads, no blocking I/O) |

- Name things after what the original does, and put the source address on each ported routine:
  `/// Port of 1000:2f3a (fixed-point multiply).`
- Keep constants with their source address; don't "tidy" magic numbers.
- **No legacy hardware setup.** Skip anything that only configured the 1993 PC (sound card
  prompt, driver overlay choice, memory checks): boot goes straight into the game. See
  `docs/DEVIATIONS.md`; add a row there for any new deviation.
- Mark deliberate differences with `// Deviation:` and a reason (e.g. 320×200 palette framebuffer
  scaled by the GPU instead of VGA hardware).
- The original renders a 320×200 8-bit palettised frame. Reproduce that frame in software first,
  present it with wgpu, so captures can be compared pixel for pixel with DOSBox.
- Smallest end-to-end slice first; build often (`cargo build --release -p frontier-app`, and the
  WASM target), then verify and land per slice.
