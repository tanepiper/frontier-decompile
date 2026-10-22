//! For each face of a main-table model, the stored normal against the normal from its corners
//! (for finding faces whose stored normal disagrees). Usage: face_normals GAME_DIR MODEL
use frontier_formats::model::{self, Prim};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let exe = std::fs::read(format!("{}/FRONTIER.EXE", a[1])).unwrap();
    let (image, table) = model::main_table(&exe).unwrap();
    let n: usize = a[2].parse().unwrap();
    let off = table.models[n].unwrap();
    let h = model::header(&image, off).unwrap();
    let pos = model::resolve(&model::vertex_defs(&image, &h).unwrap());
    let normals = model::normals(&image, &h);
    let mut p = model::Paint::new(std::sync::Arc::new(model::sine_table(&image)));
    p.object.copy_from_slice(&model::SHOWN_SHIP);
    let m = model::walk_with(&image, off + h.mesh as usize, model::Branches::Near, &mut p);
    for (prim, c) in m.prims {
        let (v, k): (Vec<u8>, u8) = match prim {
            Prim::Tri(v, k) => (v.to_vec(), k),
            Prim::Quad(v, k) => (v.to_vec(), k),
            _ => continue,
        };
        let pts: Vec<[f32; 3]> = v.iter().filter_map(|&i| pos.get(i as usize).copied().flatten()).collect();
        if pts.len() < 3 {
            continue;
        }
        let (a, b, cc) = (pts[0], pts[1], pts[2]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [cc[0] - a[0], cc[1] - a[1], cc[2] - a[2]];
        let g = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let s = normals.get(k as usize).copied().flatten();
        let dot = s.map(|s| s[0] * g[0] + s[1] * g[1] + s[2] * g[2]);
        // Also: is the stored normal pointing out (same side as the face centre from the origin)?
        let ctr: Vec<f32> = (0..3).map(|i| pts.iter().map(|p| p[i]).sum::<f32>() / pts.len() as f32).collect();
        let out = s.map(|s| s[0] * ctr[0] + s[1] * ctr[1] + s[2] * ctr[2]);
        println!("{v:?} normal {k} colour {c:03x} stored {s:?} geometric·stored {dot:?} out {out:?}");
    }
}
