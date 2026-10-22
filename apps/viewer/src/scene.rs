//! A model and its sub-objects as one list of shapes in the model's space, and drawing it.
//!
//! Deviation: this is the viewer's picture of the geometry with its own orthographic camera,
//! flat shading and depth sort. Thruster flames are drawn only when asked for. The game's renderer (`1000:b207` and EL2MCGA3) is not used:
//! conditional commands are all followed (every level of detail at once), sub-objects keep the
//! parent's orientation, and discs and thruster flames get a fixed screen size.

use egui::{Color32, Pos2, Stroke, Vec2};
use frontier_formats::model::{self, Prim};

use crate::library::ModelSet;

pub enum Shape {
    Face(Vec<[f32; 3]>, u16),
    Line([[f32; 3]; 2], u16),
    Thruster([[f32; 3]; 2], u16),
    Disc([f32; 3], u16),
    Label([f32; 3], String),
}

#[derive(Default)]
pub struct Scene {
    pub shapes: Vec<Shape>,
    pub points: Vec<[f32; 3]>,
    /// Sub-objects drawn, by model number.
    pub parts: Vec<u16>,
}

/// Flatten model `n` of `set`. Sub-objects (`0E`) are placed at their vertex and scaled by
/// 2^(their header scale − the parent's), since coordinates are shifted by `17h − scale`
/// (`EL2MATH3:0d58`, *inferred* for sub-objects); depth is limited to 4.
pub fn build(set: &ModelSet, n: usize) -> Scene {
    let mut s = Scene::default();
    add(set, n, [0.0; 3], 1.0, 0, &mut s);
    s
}

fn add(set: &ModelSet, n: usize, origin: [f32; 3], scale: f32, depth: u32, s: &mut Scene) {
    let Some(Some(off)) = set.table.models.get(n).copied() else { return };
    let Ok(h) = model::header(&set.image, off) else { return };
    let defs = model::vertex_defs(&set.image, &h).unwrap_or_default();
    let pos = model::resolve(&defs);
    let at = |v: u8| -> Option<[f32; 3]> {
        let p = pos.get(v as usize).copied().flatten()?;
        Some([origin[0] + p[0] * scale, origin[1] + p[1] * scale, origin[2] + p[2] * scale])
    };
    if depth == 0 {
        s.points.extend((0..pos.len()).filter_map(|v| at(v as u8)));
    }
    let mesh = model::walk(&set.image, off + h.mesh as usize);
    for (prim, c) in mesh.prims {
        match prim {
            Prim::Line(v) => {
                if let (Some(a), Some(b)) = (at(v[0]), at(v[1])) {
                    s.shapes.push(Shape::Line([a, b], c));
                }
            }
            Prim::Tri(v) => face(&v, c, &at, s),
            Prim::Quad(v) => face(&v, c, &at, s),
            Prim::Polygon(v) => face(&v, c, &at, s),
            Prim::Disc { at: v, .. } => {
                if let Some(a) = at(v) {
                    s.shapes.push(Shape::Disc(a, c));
                }
            }
            Prim::Thruster { from, to, .. } => {
                if let (Some(a), Some(b)) = (at(from), at(to)) {
                    s.shapes.push(Shape::Thruster([a, b], c));
                }
            }
            Prim::Sub { model: m, at: v } => {
                let Some(o) = at(v) else { continue };
                s.parts.push(m);
                let sub_scale = set
                    .table
                    .models
                    .get(m as usize)
                    .copied()
                    .flatten()
                    .and_then(|o| model::header(&set.image, o).ok())
                    .and_then(|h| h.scale());
                if depth < 4 && m as usize != n {
                    let k = match (sub_scale, h.scale()) {
                        (Some(a), Some(b)) => 2f32.powi(a as i32 - b as i32),
                        _ => 1.0,
                    };
                    add(set, m as usize, o, scale * k.clamp(1.0 / 64.0, 64.0), depth + 1, s);
                } else {
                    s.shapes.push(Shape::Label(o, format!("model {m}")));
                }
            }
        }
    }
}

fn face(v: &[u8], c: u16, at: &dyn Fn(u8) -> Option<[f32; 3]>, s: &mut Scene) {
    let pts: Vec<[f32; 3]> = v.iter().filter_map(|&i| at(i)).collect();
    if pts.len() >= 3 {
        s.shapes.push(Shape::Face(pts, c));
    }
}

/// Draw the scene into `rect` with an orthographic camera turned by `yaw` and `pitch`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &egui::Painter,
    rect: egui::Rect,
    scene: &Scene,
    default: u16,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    filled: bool,
    thrusters: bool,
) {
    p.rect_filled(rect, 0.0, Color32::from_rgb(8, 10, 16));
    let mut fit: Vec<[f32; 3]> = Vec::new();
    for sh in &scene.shapes {
        match sh {
            Shape::Face(v, _) => fit.extend(v),
            Shape::Line(v, _) => fit.extend(v),
            Shape::Thruster(v, _) if thrusters => fit.extend(v),
            Shape::Thruster(..) => {}
            Shape::Disc(a, _) | Shape::Label(a, _) => fit.push(*a),
        }
    }
    if fit.is_empty() {
        fit = scene.points.clone();
    }
    let radius = fit.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(1.0f32, f32::max);
    let s = rect.width().min(rect.height()) * 0.45 / radius * zoom;
    let (cy, sy, cp, sp) = (yaw.cos(), yaw.sin(), pitch.cos(), pitch.sin());
    // Model space: x right, y up, z away from the viewer.
    let xf = |v: [f32; 3]| -> [f32; 3] {
        let x = v[0] * cy - v[2] * sy;
        let z = v[0] * sy + v[2] * cy;
        [x, v[1] * cp - z * sp, v[1] * sp + z * cp]
    };
    let scr = |v: [f32; 3]| Pos2::new(rect.center().x + v[0] * s, rect.center().y - v[1] * s);
    let colour = |c: u16| {
        let c = if c & 0xeee == 0 { default } else { c };
        let [r, g, b] = model::rgb12(c);
        Color32::from_rgb(r, g, b)
    };
    // Painter's order over every shape: farthest first.
    let mut items: Vec<(f32, &Shape)> = scene
        .shapes
        .iter()
        .map(|sh| {
            let d = match sh {
                Shape::Face(v, _) => v.iter().map(|&p| xf(p)[2]).sum::<f32>() / v.len() as f32,
                Shape::Line(v, _) | Shape::Thruster(v, _) => (xf(v[0])[2] + xf(v[1])[2]) / 2.0,
                Shape::Disc(a, _) | Shape::Label(a, _) => xf(*a)[2],
            };
            (d, sh)
        })
        .collect();
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, sh) in items {
        match sh {
            Shape::Face(v, c) => {
                let col = colour(*c);
                let t: Vec<[f32; 3]> = v.iter().map(|&p| xf(p)).collect();
                let pts: Vec<Pos2> = t.iter().map(|&q| scr(q)).collect();
                if filled {
                    let (a, b, c) = (t[0], t[1], t[2]);
                    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                    let light = (0.35 + 0.65 * ((n[0] * 0.3 + n[1] * 0.6 - n[2] * 0.75) / len).abs()).min(1.0);
                    let shade =
                        Color32::from_rgb((col.r() as f32 * light) as u8, (col.g() as f32 * light) as u8, (col.b() as f32 * light) as u8);
                    p.add(egui::Shape::convex_polygon(pts, shade, Stroke::new(0.5, Color32::from_black_alpha(120))));
                } else {
                    let mut closed = pts.clone();
                    closed.push(pts[0]);
                    p.add(egui::Shape::line(closed, Stroke::new(1.0, col)));
                }
            }
            Shape::Line(v, c) => {
                p.line_segment([scr(xf(v[0])), scr(xf(v[1]))], Stroke::new(1.5, colour(*c)));
            }
            Shape::Thruster(..) if !thrusters => {}
            Shape::Thruster(v, c) => {
                p.line_segment([scr(xf(v[0])), scr(xf(v[1]))], Stroke::new(4.0, colour(*c).gamma_multiply(0.8)));
            }
            Shape::Disc(a, c) => {
                p.circle_filled(scr(xf(*a)), 4.0, colour(*c));
            }
            Shape::Label(a, text) => {
                let q = scr(xf(*a));
                p.circle_stroke(q, 4.0, Stroke::new(1.0, Color32::LIGHT_BLUE));
                p.text(q + Vec2::new(5.0, -5.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(10.0), Color32::LIGHT_BLUE);
            }
        }
    }
    for &v in &scene.points {
        p.circle_filled(scr(xf(v)), 1.2, Color32::from_gray(110));
    }
}
