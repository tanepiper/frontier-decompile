//! A small software rasteriser for [`scene::Item`]s and a PNG writer, so the viewer can draw a
//! model into a file without a window (`frontier-viewer --render`): for agents and tests, which
//! cannot look at a window. It draws the same items the window does, without anti-aliasing.

use egui::{Color32, Pos2};

use crate::scene::Item;

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// A texture as `(width, height, rgba)`.
pub type Texels = (usize, usize, Vec<u8>);

impl Canvas {
    pub fn new(w: usize, h: usize, bg: Color32) -> Canvas {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            rgba.extend_from_slice(&[bg.r(), bg.g(), bg.b(), 255]);
        }
        Canvas { w, h, rgba }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = (y as usize * self.w + x as usize) * 4;
            self.rgba[i..i + 3].copy_from_slice(&c);
        }
    }

    /// Fill a triangle, colouring each pixel with `shade(barycentric a, b, c)`.
    fn triangle(&mut self, p: [Pos2; 3], shade: &mut dyn FnMut(f32, f32, f32) -> [u8; 3]) {
        let (x0, x1) =
            (p.iter().map(|q| q.x).fold(f32::MAX, f32::min).floor() as i32, p.iter().map(|q| q.x).fold(f32::MIN, f32::max).ceil() as i32);
        let (y0, y1) =
            (p.iter().map(|q| q.y).fold(f32::MAX, f32::min).floor() as i32, p.iter().map(|q| q.y).fold(f32::MIN, f32::max).ceil() as i32);
        let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
        if area.abs() < 1e-6 {
            return;
        }
        for y in y0.max(0)..=y1.min(self.h as i32 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i32 - 1) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1].x - px) * (p[2].y - py) - (p[2].x - px) * (p[1].y - py)) / area;
                let w1 = ((p[2].x - px) * (p[0].y - py) - (p[0].x - px) * (p[2].y - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let c = shade(w0, w1, w2);
                    self.put(x, y, c);
                }
            }
        }
    }

    fn line(&mut self, a: Pos2, b: Pos2, width: f32, c: [u8; 3]) {
        let n = ((b.x - a.x).abs().max((b.y - a.y).abs()) as usize).max(1);
        let r = (width / 2.0).max(0.5) as i32;
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (x, y) = ((a.x + (b.x - a.x) * t) as i32, (a.y + (b.y - a.y) * t) as i32);
            for dy in -r + 1..r.max(1) {
                for dx in -r + 1..r.max(1) {
                    self.put(x + dx, y + dy, c);
                }
            }
        }
    }

    /// Draw `items` in order. `texture(entry, base)` gives a texture's texels.
    pub fn draw(&mut self, items: &[Item], texture: &mut dyn FnMut(u8, u16) -> Option<Texels>) {
        let rgb = |c: Color32| [c.r(), c.g(), c.b()];
        for item in items {
            match item {
                Item::Fill { pts, colour, texture: tex } => {
                    let texels = tex.as_ref().and_then(|(e, base, _)| texture(*e, *base));
                    for i in 1..pts.len().saturating_sub(1) {
                        let tri = [pts[0], pts[i], pts[i + 1]];
                        match (&texels, tex) {
                            (Some((w, h, px)), Some((_, _, uv))) => {
                                let (a, b, c) = (uv[0], uv[i], uv[i + 1]);
                                self.triangle(tri, &mut |l0, l1, l2| {
                                    let u = (a[0] * l0 + b[0] * l1 + c[0] * l2).clamp(0.0, 0.999);
                                    let v = (a[1] * l0 + b[1] * l1 + c[1] * l2).clamp(0.0, 0.999);
                                    let k = ((v * *h as f32) as usize * w + (u * *w as f32) as usize) * 4;
                                    [px[k], px[k + 1], px[k + 2]]
                                });
                            }
                            _ => self.triangle(tri, &mut |_, _, _| rgb(*colour)),
                        }
                    }
                }
                Item::Outline { pts, colour } => {
                    for i in 0..pts.len() {
                        self.line(pts[i], pts[(i + 1) % pts.len()], 1.0, rgb(*colour));
                    }
                }
                Item::Line { a, b, width, colour } => self.line(*a, *b, *width, rgb(*colour)),
                Item::Dot { at, radius, colour } => {
                    let r = radius.ceil() as i32;
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if (dx * dx + dy * dy) as f32 <= radius * radius {
                                self.put(at.x as i32 + dx, at.y as i32 + dy, rgb(*colour));
                            }
                        }
                    }
                }
                Item::Label { at, .. } => {
                    for d in -3..=3 {
                        self.put(at.x as i32 + d, at.y as i32, [120, 170, 255]);
                        self.put(at.x as i32, at.y as i32 + d, [120, 170, 255]);
                    }
                }
            }
        }
    }

    /// Copy `other` to (`x`, `y`).
    pub fn blit(&mut self, other: &Canvas, x: usize, y: usize) {
        for row in 0..other.h.min(self.h.saturating_sub(y)) {
            let n = other.w.min(self.w.saturating_sub(x)) * 4;
            let d = ((y + row) * self.w + x) * 4;
            self.rgba[d..d + n].copy_from_slice(&other.rgba[row * other.w * 4..row * other.w * 4 + n]);
        }
    }

    /// An RGB PNG with uncompressed (stored) deflate blocks: no compression crate needed.
    pub fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(self.h * (self.w * 3 + 1));
        for y in 0..self.h {
            raw.push(0);
            for x in 0..self.w {
                let i = (y * self.w + x) * 4;
                raw.extend_from_slice(&self.rgba[i..i + 3]);
            }
        }
        let mut z = vec![0x78, 0x01];
        for (k, block) in raw.chunks(65535).enumerate() {
            let last = (k + 1) * 65535 >= raw.len();
            z.push(last as u8);
            z.extend_from_slice(&(block.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = (self.w as u32).to_be_bytes().to_vec();
        ihdr.extend_from_slice(&(self.h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        for (kind, data) in [(b"IHDR", ihdr), (b"IDAT", z), (b"IEND", Vec::new())] {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = kind.to_vec();
            c.extend_from_slice(&data);
            out.extend_from_slice(&c);
            out.extend_from_slice(&crc32(&c).to_be_bytes());
        }
        out
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}
