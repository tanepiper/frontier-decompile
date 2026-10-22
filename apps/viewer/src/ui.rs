//! The viewer's interface: the file list, and a view for every kind of resource.

use std::collections::HashMap;

use egui::{Color32, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use frontier_formats::identify::Kind;
use frontier_formats::{grf, model, overlay, smf};

use crate::library::{Library, Parsed};
use crate::scene;
use crate::sound::{self, Player};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sel {
    File(usize),
    Models(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Content,
    Strings,
    Hex,
}

pub struct Ui {
    pub lib: Library,
    pub player: Player,
    sel: Option<Sel>,
    tab: Tab,
    entry: usize,
    model: usize,
    textures: HashMap<(usize, usize), TextureHandle>,
    scale: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    filled: bool,
    spin: bool,
    thrusters: bool,
    show_textures: bool,
    back_faces: bool,
    vertices: bool,
    all_branches: bool,
    livery: u8,
    shown: bool,
    registration: String,
    model_textures: HashMap<(u8, u16), TextureHandle>,
    message: String,
}

const MONO: f32 = 13.0;

/// The model list's groups, in order (`model::Role::group`).
const GROUPS: [&str; 6] =
    ["Ships and craft", "Missiles and mines", "Stations, starports and buildings", "Planets, stars and terrain", "Parts", "Other"];

impl Ui {
    pub fn new(lib: Library) -> Ui {
        Ui {
            lib,
            player: Player::new(),
            sel: None,
            tab: Tab::Content,
            entry: 0,
            model: 17,
            textures: HashMap::new(),
            scale: 3.0,
            yaw: 0.6,
            pitch: 0.35,
            zoom: 1.0,
            filled: true,
            spin: false,
            thrusters: false,
            show_textures: true,
            back_faces: false,
            vertices: false,
            all_branches: false,
            livery: 0,
            shown: true,
            registration: "WT-920".into(),
            model_textures: HashMap::new(),
            message: String::new(),
        }
    }

    /// Whether the view changes without input (a spinning model).
    pub fn animating(&self) -> bool {
        self.spin && matches!(self.sel, Some(Sel::Models(_)))
    }

    pub fn show(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("bar").show(root, |ui| self.top_bar(ui));
        egui::Panel::left("files").resizable(true).default_size(230.0).show(root, |ui| self.file_list(ui));
        egui::CentralPanel::default().show(root, |ui| match self.sel {
            None => {
                ui.heading("Frontier: Elite II resources");
                ui.label("Pick a file on the left. Models are listed under their tables.");
                ui.label(format!("{} files loaded. Pictures use the palette from {}.", self.lib.files.len(), self.lib.palette_source));
            }
            Some(Sel::File(i)) => self.file_view(ui, i),
            Some(Sel::Models(s)) => self.models_view(ui, s),
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Frontier resource viewer");
            ui.separator();
            if self.player.playing() {
                ui.label(format!("♪ {}", self.player.now_playing));
                if ui.button("Stop").clicked() {
                    self.player.stop();
                }
                ui.separator();
            }
            if !self.message.is_empty() {
                ui.colored_label(Color32::LIGHT_RED, &self.message);
            }
        });
    }

    fn file_list(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let groups = [
                ("Program", &[Kind::Program][..]),
                ("Drivers", &[Kind::Driver]),
                ("Modules", &[Kind::Module]),
                ("Graphics", &[Kind::ImageArchive, Kind::Screenshot]),
                ("Music", &[Kind::Midi]),
                ("Positions and saves", &[Kind::Position]),
                ("Other", &[Kind::Unknown]),
            ];
            for (title, kinds) in groups {
                let items: Vec<usize> = (0..self.lib.files.len()).filter(|&i| kinds.contains(&self.lib.files[i].kind)).collect();
                if items.is_empty() {
                    continue;
                }
                ui.label(RichText::new(title).strong());
                for i in items {
                    let f = &self.lib.files[i];
                    let text = format!("{}  ({} B)", f.name, f.data.len());
                    if ui.selectable_label(self.sel == Some(Sel::File(i)), text).clicked() {
                        self.sel = Some(Sel::File(i));
                        self.tab = Tab::Content;
                        self.entry = 0;
                    }
                }
                ui.add_space(6.0);
            }
            if !self.lib.models.is_empty() {
                ui.label(RichText::new("3D models").strong());
                for (s, set) in self.lib.models.iter().enumerate() {
                    let n = set.table.models.iter().flatten().count();
                    if ui.selectable_label(self.sel == Some(Sel::Models(s)), format!("{} ({n})", set.label)).clicked() {
                        self.sel = Some(Sel::Models(s));
                    }
                }
            }
        });
    }

    fn file_view(&mut self, ui: &mut egui::Ui, i: usize) {
        self.lib.ensure(i);
        let (name, kind, len) = {
            let f = &self.lib.files[i];
            (f.name.clone(), f.kind, f.data.len())
        };
        ui.horizontal(|ui| {
            ui.heading(&name);
            ui.label(format!("{} · {len} bytes", kind.label()));
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Content, "Content");
            if matches!(kind, Kind::Program | Kind::Driver | Kind::Module) {
                ui.selectable_value(&mut self.tab, Tab::Strings, "Strings");
            }
            ui.selectable_value(&mut self.tab, Tab::Hex, "Hex");
        });
        ui.separator();
        match self.tab {
            Tab::Hex => hex_view(ui, &self.lib.files[i].data),
            Tab::Strings => self.strings_view(ui, i),
            Tab::Content => match kind {
                Kind::Program | Kind::Driver | Kind::Module => self.binary_view(ui, i),
                Kind::ImageArchive => self.archive_view(ui, i),
                Kind::Screenshot => self.screenshot_view(ui, i),
                Kind::Midi => self.midi_view(ui, i),
                Kind::Position => {
                    ui.label("All ten of these files start with the bytes 11 00 C6 and the rest looks packed.");
                    ui.label("They are the start positions the menu offers (Mars, Lave, …) or saved games: inferred, the reader is not traced yet (docs/FORMATS.md §6).");
                    ui.separator();
                    hex_view(ui, &self.lib.files[i].data);
                }
                Kind::Unknown => hex_view(ui, &self.lib.files[i].data),
            },
        }
    }

    fn binary_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let name = self.lib.files[i].name.to_ascii_uppercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            binary_info(ui, self.lib.get(i));
            if name == "EL2ADLIB.OVL" {
                ui.separator();
                self.adlib_panel(ui, i);
            }
            if name == "EL2M08.OVL" || name == "FRONTIER.EXE" {
                ui.separator();
                ui.label("This file holds a model table: see “3D models” in the list.");
            }
        });
    }

    fn adlib_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.label(RichText::new("Music and effects (the original driver, ported, on the OPL2 model)").strong());
        let exe = self.lib.find("FRONTIER.EXE").map(|f| f.data.clone());
        let ovl = self.lib.files[i].data.clone();
        let mut start = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Songs:");
            for n in 0..sound::SONGS {
                if ui.button(format!("{n}")).clicked() {
                    start = Some((n, true));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Effects:");
            for n in 0..sound::EFFECTS {
                if ui.small_button(format!("{n}")).clicked() {
                    start = Some((n, false));
                }
            }
        });
        if let Some((n, song)) = start {
            if let Err(e) = self.player.play_adlib(&ovl, exe.as_deref(), n, song) {
                self.message = e;
            }
        }
    }

    fn strings_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let Parsed::Binary { strings, .. } = self.lib.get(i) else { return };
        ui.label(format!("{} runs of text in the load image (offsets from its start; control bytes shown as {{xx}})", strings.len()));
        let h = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both().auto_shrink(false).show_rows(ui, h, strings.len(), |ui, range| {
            for s in &strings[range] {
                ui.monospace(format!("{:05x}  {}", s.offset, s.text));
            }
        });
    }

    fn texture(&mut self, ctx: &egui::Context, key: (usize, usize), img: &grf::Image) -> TextureHandle {
        let lib = &self.lib;
        self.textures
            .entry(key)
            .or_insert_with(|| {
                let rgba = lib.rgba(&img.pixels, img.transparent);
                ctx.load_texture(
                    format!("img{key:?}"),
                    egui::ColorImage::from_rgba_unmultiplied([img.width.max(1), img.height.max(1)], &rgba),
                    TextureOptions::NEAREST,
                )
            })
            .clone()
    }

    fn archive_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let entries = match self.lib.get(i) {
            Parsed::Archive(Ok(e)) => e.clone(),
            Parsed::Archive(Err(e)) => {
                ui.colored_label(Color32::LIGHT_RED, e);
                return;
            }
            _ => return,
        };
        ui.horizontal(|ui| {
            ui.label(format!("{} entries, loaded by 1000:e2ba, drawn by EL2MCGA3:02d3.", entries.len()));
            ui.add(egui::Slider::new(&mut self.scale, 1.0..=6.0).text("zoom"));
        });
        if let Some(e) = entries.get(self.entry) {
            let h = e.header.unwrap();
            ui.monospace(format!(
                "#{:<3} file offset {:05x}  {}×{}  flags {:02x}{}{}  offset ({}, {})",
                self.entry,
                e.offset,
                h.width,
                h.height,
                h.flags,
                if h.flags & grf::FLAG_TRANSPARENT != 0 { " transparent" } else { "" },
                if h.flags & grf::FLAG_PACKED != 0 { " packed" } else { "" },
                h.dx,
                h.dy
            ));
        }
        ui.separator();
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (k, e) in entries.iter().enumerate() {
                    let Some(Ok(img)) = e.image() else { continue };
                    let tex = self.texture(&ctx, (i, k), &img);
                    let size = Vec2::new(img.width as f32, img.height as f32) * self.scale;
                    let r = ui.add(egui::Image::new(egui::load::SizedTexture::new(tex.id(), size)).sense(Sense::click()));
                    if k == self.entry {
                        ui.painter().rect_stroke(r.rect.expand(1.0), 0.0, Stroke::new(1.5, Color32::YELLOW), egui::StrokeKind::Outside);
                    }
                    if r.on_hover_text(format!("#{k}: {}×{}", img.width, img.height)).clicked() {
                        self.entry = k;
                    }
                }
            });
        });
    }

    fn screenshot_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let s = match self.lib.get(i) {
            Parsed::Screenshot(Ok(s)) => s.clone(),
            Parsed::Screenshot(Err(e)) => {
                ui.colored_label(Color32::LIGHT_RED, e);
                return;
            }
            _ => return,
        };
        ui.label("A screenshot the game wrote (1000:db9f): the VGA palette, then the 320×200 frame packed with PackBits.");
        ui.add(egui::Slider::new(&mut self.scale, 1.0..=6.0).text("zoom"));
        let img = grf::Image { width: grf::SCREEN_WIDTH, height: grf::SCREEN_HEIGHT, pixels: s.pixels.clone(), transparent: false };
        let tex = self.texture(&ui.ctx().clone(), (i, 0), &img);
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(egui::Image::new(egui::load::SizedTexture::new(tex.id(), Vec2::new(320.0, 200.0) * self.scale)));
            ui.label("Palette (6-bit DAC values):");
            let (rect, _) = ui.allocate_exact_size(Vec2::new(16.0 * 18.0, 16.0 * 18.0), Sense::hover());
            for (k, c) in s.palette.iter().enumerate() {
                let p = rect.min + Vec2::new((k % 16) as f32 * 18.0, (k / 16) as f32 * 18.0);
                let c = c.map(grf::dac_to_8bit);
                ui.painter().rect_filled(egui::Rect::from_min_size(p, Vec2::splat(16.0)), 0.0, Color32::from_rgb(c[0], c[1], c[2]));
            }
        });
    }

    fn midi_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let song = match self.lib.get(i) {
            Parsed::Midi(Ok(s)) => s.clone(),
            Parsed::Midi(Err(e)) => {
                ui.colored_label(Color32::LIGHT_RED, e);
                return;
            }
            _ => return,
        };
        let name = self.lib.files[i].name.clone();
        let length = song.timeline().last().map_or(0.0, |t| t.time);
        ui.label(format!(
            "Standard MIDI File, format {}, {} tracks, division {}, {:.0} s. Loaded by EL2ROL for a Roland MT-32 (docs/AUDIO.md §4).",
            song.format,
            song.tracks.len(),
            song.division,
            length
        ));
        ui.horizontal(|ui| {
            if ui.button("▶ Play on the OPL2 (approximation)").clicked() {
                if let Err(e) = self.player.play_midi(&name, &song) {
                    self.message = e;
                }
            }
            ui.label("The game plays these only on an MT-32; this is a single generic FM voice.");
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (t, track) in song.tracks.iter().enumerate() {
                let notes =
                    track.events.iter().filter(|e| matches!(e.kind, smf::EventKind::Channel { status, .. } if status >> 4 == 9)).count();
                ui.collapsing(format!("Track {t}: {} events, {notes} note-ons", track.events.len()), |ui| {
                    let mut tick = 0u64;
                    for e in track.events.iter().take(4000) {
                        tick += e.delta as u64;
                        ui.monospace(format!("{tick:>8}  {}", e.kind.describe()));
                    }
                    if track.events.len() > 4000 {
                        ui.label("… (first 4000 events shown)");
                    }
                });
            }
        });
    }

    fn models_view(&mut self, ui: &mut egui::Ui, s: usize) {
        let n = self.lib.models[s].table.models.len();
        egui::Panel::left("model list").resizable(true).default_size(240.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let set = &self.lib.models[s];
                for group in GROUPS {
                    let members: Vec<usize> =
                        (0..set.roles.len()).filter(|&k| set.roles[k].as_ref().is_some_and(|r| r.group() == group)).collect();
                    if members.is_empty() {
                        continue;
                    }
                    let open = group != "Parts" && group != "Other";
                    egui::CollapsingHeader::new(format!("{group} ({})", members.len())).id_salt((s, group)).default_open(open).show(
                        ui,
                        |ui| {
                            for k in members {
                                let label = match &set.roles[k] {
                                    Some(model::Role::Named(n)) => format!("{k:3}  {n}"),
                                    _ => {
                                        let v = set.table.models[k]
                                            .and_then(|o| model::header(&set.image, o).ok())
                                            .map_or(0, |h| h.vertex_count());
                                        format!("{k:3}  ({v} vertices)")
                                    }
                                };
                                if ui.selectable_label(self.model == k, label).clicked() {
                                    self.model = k;
                                }
                            }
                        },
                    );
                }
            });
        });
        let set = &self.lib.models[s];
        let Some(Some(off)) = set.table.models.get(self.model.min(n.saturating_sub(1))).copied() else {
            ui.label("Pick a model.");
            return;
        };
        let h = match model::header(&set.image, off) {
            Ok(h) => h,
            Err(e) => {
                ui.colored_label(Color32::LIGHT_RED, e.to_string());
                return;
            }
        };
        let defs = model::vertex_defs(&set.image, &h).unwrap_or_default();
        let pos = model::resolve(&defs);
        let mesh = model::walk(&set.image, off + h.mesh as usize);
        let branches = if self.all_branches { model::Branches::All } else { model::Branches::Near };
        let livery = (self.livery > 0).then_some(self.livery);
        let cx = self.lib.context(set, branches, self.shown, livery, &self.registration);
        let scene = scene::build(&cx, self.model);
        let title = match set.roles.get(self.model).cloned().flatten() {
            Some(model::Role::Named(n)) => format!("{n} (model {})", self.model),
            Some(r) => format!("Model {} · {}", self.model, r.group()),
            None => format!("Model {}", self.model),
        };
        ui.heading(title);
        // The controls wrap onto more lines, so the panel never grows wider than the window.
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.filled, "flat shaded");
            ui.checkbox(&mut self.spin, "spin");
            ui.checkbox(&mut self.thrusters, "thruster flames");
            ui.checkbox(&mut self.show_textures, "textures");
            ui.checkbox(&mut self.back_faces, "back faces");
            ui.checkbox(&mut self.vertices, "vertices").on_hover_text("The viewer's markers at every vertex; the game does not draw them");
            ui.checkbox(&mut self.all_branches, "every level of detail");
            ui.add(egui::Slider::new(&mut self.livery, 0..=7).text("livery (0 = from the variables)"));
            ui.checkbox(&mut self.shown, "ship on show (measured variables)");
            ui.add(egui::TextEdit::singleline(&mut self.registration).desired_width(70.0)).on_hover_text("Registration (text 3016h)");
            ui.add(egui::Slider::new(&mut self.zoom, 0.2..=5.0).text("zoom"));
        });
        match &mesh.stopped {
            None => ui.label(format!(
                "{} commands, {} shapes, {} sub-objects ({}); drag to turn, scroll to zoom.",
                mesh.commands.len(),
                scene.shapes.len(),
                scene.parts.len(),
                set.label
            )),
            Some(why) => ui.colored_label(Color32::from_rgb(230, 180, 90), format!("Partial: {why}. {} shapes drawn.", mesh.prims.len())),
        };
        // The drawing fills the visible width of the panel (not any wider content), so the model
        // is centred in what can be seen.
        let avail = ui.available_size();
        let width = avail.x.min(ui.clip_rect().right() - ui.cursor().left()).max(100.0);
        let view_h = (avail.y * 0.6).max(200.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, view_h), Sense::drag());
        if resp.dragged() {
            let d = resp.drag_delta();
            self.yaw += d.x * 0.01;
            self.pitch = (self.pitch + d.y * 0.01).clamp(-1.5, 1.5);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 5.0);
        }
        if self.spin {
            self.yaw += ui.input(|i| i.stable_dt).min(0.1) * 0.6;
        }
        let default = h.colour().unwrap_or(0x666);
        let options = scene::Options {
            filled: self.filled,
            thrusters: self.thrusters,
            textures: self.show_textures,
            back_faces: self.back_faces,
            vertices: self.vertices,
            yaw: self.yaw,
            pitch: self.pitch,
            zoom: self.zoom,
        };
        let ctx = ui.ctx().clone();
        let (lib, cache) = (&self.lib, &mut self.model_textures);
        let mut texture = |entry: u8, base: u16| -> Option<egui::TextureId> {
            if let Some(t) = cache.get(&(entry, base)) {
                return Some(t.id());
            }
            let (w, h, rgba) = lib.texture_rgba(entry, base)?;
            let t = ctx.load_texture(
                format!("tex{entry}-{base:03x}"),
                egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                TextureOptions::NEAREST,
            );
            let id = t.id();
            cache.insert((entry, base), t);
            Some(id)
        };
        scene::draw(&ui.painter_at(rect), rect, &scene, default, options, &mut texture);
        if matches!(set.roles.get(self.model), Some(Some(model::Role::Named(_)))) {
            ui.label("Name: from the ship's data block in FRONTIER.EXE (checked in the game for the Eagle, Constrictor, Tiger Trader and Puma Clipper; docs/FORMATS.md §4). Other groups are inferred.");
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.collapsing("Header", |ui| {
                ui.monospace(format!(
                    "at {:05x}: mesh +{:04x}, vertices +{:04x} ({} vertices), normals +{:04x} ({} bytes)",
                    h.offset,
                    h.mesh,
                    h.vertices,
                    h.vertex_count(),
                    h.normals,
                    h.normal_bytes
                ));
                ui.monospace(h.words.iter().enumerate().map(|(k, w)| format!("+{:02x}:{w:04x}", k * 2)).collect::<Vec<_>>().join(" "));
            });
            ui.collapsing(format!("Vertex definitions ({})", defs.len()), |ui| {
                for (k, d) in defs.iter().enumerate() {
                    let p = |v: Option<[f32; 3]>| v.map_or("—".to_string(), |p| format!("({:.0}, {:.0}, {:.0})", p[0], p[1], p[2]));
                    ui.monospace(format!(
                        "{:3}/{:<3} type {:3}  {:<36} → {} / {}",
                        2 * k,
                        2 * k + 1,
                        d.kind,
                        d.describe(),
                        p(pos[2 * k]),
                        p(pos[2 * k + 1])
                    ));
                }
            });
            ui.collapsing(format!("Mesh commands ({})", mesh.commands.len()), |ui| {
                for c in &mesh.commands {
                    ui.monospace(format!(
                        "{:05x}  {:04x}  {:02x} {:<18} colour {:03x}  {}",
                        c.offset,
                        c.word,
                        c.op(),
                        model::op_name(c.op()),
                        c.colour(),
                        c.operands.iter().map(|w| format!("{w:04x}")).collect::<Vec<_>>().join(" ")
                    ));
                }
            });
        });
    }
}

/// Offset, 16 bytes in hex and as text per row.
fn hex_view(ui: &mut egui::Ui, data: &[u8]) {
    let rows = data.len().div_ceil(16);
    let h = ui.text_style_height(&egui::TextStyle::Monospace).max(MONO);
    egui::ScrollArea::both().auto_shrink(false).show_rows(ui, h, rows, |ui, range| {
        for r in range {
            let row = &data[r * 16..(r * 16 + 16).min(data.len())];
            let hex: String = (0..16).map(|k| row.get(k).map_or("   ".into(), |b| format!("{b:02x} "))).collect();
            let txt: String = row.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
            ui.monospace(format!("{:06x}  {hex} {txt}", r * 16));
        }
    });
}

/// MZ header, relocations and the overlay export block or module header of a binary.
fn binary_info(ui: &mut egui::Ui, parsed: &Parsed) {
    let Parsed::Binary { mz, overlay, .. } = parsed else { return };
    match mz {
        Ok(h) => {
            egui::Grid::new("mz").striped(true).show(ui, |ui| {
                let rows = [
                    ("Header size", format!("{} bytes ({} paragraphs)", h.header_size(), h.header_paragraphs)),
                    ("Load image", format!("{} bytes", h.image_size())),
                    ("Entry CS:IP", format!("{:04x}:{:04x}", h.initial_cs, h.initial_ip)),
                    ("Stack SS:SP", format!("{:04x}:{:04x}", h.initial_ss, h.initial_sp)),
                    ("Extra memory", format!("min {:04x}h, max {:04x}h paragraphs", h.min_alloc, h.max_alloc)),
                    ("Relocations", format!("{}", h.relocations.len())),
                ];
                for (k, v) in rows {
                    ui.label(k);
                    ui.monospace(v);
                    ui.end_row();
                }
            });
            ui.collapsing(format!("Relocations ({})", h.relocations.len()), |ui| {
                for chunk in h.relocations.chunks(8) {
                    ui.monospace(chunk.iter().map(|(s, o)| format!("{s:04x}:{o:04x}")).collect::<Vec<_>>().join(" "));
                }
            });
        }
        Err(e) => {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
    }
    match overlay {
        Some(Ok(overlay::Overlay::Driver { first_slot, exports })) => {
            ui.separator();
            ui.label(RichText::new("Export block at 005e (docs/OVERLAYS.md §1)").strong());
            egui::Grid::new("exports").striped(true).show(ui, |ui| {
                ui.label("slot");
                ui.label("routine");
                ui.label("table");
                ui.end_row();
                for (k, f) in exports.iter().enumerate() {
                    let slot = first_slot + k as u16 * 4;
                    ui.monospace(format!("DS:{slot:04x}"));
                    ui.monospace(format!("{f}"));
                    ui.label(overlay::slot_owner(slot));
                    ui.end_row();
                }
            });
        }
        Some(Ok(overlay::Overlay::Module(m))) => {
            ui.separator();
            ui.label(RichText::new("Module header at 006a (docs/OVERLAYS.md §4)").strong());
            egui::Grid::new("module").striped(true).show(ui, |ui| {
                for (k, f) in m.handlers.iter().enumerate() {
                    ui.monospace(format!("handler +{:02x}", 4 + k * 4));
                    ui.monospace(format!("{f}"));
                    ui.end_row();
                }
                ui.label("first-load dword (H+00)");
                ui.monospace(format!("{:08x}", m.word0));
                ui.end_row();
                ui.label("slot dword (H+34)");
                ui.monospace(format!("{:08x}", m.word34));
                ui.end_row();
                ui.label(format!("byte pairs at {}", m.byte_pairs));
                ui.monospace(m.pairs.iter().map(|(a, b)| format!("{a:02x}{b:02x}")).collect::<Vec<_>>().join(" "));
                ui.end_row();
            });
        }
        Some(Err(e)) => {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        None => {}
    }
}
