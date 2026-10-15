//! `frontier`: the reimplementation's entry point. Rendering (wgpu, native and WebGPU/WebGL2)
//! arrives once the boot flow is traced; for now it validates the user's game data.

use std::path::PathBuf;

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("game-data/original"));
    let exe = dir.join("FRONTIER.EXE");
    let data = match std::fs::read(&exe) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("frontier: cannot read {}: {err} (run tools/extract-assets.sh)", exe.display());
            std::process::exit(1);
        }
    };
    match frontier_formats::mz::parse(&data) {
        Ok(mz) => println!(
            "FRONTIER.EXE: {} byte image, {} relocations, entry {:04x}:{:04x}",
            mz.image_size(),
            mz.relocations.len(),
            mz.initial_cs,
            mz.initial_ip
        ),
        Err(err) => {
            eprintln!("frontier: {}: {err}", exe.display());
            std::process::exit(1);
        }
    }
}
