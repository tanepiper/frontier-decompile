//! Print the `0A` text commands of a main-table model with the matrix the walker gives them
//! (for checking `1C` against the original). Usage: text_matrix GAME_DIR MODEL
use frontier_formats::model;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let exe = std::fs::read(format!("{}/FRONTIER.EXE", a[1])).unwrap();
    let (image, table) = model::main_table(&exe).unwrap();
    let n: usize = a[2].parse().unwrap();
    let off = table.models[n].unwrap();
    let h = model::header(&image, off).unwrap();
    let mut p = model::Paint::new(std::sync::Arc::new(model::sine_table(&image)));
    p.object.copy_from_slice(&model::SHOWN_SHIP);
    let m = model::walk_with(&image, off + h.mesh as usize, model::Branches::Near, &mut p);
    for (prim, _) in m.prims {
        if let model::Prim::Text { at, matrix, .. } = prim {
            println!("text at {at:#x}: {matrix:.3?}");
        }
    }
}
