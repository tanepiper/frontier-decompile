//! The player's files and what the readers in `frontier-formats` make of them.

use std::collections::HashMap;

use frontier_formats::identify::{identify, Kind};
use frontier_formats::{grf, model, mz, overlay, smf, text};
use frontier_platform::files::File;

pub struct Resource {
    pub name: String,
    pub data: Vec<u8>,
    pub kind: Kind,
}

/// What a file parsed into. Parse errors are kept so the view can show them.
pub enum Parsed {
    Binary { mz: Result<mz::MzHeader, String>, overlay: Option<Result<overlay::Overlay, String>>, strings: Vec<text::Found> },
    Archive(Result<Vec<grf::Entry>, String>),
    Screenshot(Result<Box<grf::Screenshot>, String>),
    Midi(Result<smf::Smf, String>),
    Raw,
}

/// A model table and the image its offsets point into.
pub struct ModelSet {
    pub label: String,
    pub image: Vec<u8>,
    pub table: model::Table,
    /// What each model is (`model::roles`), by model number.
    pub roles: Vec<Option<model::Role>>,
}

impl ModelSet {
    fn new(label: String, image: Vec<u8>, table: model::Table, names: &[String]) -> ModelSet {
        let roles = model::roles(&image, &table, names);
        ModelSet { label, image, table, roles }
    }
}

pub struct Library {
    pub files: Vec<Resource>,
    parsed: HashMap<usize, Parsed>,
    /// 256 RGB colours for palette-indexed pictures, and where they come from.
    pub palette: [[u8; 3]; 256],
    pub palette_source: &'static str,
    pub models: Vec<ModelSet>,
    /// `FRONTIER.EXE`'s second model table: the fonts of `0A` text.
    pub fonts: Option<ModelSet>,
    /// The sine table (`model::sine_table`) and the texts `4000h`– (`model::texts`).
    pub sine: std::sync::Arc<Vec<i16>>,
    pub texts: Vec<String>,
    /// The face texture table of `FRONTIER.EXE` (`model::textures`), by entry.
    pub textures: Vec<Option<model::Texture>>,
    /// `EL2G.GRF`'s pictures by entry number, for the textures.
    pub bitmaps: Vec<Option<grf::Image>>,
}

impl Library {
    pub fn new(mut files: Vec<File>) -> Library {
        files.sort_by_key(|f| f.0.to_ascii_uppercase());
        let files: Vec<Resource> = files.into_iter().map(|(name, data)| Resource { kind: identify(&name, &data), name, data }).collect();
        // Assumption: the game sets its palette at run time (EL2MCGA3's colour allocator, not
        // traced); the palette saved in PIC000.GRF is one the game really used, so pictures are
        // shown with it. Without that file, a grey ramp.
        let mut palette = [[0u8; 3]; 256];
        for (i, c) in palette.iter_mut().enumerate() {
            *c = [i as u8; 3];
        }
        let mut palette_source = "grey ramp (no PIC000.GRF)";
        if let Some(pic) = files.iter().find(|f| f.kind == Kind::Screenshot) {
            if let Ok(s) = grf::parse_screenshot(&pic.data) {
                for (d, s) in palette.iter_mut().zip(s.palette) {
                    *d = s.map(grf::dac_to_8bit);
                }
                palette_source = "PIC000.GRF (a palette the game set)";
            }
        }
        let mut models = Vec::new();
        let (mut textures, mut fonts, mut sine, mut texts) = (Vec::new(), None, Vec::new(), Vec::new());
        if let Some(exe) = files.iter().find(|f| f.kind == Kind::Program) {
            if let Ok((image, _)) = model::main_table(&exe.data) {
                textures = model::textures(&image);
                sine = model::sine_table(&image);
                texts = model::texts(&exe.data);
                if let Ok(t) = model::text_table(&image) {
                    fonts = Some(ModelSet::new(format!("{} (fonts)", exe.name), image.clone(), t, &[]));
                }
            }
        }
        let bitmaps = files
            .iter()
            .find(|f| f.kind == Kind::ImageArchive)
            .and_then(|f| grf::parse_archive(&f.data).ok())
            .map(|e| e.iter().map(|e| e.image().and_then(Result::ok)).collect())
            .unwrap_or_default();
        if let Some(exe) = files.iter().find(|f| f.kind == Kind::Program) {
            if let Ok((image, table)) = model::main_table(&exe.data) {
                let texts = model::texts(&exe.data);
                models.push(ModelSet::new(format!("{} (in game)", exe.name), image, table, &texts));
            }
        }
        if let Some(m08) = files.iter().find(|f| f.name.eq_ignore_ascii_case("EL2M08.OVL")) {
            if let Ok(img) = overlay::image(&m08.data) {
                if let Ok(table) = model::table(img, model::EL2M08_BASE) {
                    models.push(ModelSet::new(format!("{} (title sequence)", m08.name), img.to_vec(), table, &[]));
                }
            }
        }
        Library {
            files,
            parsed: HashMap::new(),
            palette,
            palette_source,
            models,
            fonts,
            sine: std::sync::Arc::new(sine),
            texts,
            textures,
            bitmaps,
        }
    }

    pub fn find(&self, name: &str) -> Option<&Resource> {
        self.files.iter().find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// What file `i` parsed into ([`Self::ensure`] must have run for it).
    pub fn get(&self, i: usize) -> &Parsed {
        self.parsed.get(&i).unwrap_or(&Parsed::Raw)
    }

    /// Parse file `i` once.
    pub fn ensure(&mut self, i: usize) {
        let f = &self.files[i];
        self.parsed.entry(i).or_insert_with(|| {
            let e = |e: frontier_formats::Error| e.to_string();
            match f.kind {
                Kind::Program | Kind::Driver | Kind::Module => {
                    let header = mz::parse(&f.data).map_err(e);
                    let ovl = (f.kind != Kind::Program).then(|| overlay::parse(&f.data, f.kind == Kind::Module).map_err(e));
                    let image = header.as_ref().ok().map(|h| &f.data[h.header_size().min(f.data.len())..]).unwrap_or(&f.data);
                    Parsed::Binary { mz: header, overlay: ovl, strings: text::scan(image, 4) }
                }
                Kind::ImageArchive => Parsed::Archive(grf::parse_archive(&f.data).map_err(e)),
                Kind::Screenshot => Parsed::Screenshot(grf::parse_screenshot(&f.data).map(Box::new).map_err(e)),
                Kind::Midi => Parsed::Midi(smf::parse(&f.data).map_err(e)),
                Kind::Position | Kind::Unknown => Parsed::Raw,
            }
        });
    }

    /// What to build a model's scene with: `shown` uses the object variables measured for a ship
    /// on show ([`model::SHOWN_SHIP`]), otherwise all 0; `livery` overrides the paint variable.
    pub fn context<'a>(
        &'a self,
        set: &'a ModelSet,
        branches: model::Branches,
        shown: bool,
        livery: Option<u8>,
        registration: &'a str,
    ) -> crate::scene::Context<'a> {
        let mut paint = model::Paint::new(self.sine.clone());
        if shown {
            paint.object.copy_from_slice(&model::SHOWN_SHIP);
        }
        paint.livery = livery;
        crate::scene::Context { set, fonts: self.fonts.as_ref(), paint, branches, registration, texts: &self.texts }
    }

    /// Texture-table entry `entry` as RGBA on the 12-bit base colour `base`: texel t gets
    /// `base + offset t`, channel by channel (`model::add_rgb12`). `None` if the entry or its
    /// bitmap is missing.
    pub fn texture_rgba(&self, entry: u8, base: u16) -> Option<(usize, usize, Vec<u8>)> {
        let t = self.textures.get(entry as usize).copied().flatten()?;
        let img = self.bitmaps.get(t.bitmap as usize)?.as_ref()?;
        let mut out = Vec::with_capacity(img.pixels.len() * 4);
        for &p in &img.pixels {
            let c = model::add_rgb12(base, t.offsets.get(p as usize).copied().unwrap_or(0));
            let [r, g, b] = model::rgb12(c);
            out.extend_from_slice(&[r, g, b, 255]);
        }
        Some((img.width, img.height, out))
    }

    /// Palette indices as RGBA, colour 0 clear if `transparent`.
    pub fn rgba(&self, pixels: &[u8], transparent: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for &p in pixels {
            let c = self.palette[p as usize];
            out.extend_from_slice(&[c[0], c[1], c[2], if transparent && p == 0 { 0 } else { 255 }]);
        }
        out
    }
}
