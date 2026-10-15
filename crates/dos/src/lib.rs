//! Reference runtime for Frontier: Elite II (DOS).
//!
//! Runs the user's own `FRONTIER.EXE` and `EL2*.OVL` files on an in-process 386 real-mode CPU with
//! the DOS, BIOS, EMS, mouse, timer, keyboard and VGA services the program uses. It serves two
//! purposes (docs/RUNTIME.md):
//!
//! 1. **Oracle.** Any original routine can be called with chosen inputs and its results compared
//!    with the native port, in `cargo test`, natively or in the browser.
//! 2. **Scaffold.** The playable build starts on the original code; native ports replace original
//!    routines address by address until nothing of the original runs.
//!
//! No game code or data is included: everything is read from the user's files at run time.

pub mod audio;
pub mod cpu;
pub mod devices;
pub mod machine;
pub mod mem;
pub mod png;
pub mod vfs;

pub use machine::{HookDevFn, HookFn, Image, Machine, Stop};
pub use vfs::Vfs;
