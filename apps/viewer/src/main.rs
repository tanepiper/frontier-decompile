//! `frontier-viewer`: browse every resource in the player's own Frontier: Elite II files.
//!
//! Native:  `cargo run -p frontier-viewer --release [-- path/to/game]` (default
//!          `game-data/original`).
//! Browser: `tools/dev-web.sh --app viewer`, then open `/?data=/game-data/` or pick the files.
//! Files:   `frontier-viewer --render TABLE N[,N…] OUT.png [--dir DIR] [--yaw R] [--pitch R]
//!          [--livery 0-7] [--wire] [--no-textures] [--back-faces] [--all-branches] [--thrusters]` draws models
//!          of TABLE (`main` for FRONTIER.EXE, `intro` for EL2M08) into a PNG, 320×240 each, for
//!          agents and tests that cannot look at a window.
//!
//! What each view shows and how it is known is in docs/FORMATS.md. Nothing the viewer reads
//! leaves the machine or the browser.

mod gpu;
mod library;
#[cfg(not(target_arch = "wasm32"))]
mod raster;
mod scene;
mod sound;
mod ui;

use std::sync::Arc;

use frontier_platform::files::File;
#[cfg(target_arch = "wasm32")]
use frontier_platform::web;
use gpu::Gpu;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Id of the page's canvas (index.html).
#[cfg(target_arch = "wasm32")]
const CANVAS: &str = "viewer";

pub enum UserEvent {
    Gpu(Box<Gpu>),
    Files(Vec<File>),
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui: Option<egui_winit::State>,
    ctx: egui::Context,
    ui: Option<ui::Ui>,
    pending: Option<Vec<File>>,
    last: Option<Instant>,
}

fn set_status(text: &str, loaded: bool) {
    if !text.is_empty() {
        log::info!("{text}");
    }
    #[cfg(target_arch = "wasm32")]
    web::set_status(text, loaded, CANVAS);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = loaded;
}

impl App {
    fn try_start(&mut self) {
        if self.gpu.is_none() {
            return;
        }
        let Some(files) = self.pending.take() else { return };
        let n = files.len();
        self.ui = Some(ui::Ui::new(library::Library::new(files)));
        set_status(&format!("{n} files"), true);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes().with_title("Frontier resource viewer");
        #[cfg(not(target_arch = "wasm32"))]
        {
            attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        }
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attrs = attrs.with_canvas(Some(web::canvas(CANVAS)));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());
        let proxy = self.proxy.clone();
        let init = async move {
            let gpu = Gpu::new(window).await;
            let _ = proxy.send_event(UserEvent::Gpu(Box::new(gpu)));
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(init);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(init);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Gpu(mut gpu) => {
                let window = self.window.as_ref().expect("window before GPU");
                let size = window.inner_size();
                gpu.resize(size.width, size.height);
                log::info!("GPU: {}", gpu.backend);
                self.egui = Some(egui_winit::State::new(
                    self.ctx.clone(),
                    egui::ViewportId::ROOT,
                    window.as_ref(),
                    Some(window.scale_factor() as f32),
                    None,
                    Some(gpu.max_texture_side()),
                ));
                self.gpu = Some(*gpu);
            }
            UserEvent::Files(files) => {
                set_status(&format!("Reading {} file(s)…", files.len()), false);
                self.pending = Some(files);
            }
        }
        self.try_start();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(window), Some(state)) = (&self.window, &mut self.egui) else {
            if let WindowEvent::CloseRequested = event {
                event_loop.exit();
            }
            return;
        };
        let response = state.on_window_event(window, &event);
        if response.repaint {
            window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(s.width, s.height);
                }
            }
            WindowEvent::RedrawRequested => {
                let (Some(gpu), Some(ui)) = (&mut self.gpu, &mut self.ui) else { return };
                let now = Instant::now();
                let dt = self.last.map_or(0.0, |t| (now - t).as_secs_f64());
                self.last = Some(now);
                ui.player.pump(dt);
                let input = state.take_egui_input(window);
                let full = self.ctx.run_ui(input, |root| ui.show(root));
                state.handle_platform_output(window, full.platform_output.clone());
                gpu.paint(full, &self.ctx);
                if ui.player.playing() || ui.animating() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn new_app(proxy: EventLoopProxy<UserEvent>) -> App {
    App { proxy, window: None, gpu: None, egui: None, ctx: egui::Context::default(), ui: None, pending: None, last: None }
}

/// `--render`: draw models into a PNG without a window.
#[cfg(not(target_arch = "wasm32"))]
fn render(args: &[String]) -> Result<(), String> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    // Positional arguments after `--render`, skipping the values of `--dir`, `--yaw`, `--pitch`.
    let mut pos: Vec<&String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" | "--yaw" | "--pitch" | "--livery" => i += 2,
            a if a.starts_with("--") => i += 1,
            _ => {
                pos.push(&args[i]);
                i += 1;
            }
        }
    }
    let [table, models, out] = [pos.first(), pos.get(1), pos.get(2)].map(|a| a.map(|s| s.as_str()));
    let (Some(table), Some(models), Some(out)) = (table, models, out) else {
        return Err("usage: --render TABLE N[,N…] OUT.png".into());
    };
    let dir = value("--dir").unwrap_or_else(|| "game-data/original".into());
    let lib = library::Library::new(frontier_platform::files::read_dir(std::path::Path::new(&dir)));
    let set = lib
        .models
        .iter()
        .find(|m| m.label.starts_with(if table == "intro" { "EL2M08" } else { "FRONTIER" }))
        .ok_or("no such model table")?;
    let numbers: Vec<usize> = models.split(',').filter_map(|n| n.parse().ok()).collect();
    let o = scene::Options {
        filled: !flag("--wire"),
        thrusters: flag("--thrusters"),
        textures: !flag("--no-textures"),
        back_faces: flag("--back-faces"),
        yaw: value("--yaw").and_then(|v| v.parse().ok()).unwrap_or(0.6),
        pitch: value("--pitch").and_then(|v| v.parse().ok()).unwrap_or(0.35),
        zoom: 1.0,
    };
    let branches = if flag("--all-branches") { frontier_formats::model::Branches::All } else { frontier_formats::model::Branches::Near };
    let (cw, ch, cols) = (320usize, 240usize, numbers.len().clamp(1, 4));
    let rows = numbers.len().div_ceil(cols);
    let mut sheet = raster::Canvas::new(cw * cols, ch * rows, egui::Color32::BLACK);
    for (k, &n) in numbers.iter().enumerate() {
        let default = set
            .table
            .models
            .get(n)
            .copied()
            .flatten()
            .and_then(|o| frontier_formats::model::header(&set.image, o).ok())
            .and_then(|h| h.colour())
            .unwrap_or(0x666);
        let livery = value("--livery").and_then(|v| v.parse().ok()).unwrap_or(0);
        let sc = scene::build(set, n, branches, livery);
        let items = scene::project(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(cw as f32, ch as f32)), &sc, default, o);
        let mut cell = raster::Canvas::new(cw, ch, egui::Color32::from_rgb(8, 10, 16));
        cell.draw(&items, &mut |e, base| lib.texture_rgba(e, base));
        sheet.blit(&cell, (k % cols) * cw, (k / cols) * ch);
    }
    std::fs::write(out, sheet.png()).map_err(|e| format!("{out}: {e}"))?;
    println!("wrote {out}: {} model(s) of {}", numbers.len(), set.label);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--render") {
        let rest: Vec<String> = args[args.iter().position(|a| a == "--render").unwrap()..].to_vec();
        if let Err(e) = render(&rest) {
            eprintln!("frontier-viewer: {e}");
            std::process::exit(2);
        }
        return;
    }
    let dir = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "game-data/original".into());
    let files = frontier_platform::files::read_dir(&dir);
    if files.is_empty() {
        eprintln!("frontier-viewer: no files in {} (run tools/extract-assets.sh or pass your game folder)", dir.display());
        std::process::exit(1);
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let mut app = new_app(event_loop.create_proxy());
    app.pending = Some(files);
    event_loop.run_app(&mut app).expect("run app");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();
    let send = proxy.clone();
    let on_files: web::OnFiles = std::rc::Rc::new(move |files| {
        let _ = send.send_event(UserEvent::Files(files));
    });
    let status: fn(&str) = |text| web::set_status(text, false, CANVAS);
    web::install_file_picker(on_files.clone(), status);
    web::load_from_query(on_files, status);
    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(new_app(proxy));
}
