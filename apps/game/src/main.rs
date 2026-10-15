//! `frontier`: Frontier: Elite II, native and in the browser.
//!
//! The game currently runs on the reference runtime (`frontier-dos`): the user's own
//! `FRONTIER.EXE` and overlays execute on the in-process 386 interpreter, and this app supplies
//! the window, the 320×200 frame, keyboard, mouse and timing. Native ports replace original
//! routines as they are traced (docs/RUNTIME.md). Legacy DOS set-up is skipped (DEVIATIONS.md).

mod input;
mod render;
#[cfg(target_arch = "wasm32")]
mod web;

use std::sync::Arc;

use frontier_dos::{Machine, Stop, Vfs};
use render::Renderer;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

pub enum UserEvent {
    Renderer(Box<Renderer>),
    Files(Vec<(String, Vec<u8>)>),
}

/// The running game and where its writes go.
struct Game {
    machine: Machine,
    stopped: Option<Stop>,
    rgba: Vec<u8>,
    #[cfg(not(target_arch = "wasm32"))]
    save_dir: Option<std::path::PathBuf>,
}

impl Game {
    fn new(files: Vec<(String, Vec<u8>)>) -> Result<Game, String> {
        let mut vfs = Vfs::new();
        for (name, data) in files {
            vfs.insert(&name, data);
        }
        let mut machine = Machine::new(vfs);
        machine.boot("FRONTIER.EXE")?;
        Ok(Game {
            machine,
            stopped: None,
            rgba: vec![0; (render::WIDTH * render::HEIGHT * 4) as usize],
            #[cfg(not(target_arch = "wasm32"))]
            save_dir: None,
        })
    }

    fn update(&mut self, dt: f64) {
        if self.stopped.is_some() {
            return;
        }
        let budget = (self.machine.dev.ips * dt) as u64;
        let st = self.machine.run(budget);
        let m = &mut self.machine;
        for line in m.log.drain(..) {
            log::info!("{line}");
        }
        if st != Stop::Budget {
            log::error!("game stopped: {st:?}");
            let recent: Vec<String> = m.cpu.recent().iter().rev().take(12).map(|(c, i)| format!("{c:04x}:{i:04x}")).collect();
            log::error!("recent: {}", recent.join(" "));
            self.stopped = Some(st);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.save_dir {
            for (name, data) in self.machine.vfs.take_dirty() {
                let path = dir.join(name.replace('\\', "/"));
                let r = match data {
                    Some(d) => std::fs::write(&path, d),
                    None => std::fs::remove_file(&path),
                };
                if let Err(e) = r {
                    log::warn!("{}: {e}", path.display());
                }
            }
        }
        self.machine.frame_rgba(&mut self.rgba);
    }
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    game: Option<Game>,
    pending: Option<Vec<(String, Vec<u8>)>>,
    #[cfg(not(target_arch = "wasm32"))]
    save_dir: Option<std::path::PathBuf>,
    last_frame: Option<Instant>,
    cursor: Option<(f64, f64)>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        App {
            proxy,
            window: None,
            renderer: None,
            game: None,
            pending: None,
            #[cfg(not(target_arch = "wasm32"))]
            save_dir: None,
            last_frame: None,
            cursor: None,
        }
    }

    fn try_start(&mut self) {
        if self.renderer.is_none() {
            return;
        }
        let Some(files) = self.pending.take() else { return };
        match Game::new(files) {
            Ok(game) => {
                #[cfg(not(target_arch = "wasm32"))]
                let game = Game { save_dir: self.save_dir.clone(), ..game };
                self.game = Some(game);
                set_status("", true);
            }
            Err(e) => set_status(&format!("Could not start: {e}"), false),
        }
    }

    fn send_key(&mut self, key: PhysicalKey, pressed: bool) {
        let (Some(game), PhysicalKey::Code(code)) = (&mut self.game, key) else { return };
        if let Some((ext, make)) = input::scancode(code) {
            if ext {
                game.machine.key(0xE0);
            }
            game.machine.key(if pressed { make } else { make | 0x80 });
        }
    }

    fn mouse_to_game(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        let r = self.renderer.as_ref()?;
        let game = self.game.as_ref()?;
        let (vx, vy, vw, vh) = r.viewport();
        let fx = ((x as f32 - vx) / vw).clamp(0.0, 1.0);
        let fy = ((y as f32 - vy) / vh).clamp(0.0, 1.0);
        let m = &game.machine.mouse;
        let gx = m.min_x + (fx * (m.max_x - m.min_x + 1) as f32) as i32;
        let gy = m.min_y + (fy * (m.max_y - m.min_y + 1) as f32) as i32;
        Some((gx.min(m.max_x), gy.min(m.max_y)))
    }
}

fn set_status(text: &str, loaded: bool) {
    if !text.is_empty() {
        log::info!("{text}");
    }
    #[cfg(target_arch = "wasm32")]
    web::set_status(text, loaded);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = loaded;
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes().with_title("Frontier: Elite II");
        #[cfg(not(target_arch = "wasm32"))]
        {
            attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(960.0, 720.0));
        }
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attrs = attrs.with_canvas(Some(web::canvas()));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        window.set_cursor_visible(false);
        self.window = Some(window.clone());
        let proxy = self.proxy.clone();
        let init = async move {
            let renderer = Renderer::new(window).await;
            let _ = proxy.send_event(UserEvent::Renderer(Box::new(renderer)));
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(init);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(init);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Renderer(mut renderer) => {
                if let Some(w) = &self.window {
                    let size = w.inner_size();
                    renderer.resize(size.width, size.height);
                }
                log::info!("GPU: {}", renderer.backend);
                self.renderer = Some(*renderer);
            }
            UserEvent::Files(files) => {
                set_status(&format!("Starting with {} file(s)…", files.len()), false);
                self.pending = Some(files);
            }
        }
        self.try_start();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if !event.repeat || event.state == ElementState::Pressed {
                    self.send_key(event.physical_key, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let target = self.mouse_to_game(position.x, position.y);
                self.cursor = Some((position.x, position.y));
                if let (Some(game), Some((gx, gy))) = (&mut self.game, target) {
                    let m = &mut game.machine;
                    let (dx, dy) = (gx - m.mouse.x, gy - m.mouse.y);
                    m.mouse_move(dx, dy);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(game) = &mut self.game {
                    let bit = match button {
                        MouseButton::Left => 1,
                        MouseButton::Right => 2,
                        MouseButton::Middle => 4,
                        _ => 0,
                    };
                    game.machine.mouse_button(bit, state == ElementState::Pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                let (Some(renderer), Some(game)) = (&mut self.renderer, &mut self.game) else { return };
                let now = Instant::now();
                let dt = self.last_frame.map_or(0.0, |t| (now - t).as_secs_f64()).min(0.1);
                self.last_frame = Some(now);
                game.update(dt);
                if let Some(Stop::Exited(_)) = game.stopped {
                    event_loop.exit();
                    return;
                }
                renderer.render(&game.rgba);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let (Some(w), Some(_)) = (&self.window, &self.game) {
            w.request_redraw();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_dir_files(root: &std::path::Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            read_dir_files(root, &r, out);
        } else if let Ok(d) = std::fs::read(e.path()) {
            out.push((r.replace('/', "\\"), d));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let arg = std::env::args().nth(1);
    // Default: the writable DOSBox copy (saves land there), else the read-only original.
    let dir = arg.map(std::path::PathBuf::from).unwrap_or_else(|| {
        let run = std::path::PathBuf::from("game-data/run");
        if run.join("FRONTIER.EXE").exists() { run } else { "game-data/original".into() }
    });
    let mut files = Vec::new();
    read_dir_files(&dir, "", &mut files);
    if !files.iter().any(|(n, _)| n.eq_ignore_ascii_case("FRONTIER.EXE")) {
        eprintln!("frontier: no FRONTIER.EXE in {} (run tools/extract-assets.sh or pass your game folder)", dir.display());
        std::process::exit(1);
    }
    let writable = !dir.ends_with("original");
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let mut app = App::new(event_loop.create_proxy());
    app.pending = Some(files);
    app.save_dir = writable.then_some(dir);
    event_loop.run_app(&mut app).expect("run app");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();
    web::install_file_picker(proxy.clone());
    web::load_from_query(proxy.clone());
    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(App::new(proxy));
}
