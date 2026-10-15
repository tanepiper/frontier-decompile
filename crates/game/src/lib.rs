//! Deterministic game logic ported 1:1 from `FRONTIER.EXE`: fixed-point maths, the RNG, galaxy
//! and system generation, orbital mechanics, ships, trade and missions.
//!
//! No rendering, windowing or sound output here (the sound driver's sequencer is, in [`adlib`];
//! synthesis is `frontier-opl`). Every ported routine cites its original address
//! (`/// Port of 1000:2f3a: …`) and numeric code is proven bit-exact against emulator ground truth
//! (see the `frontier-emulate` skill).
pub mod adlib;
pub mod math3;
