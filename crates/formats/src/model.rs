//! 3D models: the model tables, the model header, the vertex definitions and the mesh command
//! lists the main program's renderer interprets (docs/FORMATS.md §4).
//!
//! The renderer asks `1000:18b5` (slot `DS:0050`) for a model: `ES` = the table segment
//! `[DS:5701]`, `BX` = `ES:[BX + base] + base` with `base` = `[DS:56ff]`. By default
//! (`1000:18de`, slot `DS:013c`) the segment is the relocated word at `DS:07fa`, which points
//! into `FRONTIER.EXE` itself, and `base` = 4. A module can install its own table with
//! `1000:18c5` (slot `DS:0150`): `EL2M08` (title sequence) does so at `EL2M08:6eeb` with its
//! code segment and `base` = `3B0h`.
//!
//! What is *observed* (read in the code): the table lookup, the header words `+0` (mesh), `+2`
//! (vertex definitions), `+4` (vertex record bytes, 20h per vertex), `+6` (normals), `+8`
//! (normal list bytes); the vertex definition layout `[x, type, z, y]` per mirror pair
//! (`EL2MATH3:0d3a`, `0d58`); the mesh dispatch (`1000:b21e`: opcode = low 5 bits of a word,
//! through the table at `CS:2864`); the operand words of the opcodes listed in [`operands`].
//! What is *inferred*: the table length (it ends where the first model starts), how derived
//! vertex types place a point when there is no frame to project into, and how a few opcodes
//! draw (see [`Prim`]).

use crate::{mz, Error, Result};

/// Main data segment paragraph in `FRONTIER.EXE`'s image (`1000:071e`, docs/OVERLAYS.md).
pub const MAIN_DS: usize = 0x1615;
/// `DS:07fa`: relocated word holding the default model table segment (*observed* at `1000:18ef`).
pub const DEFAULT_TABLE_SEG_VAR: usize = 0x07fa;
/// The default table's `base` (`1000:18f7`).
pub const DEFAULT_BASE: usize = 4;
/// `EL2M08`'s table base, passed in `AX` to `1000:18c5` at `EL2M08:6ede`.
pub const EL2M08_BASE: usize = 0x3b0;

/// A model table: models are numbered by their slot (the word index `BX / 2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Image offset that the table's offsets are relative to (segment start + `base`).
    pub origin: usize,
    /// Image offset of each model header, `None` for an empty slot (offset word 0).
    pub models: Vec<Option<usize>>,
}

fn le16(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(Error::Truncated { needed: at + 2, available: d.len() })
}

/// Read a table at `origin`: words until the first model begins (*inferred*: in both shipped
/// tables the smallest offset is exactly the table's size).
pub fn table(image: &[u8], origin: usize) -> Result<Table> {
    let mut words = Vec::new();
    let mut end = usize::MAX;
    let mut p = origin;
    while p + 2 <= origin.saturating_add(end) && p + 2 <= image.len() {
        let w = le16(image, p)? as usize;
        if w != 0 && w < end {
            end = w;
        }
        words.push(w);
        p += 2;
        if words.len() > 4096 {
            break;
        }
    }
    words.truncate(end / 2);
    let models = words.into_iter().map(|w| (w != 0 && origin + w < image.len()).then_some(origin + w)).collect();
    Ok(Table { origin, models })
}

/// The default model table of `FRONTIER.EXE`: the load image and the table.
pub fn main_table(exe: &[u8]) -> Result<(Vec<u8>, Table)> {
    let h = mz::parse(exe)?;
    let image = exe.get(h.header_size()..).ok_or(Error::Invalid("no load image"))?.to_vec();
    let seg = le16(&image, MAIN_DS * 16 + DEFAULT_TABLE_SEG_VAR)? as usize;
    let t = table(&image, seg * 16 + DEFAULT_BASE)?;
    Ok((image, t))
}

/// The model header words. Field names past `+8` are *inferred* from use (`1000:7c48`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Image offset of the header.
    pub offset: usize,
    /// `+0`: mesh commands, relative to the header.
    pub mesh: u16,
    /// `+2`: vertex definitions, relative to the header (also the header's length).
    pub vertices: u16,
    /// `+4`: bytes of vertex records the renderer reserves (`1000:b290`): 20h per vertex.
    pub vertex_bytes: u16,
    /// `+6`: normals, relative to the header.
    pub normals: u16,
    /// `+8`: bytes of the normal list (`1000:b298`), 4 per normal.
    pub normal_bytes: u16,
    /// Every header word from `+0` up to the vertex definitions.
    pub words: Vec<u16>,
}

impl Header {
    pub fn vertex_count(&self) -> usize {
        self.vertex_bytes as usize / 0x20
    }
    /// `+0Ah`: scale (*inferred*: added with `+0Ch` into the frame's scale at `1000:7c4e`).
    pub fn scale(&self) -> Option<u16> {
        self.words.get(5).copied()
    }
    /// `+12h`: the default colour, 12-bit `0RGB` (*inferred*: `0666h` grey on most models).
    pub fn colour(&self) -> Option<u16> {
        self.words.get(9).copied()
    }
}

pub fn header(image: &[u8], offset: usize) -> Result<Header> {
    let w = |i: usize| le16(image, offset + i * 2);
    let vertices = w(1)?;
    let n = (vertices as usize / 2).clamp(5, 32);
    let words = (0..n).map(w).collect::<Result<Vec<_>>>()?;
    Ok(Header { offset, mesh: words[0], vertices, vertex_bytes: words[2], normals: words[3], normal_bytes: words[4], words })
}

/// One vertex definition: 4 bytes per mirror pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexDef {
    pub kind: i8,
    /// Byte 0 (`BL` at `EL2MATH3:0d58`).
    pub x: i8,
    /// Byte 3 (`DH`).
    pub y: i8,
    /// Byte 2 (`DL`).
    pub z: i8,
}

impl VertexDef {
    pub fn describe(&self) -> String {
        let (a, b, c) = (self.z as u8, self.y as u8, self.x as u8);
        match self.kind {
            k if k <= 2 => format!("point ({}, {}, {})", self.x, self.y, self.z),
            3 | 4 => format!("screen midpoint of {b} and {a}"),
            5 | 6 => format!("negation of {a}"),
            7 | 8 => format!("{a} plus random >> {c}"),
            9 | 10 => format!("point ({}, {}, {}), partner x = 0", self.x, self.y, self.z),
            11 | 12 => format!("average of {a} and {b}"),
            13 | 14 => format!("screen midpoint of {a} and {b}, world {a}"),
            15 | 16 => format!("{a} − {b} + {c}"),
            17 | 18 => format!("{a} + {b}"),
            19 | 20 => format!("between {b} and {a} by parameter {c:02x}"),
            _ => format!("direction ({}, {}, {}) normalised", self.x, self.y, self.z),
        }
    }
}

pub fn vertex_defs(image: &[u8], h: &Header) -> Result<Vec<VertexDef>> {
    let start = h.offset + h.vertices as usize;
    let pairs = h.vertex_count().div_ceil(2);
    (0..pairs)
        .map(|i| {
            let b = image
                .get(start + i * 4..start + i * 4 + 4)
                .ok_or(Error::Truncated { needed: start + i * 4 + 4, available: image.len() })?;
            Ok(VertexDef { x: b[0] as i8, kind: b[1] as i8, z: b[2] as i8, y: b[3] as i8 })
        })
        .collect()
}

/// Model-space positions of every vertex (2 per definition: the even one and its mirror
/// partner). Plain points are exact; derived types are placed in 3D (a midpoint for the two
/// screen-midpoint types, the base point for the random type, t = ½ for the interpolated
/// type), which is *inferred* for a viewer that has no frame. Vertices that refer to the parent
/// model (negative numbers, `EL2MATH3:0c72`) stay `None`.
pub fn resolve(defs: &[VertexDef]) -> Vec<Option<[f32; 3]>> {
    let n = defs.len() * 2;
    let mut out: Vec<Option<Option<[f32; 3]>>> = vec![None; n];
    for v in 0..n {
        resolve_one(defs, v, &mut out, 0);
    }
    out.into_iter().map(|v| v.flatten()).collect()
}

fn resolve_one(defs: &[VertexDef], v: usize, memo: &mut [Option<Option<[f32; 3]>>], depth: u32) -> Option<[f32; 3]> {
    if let Some(r) = memo.get(v).copied().flatten() {
        return r;
    }
    if v >= memo.len() || depth > 32 {
        return None;
    }
    let d = defs[v / 2];
    let partner = v & 1 == 1;
    // For types above 10 the partner is built from the partners (bytes xor 1).
    let flip = |b: i8| -> usize { (b as u8 ^ partner as u8) as usize };
    let get = |b: i8, memo: &mut [Option<Option<[f32; 3]>>]| -> Option<[f32; 3]> {
        if b < 0 {
            return None;
        }
        resolve_one(defs, flip(b), memo, depth + 1)
    };
    let p = [d.x as f32, d.y as f32, d.z as f32];
    let r = match d.kind {
        k if k <= 2 => Some(if partner { [-p[0], p[1], p[2]] } else { p }),
        3 | 4 | 11..=14 => {
            let (a, b) = (get(d.z, memo), get(d.y, memo));
            a.zip(b).map(|(a, b)| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0])
        }
        5 | 6 => get(d.z, memo).map(|a| [-a[0], -a[1], -a[2]]),
        7 | 8 => get(d.z, memo),
        9 | 10 => Some(if partner { [0.0, p[1], p[2]] } else { p }),
        15 | 16 => {
            let (a, b, c) = (get(d.z, memo), get(d.y, memo), get(d.x, memo));
            match (a, b, c) {
                (Some(a), Some(b), Some(c)) => Some([c[0] - b[0] + a[0], c[1] - b[1] + a[1], c[2] - b[2] + a[2]]),
                _ => None,
            }
        }
        17 | 18 => get(d.z, memo).zip(get(d.y, memo)).map(|(a, b)| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]),
        19 | 20 => get(d.z, memo).zip(get(d.y, memo)).map(|(a, b)| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0]),
        _ => Some(if partner { [-p[0], -p[1], -p[2]] } else { p }),
    };
    memo[v] = Some(r);
    r
}

/// Opcode names, *inferred* from what each handler draws or tests.
pub fn op_name(op: u8) -> &'static str {
    match op {
        0x00 => "end",
        0x01 => "op01",
        0x02 => "line",
        0x03 => "triangle",
        0x04 => "quad",
        0x05 => "polygon",
        0x06 => "depth group",
        0x07 => "triangle + mirror",
        0x08 => "quad + mirror",
        0x09 => "op09",
        0x0a => "sub-object (0a)",
        0x0b => "skip if far",
        0x0c => "skip if near",
        0x0d | 0x1d => "variable op",
        0x0e => "sub-object",
        0x0f => "nop",
        0x13 => "skip unless flag",
        0x14 => "skip if flag",
        0x1f => "block",
        _ => "?",
    }
}

/// Operand words after the command word, for the opcodes with a fixed count. *Measured*: the
/// lengths of 1,247 consecutive commands the original fetched (`1000:b21e`, `b243`, `b268`)
/// while it drew the intro (`apps/viewer/examples/model_census.rs`, docs/FORMATS.md §4) agree
/// with this table and with [`length`]. `10`, `12` and `17` were not reached; their counts are
/// *observed* from the handlers' reads. The variable opcodes are in [`length`].
pub fn operands(op: u8) -> Option<usize> {
    Some(match op {
        0x00 | 0x0f | 0x15 | 0x19 | 0x1e => 0,
        0x02 | 0x0b | 0x0c | 0x0d | 0x12 | 0x13 | 0x14 | 0x1c | 0x1d => 1,
        0x01 | 0x03 | 0x07 | 0x09 | 0x17 | 0x1b => 2,
        0x04 | 0x08 | 0x0a | 0x10 | 0x16 => 3,
        0x11 => 5,
        _ => return None,
    })
}

/// The length in bytes of the command at `p`, the command word included, or `None` past the
/// end of the image. *Measured* for every opcode the intro uses (see [`operands`]):
/// `05` a polygon (sub-commands until sub-command 0); `06` one extra word when its parameter
/// (`word >> 5`) is 700h–7FDh; `0E` and `1A` one operand, plus 2 and 7 words when that
/// operand has bit 15 set; `18` words up to and including `7F7Fh`; `1F` two words and a block
/// of `word >> 5` words.
pub fn length(image: &[u8], p: usize) -> Option<usize> {
    let w = |p: usize| le16(image, p).ok();
    let word = w(p)?;
    let op = (word & 0x1f) as u8;
    let mut q = p + 2;
    Some(match op {
        0x05 => {
            q += 2;
            for _ in 0..512 {
                let s = w(q)?;
                q += 2;
                let sub = ((s >> 9) & 7) as u8;
                if sub == 0 {
                    break;
                }
                q += 2 * polygon_extra(sub);
            }
            q - p
        }
        0x06 => {
            if (0x700..=0x7fd).contains(&(word >> 5)) {
                4
            } else {
                2
            }
        }
        0x0e => {
            if w(q)? & 0x8000 != 0 {
                8
            } else {
                4
            }
        }
        0x1a => {
            if w(q)? & 0x8000 != 0 {
                18
            } else {
                4
            }
        }
        0x18 => {
            for _ in 0..512 {
                if w(q)? == 0x7f7f {
                    break;
                }
                q += 2;
            }
            q + 2 - p
        }
        0x1f => 6 + ((word >> 4) & !1) as usize,
        _ => 2 + 2 * operands(op)?,
    })
}

/// Extra words after a polygon sub-command, from the skip table at `CS:2902` (`1000:6b70`…).
/// The sub-command is bits 9–11 of the word (`1000:6968`); 0 ends the polygon.
fn polygon_extra(sub: u8) -> usize {
    match sub {
        1 => 2,
        2 | 4 | 6 => 1,
        _ => 0,
    }
}

/// Something the viewer can draw, with the command's 12-bit colour field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prim {
    Line([u8; 2]),
    /// Corners in drawing order, and the face's normal number (`face_shade`, [`normals`]).
    Tri([u8; 3], u8),
    Quad([u8; 4], u8),
    /// Outline of a polygon (*inferred*: the vertex bytes of sub-commands 2 and 3, and the
    /// four bytes of a sub-command 1 as curve control points).
    Polygon(Vec<u8>),
    /// Another model drawn at a vertex (`0E`: model number = command >> 5, vertex = the low
    /// byte of its operand, `1000:7c0d`).
    Sub {
        model: u16,
        at: u8,
    },
    /// `01`: a disc at a vertex (*inferred*: the handler projects one vertex, `1000:4991`), with
    /// the size word.
    Disc {
        at: u8,
        size: u16,
    },
    /// `09`: a thruster flame from one vertex towards another, with a size (*inferred*: the
    /// handler projects both vertices, `1000:726e`, `729c`; on the ships they point along all
    /// six axes from the hull, in colour `1EEh`).
    Thruster {
        from: u8,
        to: u8,
        size: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Image offset of the command word.
    pub offset: usize,
    pub word: u16,
    pub operands: Vec<u16>,
}

impl Command {
    pub fn op(&self) -> u8 {
        (self.word & 0x1f) as u8
    }
    /// `(word >> 4) & FFFh`, the colour field the drawing handlers decode (`1000:51c2`).
    pub fn colour(&self) -> u16 {
        (self.word >> 4) & 0xfff
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mesh {
    pub commands: Vec<Command>,
    pub prims: Vec<(Prim, u16)>,
    /// Why the walk stopped early, if it did.
    pub stopped: Option<String>,
}

/// Which way a walk takes the conditional commands (`0B`, `0C`, `13`, `14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branches {
    /// Follow every command in order: every level of detail at once. For listing a model.
    All,
    /// Take the branches the original takes for a model seen close up with all its variables
    /// zero (*assumed* state): `0B` (skip if far) falls through, `0C` (skip if near) skips, `13`
    /// skips when its variable is 0 (`1000:7f0b`), `14` falls through when it is 0
    /// (`1000:7f6a`). A skip of 0 bytes ends the model (`1000:7e3d`).
    Near,
}

/// The model colour while a model is drawn: the double word `[BP-9Eh]` (low word a colour, high
/// word a texture entry), here as one 12-bit colour field (`Exxh` for a texture, see
/// [`texture_entry`]). The object renderer starts it at `111h` (`1000:3abe`); a sub-object
/// starts with its parent's (`1000:7c2b`, `7daf`); opcode `1A` sets it ([`walk_with`]).
pub const START_COLOUR: u16 = 0x111;

/// Walk a mesh command list from `start` (image offset), following every command
/// ([`Branches::All`]), with the starting model colour and livery 0.
pub fn walk(image: &[u8], start: usize) -> Mesh {
    walk_with(image, start, Branches::All, &mut Paint { colour: START_COLOUR, livery: 0 })
}

/// The model colour ([`START_COLOUR`]) and the livery: the value (0–7) of the variable a `1A`
/// command consults. 0 keeps each `1A`'s own colour (*assumed* default; the game reads an
/// object variable, a ship's paint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paint {
    pub colour: u16,
    pub livery: u8,
}

/// The colour a drawing command uses: a colour with bit 4 set takes the model colour
/// (`1000:5203`–`5211`): its texture, or its colour added to the command's with bits 4 and 8
/// cleared (bit 8 means unshaded). Others are their own colour.
pub fn resolve_colour(c: u16, paint: &Paint) -> u16 {
    if c & 0x10 == 0 || texture_entry(c).is_some() {
        return c;
    }
    if texture_entry(paint.colour).is_some() {
        return paint.colour;
    }
    (c & !0x110).wrapping_add(paint.colour) & 0xfff
}

/// Walk a mesh command list from `start` (image offset), taking the conditional commands as
/// `branches` says and keeping the model colour in `paint`. Drawing commands get their
/// colours resolved ([`resolve_colour`]).
///
/// `1A` (`1000:8c8b`): its colour field, bit 0 cleared, is the new model colour; when the high
/// byte of its operand names a variable, seven alternative colours follow, and a variable value
/// v of 1–7 picks the v-th (`1000:8ce8`). Here v is `paint.livery`.
pub fn walk_with(image: &[u8], start: usize, branches: Branches, paint: &mut Paint) -> Mesh {
    let mut m = Mesh::default();
    let mut p = start;
    let w = |p: usize| le16(image, p).ok();
    while m.commands.len() < 4096 {
        let Some(word) = w(p) else {
            m.stopped = Some(format!("ran past the end of the image at {p:05x}"));
            break;
        };
        let at = p;
        p += 2;
        let op = (word & 0x1f) as u8;
        let mut ops = Vec::new();
        match op {
            0x05 => {
                let Some(first) = w(p) else { break };
                ops.push(first);
                p += 2;
                let mut outline = Vec::new();
                while let Some(s) = w(p) {
                    ops.push(s);
                    p += 2;
                    let sub = ((s >> 9) & 7) as u8;
                    if sub == 0 || ops.len() > 512 {
                        break;
                    }
                    let n = polygon_extra(sub);
                    let extra: Vec<u16> = (0..n).filter_map(|i| w(p + i * 2)).collect();
                    p += n * 2;
                    match sub {
                        2 | 3 => outline.push(s as u8),
                        1 if extra.len() == 2 => {
                            outline.extend([extra[0] as u8, (extra[0] >> 8) as u8, extra[1] as u8, (extra[1] >> 8) as u8])
                        }
                        _ => {}
                    }
                    ops.extend(extra);
                }
                m.prims.push((Prim::Polygon(outline), resolve_colour((word >> 4) & 0xfff, paint)));
            }
            _ => {
                let Some(len) = length(image, at) else {
                    m.commands.push(Command { offset: at, word, operands: ops });
                    m.stopped = Some(format!("opcode {op:02X} at {at:05x}: operand layout not traced"));
                    return m;
                };
                for i in 0..(len - 2) / 2 {
                    ops.push(w(p + i * 2).unwrap_or(0));
                }
                p = at + len;
                if branches == Branches::Near && matches!(op, 0x0b | 0x0c | 0x13 | 0x14) {
                    // 1000:7e37: skip (word >> 4) & !1 bytes after the operand; 0 ends the model.
                    // A negative operand of 0B / 0C picks another test (1000:7de0, 7e66): taken
                    // as falling through.
                    let skip = match op {
                        0x0b | 0x14 => false,
                        0x0c => ops[0] & 0x8000 == 0,
                        _ => true,
                    };
                    if skip {
                        let n = ((word >> 4) & !1) as usize;
                        m.commands.push(Command { offset: at, word, operands: ops });
                        if n == 0 {
                            return m;
                        }
                        p += n;
                        continue;
                    }
                }
                if op == 0x1a {
                    let mut c = (word >> 4) & 0xffe;
                    if ops[0] >> 8 != 0 && ops.len() == 8 && (1..=7).contains(&paint.livery) {
                        c = ops[paint.livery as usize] & 0xfff;
                    }
                    paint.colour = c;
                }
                let c = resolve_colour((word >> 4) & 0xfff, paint);
                let b = |w: u16| [w as u8, (w >> 8) as u8];
                match op {
                    0x01 => m.prims.push((Prim::Disc { at: ops[0] as u8, size: ops[1] }, c)),
                    0x02 => m.prims.push((Prim::Line(b(ops[0])), c)),
                    0x03 | 0x07 => {
                        // 1000:54c4: CX = (v1, v2), DX = (normal, v3). *Checked*: the vertex a
                        // face's normal names is one of these corners in 153 of 200 triangles.
                        let t = [ops[0] as u8, (ops[0] >> 8) as u8, (ops[1] >> 8) as u8];
                        let n = ops[1] as u8;
                        m.prims.push((Prim::Tri(t, n), c));
                        if op == 0x07 {
                            m.prims.push((Prim::Tri(t.map(|v| v ^ 1), n ^ 1), c));
                        }
                    }
                    0x04 | 0x08 => {
                        // 1000:5c6f: CX = (a, c), DX = (b, d), AX = normal. The corners go
                        // round as CL, DL, CH, DH (*measured* over the shipped models: 324 of
                        // 325 quads with four distinct plain corners are then flat and uncrossed,
                        // 142 in the byte order).
                        let q = [ops[0] as u8, ops[1] as u8, (ops[0] >> 8) as u8, (ops[1] >> 8) as u8];
                        let n = ops[2] as u8;
                        m.prims.push((Prim::Quad(q, n), c));
                        if op == 0x08 {
                            m.prims.push((Prim::Quad(q.map(|v| v ^ 1), n ^ 1), c));
                        }
                    }
                    0x09 => m.prims.push((Prim::Thruster { from: ops[0] as u8, to: (ops[0] >> 8) as u8, size: ops[1] }, c)),
                    0x0e => m.prims.push((Prim::Sub { model: word >> 5, at: ops[0] as u8 }, c)),
                    _ => {}
                }
            }
        }
        m.commands.push(Command { offset: at, word, operands: ops });
        if op == 0 {
            return m;
        }
    }
    if m.stopped.is_none() {
        m.stopped = Some("more than 4096 commands".into());
    }
    m
}

/// `1ea9:06aa` (second data segment): the table of texts `4000h`–`7FFFh`, word offsets relative
/// to the table, each to a `00h`-terminated string. `1000:18de` points `[DS:5703]` at it, and the
/// text expander reads text `4000h + n` through it ([TEXT.md](../../../docs/TEXT.md)). Entry 0 is
/// "CORE", 4 "Mine", 12h "Eagle long range fighter" … 2Bh "Long-range Cruiser" (*observed*).
pub const TEXTS_AT: usize = 0x1ea9 * 16 + 0x06aa;
/// Models 2–8 of the main table: missile and mine bodies and the missiles and mines drawn with
/// them (*inferred*: 5–8 all draw model 2, 4 draws model 3; they have no ship data block).
pub const WEAPONS: std::ops::RangeInclusive<usize> = 2..=8;
/// Models 42–58 of the main table: orbital stations (42–44) and surface starports, cities and
/// buildings (45–58). *Inferred* from drawing them (docs/FORMATS.md §4).
pub const STRUCTURES: std::ops::RangeInclusive<usize> = 42..=58;

/// The texts of `FRONTIER.EXE`'s table at [`TEXTS_AT`], by number (`4000h` + index). The table
/// ends where its first string begins.
pub fn texts(exe: &[u8]) -> Vec<String> {
    let Ok(h) = mz::parse(exe) else { return Vec::new() };
    let image = &exe[h.header_size().min(exe.len())..];
    let Ok(first) = le16(image, TEXTS_AT) else { return Vec::new() };
    (0..first as usize / 2)
        .map(|i| {
            let at = le16(image, TEXTS_AT + i * 2).map_or(usize::MAX, |o| TEXTS_AT + o as usize);
            let s = image.get(at..).unwrap_or_default();
            let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            String::from_utf8_lossy(&s[..end]).into_owned()
        })
        .collect()
}

/// The text number of a ship's name, from its data block (the header word `+1Ah`, relative to
/// the header; *observed*: `1ee8:63e3` reads it). The block starts with a list of tagged items:
/// `8080h` with 2 words, `80A0h` with 3, and `8010h`, `8050h` or `80D0h` with byte pairs up to a
/// word below 100h; a zero word (or a zero word ending a pair list) ends the list. Then come a
/// double word, a word, four words (mass in tonnes and internal capacity first) and the name as
/// text `4000h + n` ([`texts`]).
///
/// *Measured* on the runtime: the game drew model 16 on the F3 status screen of the player's
/// "Eagle long range fighter" (text 18), and models 27 (twice), 31 and 36 for a shipyard list of
/// two Constrictors, a Tiger Trader and a Puma Clipper (texts 29, 34, 33), with model 31 drawn
/// in the Tiger Trader's view; the Tiger Trader's page shows 400 t and 320 t, the two words
/// before its name (docs/FORMATS.md §4).
pub fn ship_name(image: &[u8], h: &Header) -> Option<u8> {
    let rel = *h.words.get(13)? as usize;
    if rel == 0 || rel >= 0x8000 {
        return None;
    }
    let w = |p: usize| le16(image, p).ok();
    let mut p = h.offset + rel;
    let mut ended = false;
    for _ in 0..64 {
        let tag = w(p)?;
        p += 2;
        match tag {
            0 => {
                ended = true;
                break;
            }
            0x8080 => p += 4,
            0x80a0 => p += 6,
            0x8010 | 0x8050 | 0x80d0 => loop {
                let v = w(p)?;
                p += 2;
                if v < 0x100 {
                    ended = v == 0;
                    break;
                }
            },
            _ => return None,
        }
        if ended {
            break;
        }
    }
    if !ended {
        return None;
    }
    let name = w(p + 14)?;
    (name >> 8 == 0x40).then_some(name as u8)
}

/// What a model is in the game, as far as the files tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// A ship or craft named by its data block ([`ship_name`]).
    Named(String),
    /// Missiles and mines ([`WEAPONS`], *inferred*).
    Weapon,
    /// Planets, stars and terrain: header scale 19 or more (*inferred*).
    World,
    /// Stations, starports, cities and buildings: [`STRUCTURES`] (*inferred*).
    Structure,
    /// Only drawn as part of other models (`0E` sub-objects; *observed* in the mesh lists).
    Part,
    Other,
}

impl Role {
    pub fn group(&self) -> &'static str {
        match self {
            Role::Named(_) => "Ships and craft",
            Role::Weapon => "Missiles and mines",
            Role::World => "Planets, stars and terrain",
            Role::Structure => "Stations, starports and buildings",
            Role::Part => "Parts",
            Role::Other => "Other",
        }
    }
}

/// The role of every model of a table. `texts` ([`texts`]) is given for the main table only;
/// pass an empty list for a module's table.
pub fn roles(image: &[u8], table: &Table, texts: &[String]) -> Vec<Option<Role>> {
    let mut used_as_part = vec![false; table.models.len()];
    for off in table.models.iter().flatten() {
        let Ok(h) = header(image, *off) else { continue };
        for (prim, _) in walk(image, off + h.mesh as usize).prims {
            if let Prim::Sub { model, .. } = prim {
                if let Some(u) = used_as_part.get_mut(model as usize) {
                    *u = true;
                }
            }
        }
    }
    table
        .models
        .iter()
        .enumerate()
        .map(|(n, off)| {
            let off = (*off)?;
            let h = header(image, off).ok()?;
            let main = !texts.is_empty();
            let name = ship_name(image, &h).and_then(|i| texts.get(i as usize)).filter(|_| main);
            let scale = h.scale().unwrap_or(0);
            Some(match name {
                Some(s) => Role::Named(s.clone()),
                None if main && WEAPONS.contains(&n) => Role::Weapon,
                None if main && STRUCTURES.contains(&n) => Role::Structure,
                None if scale >= 19 => Role::World,
                None if used_as_part[n] => Role::Part,
                None => Role::Other,
            })
        })
        .collect()
}

/// Face normals by normal number: number k is at `normals + (k >> 1) × 4 − 4`, bytes
/// `[x, a vertex on the face, z, y]`, odd k with x negated (*observed*, `EL2MATH3:1bcd`,
/// [MATH3.md](../../../docs/MATH3.md)). Numbers 0 and 1 have no normal: the face is always drawn.
pub fn normals(image: &[u8], h: &Header) -> Vec<Option<[f32; 3]>> {
    let count = h.normal_bytes as usize / 2;
    (0..count)
        .map(|k| {
            if k < 2 {
                return None;
            }
            let at = h.offset + h.normals as usize + (k >> 1) * 4 - 4;
            let b = image.get(at..at + 4)?;
            let x = b[0] as i8 as f32;
            Some([if k & 1 == 1 { -x } else { x }, b[3] as i8 as f32, b[2] as i8 as f32])
        })
        .collect()
}

/// `CS:25c4` in `FRONTIER.EXE`: the face texture table, 16 bytes per entry (entry 0 is not
/// used): a bitmap number of `EL2G.GRF` (`D0h`–`DCh`, 64 × 64, texels 0–6) and seven 12-bit
/// colour offsets, one per texel (*observed*: `1000:6037`–`60a8` adds texel t's offset to the
/// face's colour per channel, clamped at Fh, and hands the seven colours to the textured fills
/// through `DS:1fb1`, [VIDEO.md](../../../docs/VIDEO.md) §5.3).
pub const TEXTURES_AT: usize = 0x25c4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Texture {
    /// The `EL2G.GRF` entry (the bitmap table at `DS:5813`).
    pub bitmap: u16,
    pub offsets: [u16; 7],
}

/// The texture table of `FRONTIER.EXE`'s load image, from entry 1 up to the first entry whose
/// bitmap is not one of the 64 × 64 texture bitmaps.
pub fn textures(image: &[u8]) -> Vec<Option<Texture>> {
    let mut out = vec![None];
    for k in 1..64 {
        let at = TEXTURES_AT + k * 16;
        let Some(b) = image.get(at..at + 16) else { break };
        let w = |i: usize| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
        if !(0xd0..=0xdf).contains(&w(0)) {
            break;
        }
        out.push(Some(Texture { bitmap: w(0), offsets: std::array::from_fn(|i| w(i + 1)) }));
    }
    out
}

/// The texture entry a colour field names: colours `E00h`–`EFFh` (*observed*: `1000:54da`
/// moves bits 1–3 and 5–7 into the high word, which `1000:6037` uses as the entry number).
pub fn texture_entry(colour: u16) -> Option<u8> {
    (colour & 0xf00 == 0xe00).then_some((((colour >> 1) & 7) | (((colour >> 5) & 7) << 3)) as u8)
}

/// Add two 12-bit colours channel by channel, each capped at Fh (`1000:6055`–`60a0`).
pub fn add_rgb12(a: u16, b: u16) -> u16 {
    let ch = |s: u16| (((a >> s) & 15) + ((b >> s) & 15)).min(15) << s;
    ch(0) | ch(4) | ch(8)
}

/// The 12-bit colour field as 8-bit RGB (*inferred*: 4 bits per channel, `0RGB`).
pub fn rgb12(c: u16) -> [u8; 3] {
    [((c >> 8) & 15) as u8 * 17, ((c >> 4) & 15) as u8 * 17, (c & 15) as u8 * 17]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_pairs_and_derived_vertices() {
        let defs = [
            VertexDef { kind: 0, x: 10, y: 20, z: 30 },
            // average of vertex 0 and vertex 1 (bytes z=0, y=1)
            VertexDef { kind: 11, x: 0, y: 1, z: 0 },
        ];
        let v = resolve(&defs);
        assert_eq!(v[0], Some([10.0, 20.0, 30.0]));
        assert_eq!(v[1], Some([-10.0, 20.0, 30.0]));
        assert_eq!(v[2], Some([0.0, 20.0, 30.0]));
    }

    #[test]
    fn model_colour_and_textures() {
        let paint = Paint { colour: 0xe22, livery: 0 };
        // Bit 4 set: the model colour's texture; bit 4 clear: the command's own colour.
        assert_eq!(resolve_colour(0x0f0, &paint), 0xe22);
        assert_eq!(resolve_colour(0x0e0, &paint), 0x0e0);
        assert_eq!(texture_entry(0xe22), Some(9));
        assert_eq!(add_rgb12(0x9f8, 0x8a9), 0xfff);
    }

    #[test]
    fn walks_triangles_and_stops_at_end() {
        // tri (op 3) colour 0E0h, vertices 0,2 / normal 1, vertex 4; then end.
        let mut img = vec![];
        for w in [0x0e03u16, 0x0200, 0x0401, 0x0000] {
            img.extend_from_slice(&w.to_le_bytes());
        }
        let m = walk(&img, 0);
        assert_eq!(m.stopped, None);
        assert_eq!(m.prims, vec![(Prim::Tri([0, 2, 4], 1), 0x0e0)]);
    }
}
