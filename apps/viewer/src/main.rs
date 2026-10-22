//! `frontier-viewer`: browse every resource in the player's own Frontier: Elite II files.
//!
//! Native:  `cargo run -p frontier-viewer --release [-- path/to/game]` (default
//!          `game-data/original`).
//! Browser: `tools/dev-web.sh --app viewer`, then open `/?data=/game-data/` or pick the files.
//!
//! What each view shows and how it is known is in docs/FORMATS.md. Nothing the viewer reads
//! leaves the machine or the browser.

mod gpu;
mod library;
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

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
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
