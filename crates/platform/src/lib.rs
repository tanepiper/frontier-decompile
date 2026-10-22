//! Host services shared by the `frontier` game and the `frontier-viewer` resource browser, so
//! neither app carries its own copy: the audio output (OPL2 samples to the sound card or
//! WebAudio), finding and loading the player's own game files (a folder natively; `?data=` or
//! the file picker in the browser), and the browser page glue.

pub mod audio;
pub mod files;
#[cfg(target_arch = "wasm32")]
pub mod web;
