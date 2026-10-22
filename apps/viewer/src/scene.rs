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
    /// Ranges of `shapes` that lie on a surface (text): drawn after what is at their depth.
    pub decals: Vec<std::ops::Range<usize>>,
}

/// How to draw.
#[derive(Clone, Copy)]
pub struct Options {
    pub filled: bool,
    pub thrusters: bool,
    pub textures: bool,
    pub back_faces: bool,
    /// The viewer's own markers at every vertex of the model (not drawn by the game).
    pub vertices: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

/// What the scene is built with: the model tables, the starting state (object variables,
/// livery), the registration text and the text table.
pub struct Context<'a> {
    pub set: &'a ModelSet,
    /// The second table of `FRONTIER.EXE` (fonts for `0A` text).
    pub fonts: Option<&'a ModelSet>,
    pub paint: Paint,
    pub branches: Branches,
    /// Text `3016h`: the object's registration (`[DS:07e4]`, object `+F6h`).
    pub registration: &'a str,
    /// Texts `4000h`–`7FFFh` ([`model::texts`]).
    pub texts: &'a [String],
}

/// Flatten model `n`. Sub-objects (`0E`) are placed at their vertex and scaled by
/// 2^(their header scale − the parent's), since coordinates are shifted by `17h − scale`
/// (`EL2MATH3:0d58`, *inferred* for sub-objects); depth is limited to 4. Each starts with the
/// state the walk gives it (`model::Mesh::sub_paints`).
pub fn build(cx: &Context, n: usize) -> Scene {
    let mut s = Scene::default();
    add(cx, cx.set, n, [0.0; 3], IDENTITY, 0, cx.paint.clone(), &mut s);
    s
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// `Σ v[i] · basis[i]`: a model-space vector in the world, with `basis` the images of the
/// model's x, y and z axes (scale included).
fn apply(basis: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|j| v[0] * basis[0][j] + v[1] * basis[1][j] + v[2] * basis[2][j])
}

/// A child's basis: its orientation `m` (rows in the parent's space) scaled by `k`, in the world.
fn compose(m: &[[f32; 3]; 3], k: f32, basis: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| apply(basis, m[i]).map(|x| x * k))
}

/// Add model `n` with its origin at `origin` and its axes at `basis` (rows, scale included).
#[allow(clippy::too_many_arguments)]
fn add(cx: &Context, set: &ModelSet, n: usize, origin: [f32; 3], basis: [[f32; 3]; 3], depth: u32, mut paint: Paint, s: &mut Scene) {
    let Some(Some(off)) = set.table.models.get(n).copied() else { return };
    let Ok(h) = model::header(&set.image, off) else { return };
    let defs = model::vertex_defs(&set.image, &h).unwrap_or_default();
    let pos = model::resolve(&defs);
    let normals = model::normals(&set.image, &h);
    let at = |v: u8| -> Option<[f32; 3]> {
        let p = apply(&basis, pos.get(v as usize).copied().flatten()?);
        Some([origin[0] + p[0], origin[1] + p[1], origin[2] + p[2]])
    };
    let normal = |k: u8| normals.get(k as usize).copied().flatten().map(|n| apply(&basis, n));
    if depth == 0 {
        s.points.extend((0..pos.len()).filter_map(|v| at(v as u8)));
    }
    let mesh = model::walk_with(&set.image, off + h.mesh as usize, cx.branches, &mut paint);
    let mut subs = mesh.sub_paints.iter();
    for (prim, c) in mesh.prims {
        match prim {
            Prim::Sub { model: m, at: v, scale: step, matrix } => {
                let child = subs.next().cloned().unwrap_or_else(|| paint.child());
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
                        (Some(a), Some(b)) => 2f32.powi(a as i32 + step as i32 - b as i32),
                        _ => 1.0,
                    };
                    add(cx, set, m as usize, o, compose(&matrix, k.clamp(1.0 / 64.0, 64.0), &basis), depth + 1, child, s);
                } else {
                    s.shapes.push(Shape::Label(o, format!("model {m}")));
                }
            }
            Prim::Text { at: v, normal: tn, font, scale: fs, text, matrix } => {
                let (Some(o), Some(fonts)) = (at(v), cx.fonts) else { continue };
                let string = match text {
                    0x3016 => cx.registration.to_string(),
                    0x4000..=0x7fff => cx.texts.get((text & 0x3fff) as usize).cloned().unwrap_or_default(),
                    _ => String::new(),
                };
                let k = 2f32.powi(fs as i32 - h.scale().unwrap_or(fs as u16) as i32);
                // The 0A command's colour is the text's model colour (1000:7978, 79cc).
                let mut tp = paint.child();
                tp.colour = c;
                text_shapes(cx, fonts, font as usize, &string, o, compose(&matrix, k, &basis), &tp, normal(tn), s);
            }
            other => draw_prim(other, c, &at, &normal, s),
        }
    }
}

/// One drawing command of a walked mesh, with its corners at `at`.
fn draw_prim(prim: Prim, c: u16, at: &dyn Fn(u8) -> Option<[f32; 3]>, normal: &dyn Fn(u8) -> Option<[f32; 3]>, s: &mut Scene) {
    match prim {
        Prim::Line(v) => {
            if let (Some(a), Some(b)) = (at(v[0]), at(v[1])) {
                s.shapes.push(Shape::Line([a, b], c));
            }
        }
        Prim::Tri(v, k) => face(&v, c, normal(k), &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], at, s),
        Prim::Quad(v, k) => face(&v, c, normal(k), &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], at, s),
        Prim::Polygon { points, curves } => {
            // Curves sampled as cubic Béziers in 8 steps (*inferred* shape).
            let mut pts = Vec::new();
            let mut i = 0;
            while i < points.len() {
                if curves.contains(&i) && i + 4 <= points.len() {
                    let q: Vec<Option<[f32; 3]>> = points[i..i + 4].iter().map(|&v| at(v)).collect();
                    if let [Some(a), Some(b), Some(c2), Some(d)] = q[..] {
                        for k in 0..=8 {
                            let t = k as f32 / 8.0;
                            let u = 1.0 - t;
                            pts.push(std::array::from_fn(|j| {
                                u * u * u * a[j] + 3.0 * u * u * t * b[j] + 3.0 * u * t * t * c2[j] + t * t * t * d[j]
                            }));
                        }
                    }
                    i += 4;
                } else {
                    if let Some(p) = at(points[i]) {
                        if pts.last() != Some(&p) {
                            pts.push(p);
                        }
                    }
                    i += 1;
                }
            }
            if pts.len() >= 3 {
                let uv = planar_uv(&pts);
                s.shapes.push(Shape::Face { pts, colour: c, normal: None, uv });
            }
        }
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
        Prim::Sub { .. } | Prim::Text { .. } => {}
    }
}

/// Text drawn with font model `font` of the second table, as `1000:7a94` does: each character's
/// glyph is the command list at the font's `+20h + 2·(char − 20h)` offset, drawn at the cursor;
/// the vertex named by the glyph's end word (`>> 6`) is where the next character starts
/// (`1000:7ac6`); a control character returns to the line start moved by the vertex in header
/// word `+1Eh` (`1000:7b14`). Points are `origin + x·row0 + y·row1 + z·row2` of the font's
/// world basis (*measured*: the text frame's matrix is R·P with R constant, docs/FORMATS.md §4).
#[allow(clippy::too_many_arguments)]
fn text_shapes(
    cx: &Context,
    fonts: &ModelSet,
    font: usize,
    text: &str,
    origin: [f32; 3],
    m: [[f32; 3]; 3],
    paint: &Paint,
    normal: Option<[f32; 3]>,
    s: &mut Scene,
) {
    let Some(Some(f)) = fonts.table.models.get(font).copied() else { return };
    let Ok(fh) = model::header(&fonts.image, f) else { return };
    let fpos = model::resolve(&model::vertex_defs(&fonts.image, &fh).unwrap_or_default());
    let place =
        |base: [f32; 3], v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| base[i] + v[0] * m[0][i] + v[1] * m[1][i] + v[2] * m[2][i]) };
    let word = |o: usize| fonts.image.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let (mut cursor, mut line) = (origin, origin);
    let first = s.shapes.len();
    for ch in text.bytes().take(64) {
        if ch < 0x20 {
            let lf = word(f + 0x1e).and_then(|v| fpos.get(v as i16 as usize).copied().flatten()).unwrap_or([0.0; 3]);
            line = place(line, lf);
            cursor = line;
            continue;
        }
        let Some(rel) = word(f + 0x20 + 2 * (ch - 0x20) as usize) else { continue };
        let mut gp = paint.child();
        gp.colour = paint.colour;
        let glyph = model::walk_with(&fonts.image, f + rel as usize, cx.branches, &mut gp);
        let base = cursor;
        let at = |v: u8| fpos.get(v as usize).copied().flatten().map(|p| place(base, p));
        // The glyphs take the text command's normal: shown when it faces the viewer (1000:7917).
        for (prim, c) in glyph.prims {
            draw_prim(prim, c, &at, &|_| normal, s);
            if let Some(Shape::Face { normal: n, .. }) = s.shapes.last_mut() {
                *n = normal;
            }
        }
        if let Some(end) = glyph.commands.last().filter(|c| c.op() == 0) {
            let adv = (end.word >> 6) as usize;
            if adv != 0 {
                if let Some(p) = fpos.get(adv).copied().flatten() {
                    cursor = place(cursor, p);
                }
            }
        }
    }
    s.decals.push(first..s.shapes.len());
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
    // Centre on the middle of what is drawn (models often lie off their origin) and size the
    // view to the farthest point from there.
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &fit {
        for i in 0..3 {
            lo[i] = lo[i].min(v[i]);
            hi[i] = hi[i].max(v[i]);
        }
    }
    let centre: [f32; 3] = if fit.is_empty() { [0.0; 3] } else { std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0) };
    let radius = fit
        .iter()
        .map(|v| ((v[0] - centre[0]).powi(2) + (v[1] - centre[1]).powi(2) + (v[2] - centre[2]).powi(2)).sqrt())
        .fold(1.0f32, f32::max);
    let s = rect.width().min(rect.height()) * 0.45 / radius * o.zoom;
    let (cy, sy, cp, sp) = (o.yaw.cos(), o.yaw.sin(), o.pitch.cos(), o.pitch.sin());
    // Model space: x right, y up, z away from the viewer.
    let rot = |v: [f32; 3]| -> [f32; 3] {
        let x = v[0] * cy - v[2] * sy;
        let z = v[0] * sy + v[2] * cy;
        [x, v[1] * cp - z * sp, v[1] * sp + z * cp]
    };
    let xf = |v: [f32; 3]| rot([v[0] - centre[0], v[1] - centre[1], v[2] - centre[2]]);
    let scr = |v: [f32; 3]| Pos2::new(rect.center().x + v[0] * s, rect.center().y - v[1] * s);
    let base_colour = |c: u16| if c & 0xeee == 0 || model::texture_entry(c).is_some() { default } else { c };
    let colour = |c: u16| {
        let [r, g, b] = model::rgb12(base_colour(c));
        Color32::from_rgb(r, g, b)
    };
    // Painter's order over every shape: farthest first. Text on a surface is drawn after every
    // other shape (it lies on the hull; the game draws it after the surface it is on), back to
    // front among itself.
    let decal = |i: usize| scene.decals.iter().any(|r| r.contains(&i));
    let mut items: Vec<(f32, &Shape)> = scene
        .shapes
        .iter()
        .enumerate()
        .map(|(i, sh)| {
            let d = match sh {
                Shape::Face { pts, .. } => pts.iter().map(|&p| xf(p)[2]).sum::<f32>() / pts.len() as f32,
                Shape::Line(v, _) | Shape::Thruster(v, _) => (xf(v[0])[2] + xf(v[1])[2]) / 2.0,
                Shape::Disc(a, _) | Shape::Label(a, _) => xf(*a)[2],
            };
            (if decal(i) { d - radius * 1000.0 } else { d }, sh)
        })
        .collect();
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, sh) in items {
        match sh {
            Shape::Face { pts, colour: c, normal, uv } => {
                let t: Vec<[f32; 3]> = pts.iter().map(|&q| xf(q)).collect();
                // The face's normal, turned with the model: the stored one, else from the corners.
                let n = match normal {
                    Some(n) => rot(*n),
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
    for &v in scene.points.iter().filter(|_| o.vertices) {
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
