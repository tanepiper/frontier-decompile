//! A model and its sub-objects as one list of shapes in the model's space, and drawing it.
//!
//! Deviation: this is the viewer's picture of the geometry, with its own orthographic camera,
//! lighting and depth sort. The game's renderer (`1000:b207` and EL2MCGA3) is not used:
//! sub-objects keep the parent's orientation; discs and thruster flames get a fixed screen size;
//! textures are stretched over the whole face (the game picks one of 8 orientations,
//! VIDEO.md §5.3), on a base grey from the viewer's light (the game's base is the face's shade
//! word). By default the near branches are drawn with every model variable 0 (*assumed*), back
//! faces are hidden by their stored normals as `face_shade` does, and textures are on; each can
//! be turned off.

use egui::{Color32, Pos2, Stroke, Vec2};
use frontier_formats::model::{self, Branches, Paint, Prim};

use crate::library::ModelSet;

pub enum Shape {
    /// Corners, colour field, the stored normal if the face has one, texture coordinates.
    Face {
        pts: Vec<[f32; 3]>,
        colour: u16,
        normal: Option<[f32; 3]>,
        uv: Vec<[f32; 2]>,
    },
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

/// How to draw.
#[derive(Clone, Copy)]
pub struct Options {
    pub filled: bool,
    pub thrusters: bool,
    pub textures: bool,
    pub back_faces: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

/// Flatten model `n` of `set`. Sub-objects (`0E`) are placed at their vertex and scaled by
/// 2^(their header scale − the parent's), since coordinates are shifted by `17h − scale`
/// (`EL2MATH3:0d58`, *inferred* for sub-objects); depth is limited to 4.
/// `livery` is the value of the paint variable (0 = each model's own colours).
pub fn build(set: &ModelSet, n: usize, branches: Branches, livery: u8) -> Scene {
    let mut s = Scene::default();
    add(set, n, [0.0; 3], 1.0, 0, branches, Paint { colour: model::START_COLOUR, livery }, &mut s);
    s
}

#[allow(clippy::too_many_arguments)]
fn add(set: &ModelSet, n: usize, origin: [f32; 3], scale: f32, depth: u32, branches: Branches, mut paint: Paint, s: &mut Scene) {
    let Some(Some(off)) = set.table.models.get(n).copied() else { return };
    let Ok(h) = model::header(&set.image, off) else { return };
    let defs = model::vertex_defs(&set.image, &h).unwrap_or_default();
    let pos = model::resolve(&defs);
    let normals = model::normals(&set.image, &h);
    let at = |v: u8| -> Option<[f32; 3]> {
        let p = pos.get(v as usize).copied().flatten()?;
        Some([origin[0] + p[0] * scale, origin[1] + p[1] * scale, origin[2] + p[2] * scale])
    };
    let normal = |k: u8| normals.get(k as usize).copied().flatten();
    if depth == 0 {
        s.points.extend((0..pos.len()).filter_map(|v| at(v as u8)));
    }
    // Sub-objects start with the model colour as it is when they are drawn; the walk returns
    // them in order, so the colour is replayed alongside (*inferred*: `1000:7c2b`).
    let start_paint = paint;
    let mesh = model::walk_with(&set.image, off + h.mesh as usize, branches, &mut paint);
    let mut replay = start_paint;
    let mut cmd = 0;
    for (prim, c) in mesh.prims {
        match prim {
            Prim::Line(v) => {
                if let (Some(a), Some(b)) = (at(v[0]), at(v[1])) {
                    s.shapes.push(Shape::Line([a, b], c));
                }
            }
            Prim::Tri(v, k) => face(&v, c, normal(k), &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &at, s),
            Prim::Quad(v, k) => face(&v, c, normal(k), &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], &at, s),
            Prim::Polygon(v) => face(&v, c, None, &[], &at, s),
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
                // The model colour when this sub-object's command ran.
                while cmd < mesh.commands.len() {
                    let k = &mesh.commands[cmd];
                    cmd += 1;
                    if k.op() == 0x1a {
                        let mut c = (k.word >> 4) & 0xffe;
                        if k.operands[0] >> 8 != 0 && k.operands.len() == 8 && (1..=7).contains(&replay.livery) {
                            c = k.operands[replay.livery as usize] & 0xfff;
                        }
                        replay.colour = c;
                    }
                    if k.op() == 0x0e && (k.word >> 5) == m {
                        break;
                    }
                }
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
                    add(set, m as usize, o, scale * k.clamp(1.0 / 64.0, 64.0), depth + 1, branches, replay, s);
                } else {
                    s.shapes.push(Shape::Label(o, format!("model {m}")));
                }
            }
        }
    }
}

fn face(v: &[u8], colour: u16, normal: Option<[f32; 3]>, uv: &[[f32; 2]], at: &dyn Fn(u8) -> Option<[f32; 3]>, s: &mut Scene) {
    let pts: Vec<[f32; 3]> = v.iter().filter_map(|&i| at(i)).collect();
    if pts.len() < 3 {
        return;
    }
    let uv = if uv.len() == pts.len() { uv.to_vec() } else { planar_uv(&pts) };
    s.shapes.push(Shape::Face { pts, colour, normal, uv });
}

/// Texture coordinates for a polygon: its corners projected onto its plane, scaled to 0–1.
fn planar_uv(pts: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let u = sub(pts[1], pts[0]);
    let n = cross(u, sub(pts[2], pts[0]));
    let w = cross(n, u);
    let raw: Vec<[f32; 2]> = pts.iter().map(|&p| [dot(sub(p, pts[0]), u), dot(sub(p, pts[0]), w)]).collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in &raw {
        for i in 0..2 {
            lo[i] = lo[i].min(q[i]);
            hi[i] = hi[i].max(q[i]);
        }
    }
    raw.iter().map(|q| [(q[0] - lo[0]) / (hi[0] - lo[0]).max(1e-6), (q[1] - lo[1]) / (hi[1] - lo[1]).max(1e-6)]).collect()
}

/// One thing to draw on the screen, in back-to-front order.
pub enum Item {
    /// A filled convex polygon: corners, colour, and a texture-table entry with its base colour
    /// and texture coordinates when it is textured.
    Fill {
        pts: Vec<Pos2>,
        colour: Color32,
        texture: Option<(u8, u16, Vec<[f32; 2]>)>,
    },
    Outline {
        pts: Vec<Pos2>,
        colour: Color32,
    },
    Line {
        a: Pos2,
        b: Pos2,
        width: f32,
        colour: Color32,
    },
    Dot {
        at: Pos2,
        radius: f32,
        colour: Color32,
    },
    Label {
        at: Pos2,
        text: String,
    },
}

/// Project the scene into `rect`: what to draw, back to front.
pub fn project(rect: egui::Rect, scene: &Scene, default: u16, o: Options) -> Vec<Item> {
    let mut out = Vec::new();
    let mut fit: Vec<[f32; 3]> = Vec::new();
    for sh in &scene.shapes {
        match sh {
            Shape::Face { pts, .. } => fit.extend(pts),
            Shape::Line(v, _) => fit.extend(v),
            Shape::Thruster(v, _) if o.thrusters => fit.extend(v),
            Shape::Thruster(..) => {}
            Shape::Disc(a, _) | Shape::Label(a, _) => fit.push(*a),
        }
    }
    if fit.is_empty() {
        fit = scene.points.clone();
    }
    let radius = fit.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(1.0f32, f32::max);
    let s = rect.width().min(rect.height()) * 0.45 / radius * o.zoom;
    let (cy, sy, cp, sp) = (o.yaw.cos(), o.yaw.sin(), o.pitch.cos(), o.pitch.sin());
    // Model space: x right, y up, z away from the viewer.
    let xf = |v: [f32; 3]| -> [f32; 3] {
        let x = v[0] * cy - v[2] * sy;
        let z = v[0] * sy + v[2] * cy;
        [x, v[1] * cp - z * sp, v[1] * sp + z * cp]
    };
    let scr = |v: [f32; 3]| Pos2::new(rect.center().x + v[0] * s, rect.center().y - v[1] * s);
    let base_colour = |c: u16| if c & 0xeee == 0 || model::texture_entry(c).is_some() { default } else { c };
    let colour = |c: u16| {
        let [r, g, b] = model::rgb12(base_colour(c));
        Color32::from_rgb(r, g, b)
    };
    // Painter's order over every shape: farthest first.
    let mut items: Vec<(f32, &Shape)> = scene
        .shapes
        .iter()
        .map(|sh| {
            let d = match sh {
                Shape::Face { pts, .. } => pts.iter().map(|&p| xf(p)[2]).sum::<f32>() / pts.len() as f32,
                Shape::Line(v, _) | Shape::Thruster(v, _) => (xf(v[0])[2] + xf(v[1])[2]) / 2.0,
                Shape::Disc(a, _) | Shape::Label(a, _) => xf(*a)[2],
            };
            (d, sh)
        })
        .collect();
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, sh) in items {
        match sh {
            Shape::Face { pts, colour: c, normal, uv } => {
                let t: Vec<[f32; 3]> = pts.iter().map(|&q| xf(q)).collect();
                // The face's normal, turned with the model: the stored one, else from the corners.
                let n = match normal {
                    Some(n) => xf(*n),
                    None => {
                        let (a, b, c) = (t[0], t[1], t[2]);
                        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                        [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]]
                    }
                };
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                // A stored normal pointing away from the viewer (+z) hides the face.
                if !o.back_faces && normal.is_some() && n[2] / len > 0.0 {
                    continue;
                }
                let pts2: Vec<Pos2> = t.iter().map(|&q| scr(q)).collect();
                if !o.filled {
                    out.push(Item::Outline { pts: pts2, colour: colour(*c) });
                    continue;
                }
                let light = (0.35 + 0.65 * ((n[0] * 0.3 + n[1] * 0.6 - n[2] * 0.75) / len).abs()).min(1.0);
                let col = colour(*c);
                let shade =
                    Color32::from_rgb((col.r() as f32 * light) as u8, (col.g() as f32 * light) as u8, (col.b() as f32 * light) as u8);
                let texture =
                    model::texture_entry(*c).filter(|_| o.textures).map(|e| (e, ((light * 5.0).round() as u16) * 0x111, uv.clone()));
                out.push(Item::Fill { pts: pts2, colour: shade, texture });
            }
            Shape::Line(v, c) => out.push(Item::Line { a: scr(xf(v[0])), b: scr(xf(v[1])), width: 1.5, colour: colour(*c) }),
            Shape::Thruster(..) if !o.thrusters => {}
            Shape::Thruster(v, c) => {
                out.push(Item::Line { a: scr(xf(v[0])), b: scr(xf(v[1])), width: 4.0, colour: colour(*c).gamma_multiply(0.8) })
            }
            Shape::Disc(a, c) => out.push(Item::Dot { at: scr(xf(*a)), radius: 4.0, colour: colour(*c) }),
            Shape::Label(a, text) => out.push(Item::Label { at: scr(xf(*a)), text: text.clone() }),
        }
    }
    for &v in &scene.points {
        out.push(Item::Dot { at: scr(xf(v)), radius: 1.2, colour: Color32::from_gray(110) });
    }
    out
}

/// Draw the scene into `rect` with egui. `texture(entry, base)` gives the egui texture of
/// texture-table entry `entry` coloured on the 12-bit base colour `base`, if there is one.
pub fn draw(
    p: &egui::Painter,
    rect: egui::Rect,
    scene: &Scene,
    default: u16,
    o: Options,
    texture: &mut dyn FnMut(u8, u16) -> Option<egui::TextureId>,
) {
    p.rect_filled(rect, 0.0, Color32::from_rgb(8, 10, 16));
    for item in project(rect, scene, default, o) {
        match item {
            Item::Fill { pts, colour, texture: tex } => {
                if let Some((id, uv)) = tex.and_then(|(e, base, uv)| Some((texture(e, base)?, uv))) {
                    let mut m = egui::Mesh::with_texture(id);
                    for (q, w) in pts.iter().zip(&uv) {
                        m.vertices.push(egui::epaint::Vertex { pos: *q, uv: egui::pos2(w[0], w[1]), color: Color32::WHITE });
                    }
                    for i in 1..pts.len() as u32 - 1 {
                        m.add_triangle(0, i, i + 1);
                    }
                    p.add(egui::Shape::mesh(m));
                } else {
                    p.add(egui::Shape::convex_polygon(pts, colour, Stroke::new(0.5, Color32::from_black_alpha(120))));
                }
            }
            Item::Outline { mut pts, colour } => {
                pts.push(pts[0]);
                p.add(egui::Shape::line(pts, Stroke::new(1.0, colour)));
            }
            Item::Line { a, b, width, colour } => {
                p.line_segment([a, b], Stroke::new(width, colour));
            }
            Item::Dot { at, radius, colour } => {
                p.circle_filled(at, radius, colour);
            }
            Item::Label { at, text } => {
                p.circle_stroke(at, 4.0, Stroke::new(1.0, Color32::LIGHT_BLUE));
                p.text(at + Vec2::new(5.0, -5.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(10.0), Color32::LIGHT_BLUE);
            }
        }
    }
}
