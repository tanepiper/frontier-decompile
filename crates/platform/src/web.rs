//! Browser glue: canvas, status panel, and loading the player's game files (fetched from the dev
//! server with `?data=` or chosen with the file picker). Files never leave the browser.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::files::{has_program, File, SHIPPED};

/// Where loaded files are handed to the app (on the browser's event loop).
pub type OnFiles = std::rc::Rc<dyn Fn(Vec<File>)>;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Whether the canvas currently holds the browser's pointer lock.
pub fn pointer_locked() -> bool {
    document().pointer_lock_element().is_some()
}

/// The page's `<canvas>` with this id.
pub fn canvas(id: &str) -> web_sys::HtmlCanvasElement {
    document().get_element_by_id(id).unwrap().dyn_into().unwrap()
}

/// Show loading progress in the page's `#status`; hide `#panel` and focus the canvas once the
/// app is running.
pub fn set_status(text: &str, loaded: bool, canvas_id: &str) {
    if let Some(el) = document().get_element_by_id("status") {
        el.set_text_content(Some(text));
    }
    if loaded {
        if let Some(panel) = document().get_element_by_id("panel") {
            let _ = panel.set_attribute("hidden", "");
        }
        let _ = canvas(canvas_id).focus();
    }
}

/// `?data=<base>`: dev-server base URL for the git-ignored `game-data/` folder.
fn data_base() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get("data")
}

/// Whether `?name` (with any value) is in the page URL.
pub fn query_flag(name: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .is_some_and(|p| p.has(name))
}

pub fn load_from_query(on_files: OnFiles, status: fn(&str)) {
    let Some(base) = data_base() else {
        status("Select your game files to start.");
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let mut files = Vec::new();
        for name in SHIPPED {
            status(&format!("Fetching {name}…"));
            let url = format!("{base}original/{name}");
            let window = web_sys::window().unwrap();
            let result = async {
                let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url)).await?.dyn_into()?;
                if !resp.ok() {
                    return Err(JsValue::from_str(&format!("HTTP {}", resp.status())));
                }
                let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer()?).await?;
                Ok::<_, JsValue>(js_sys::Uint8Array::new(&buf).to_vec())
            }
            .await;
            match result {
                Ok(bytes) => files.push((name.to_string(), bytes)),
                Err(e) => log::warn!("{url}: {e:?}"),
            }
        }
        if !has_program(&files) {
            status("Could not fetch the game files. Select them to continue.");
            return;
        }
        on_files(files);
    });
}

/// Reads the files chosen in the `#files` input and hands them to the app.
pub fn install_file_picker(on_files: OnFiles, status: fn(&str)) {
    let input: web_sys::HtmlInputElement = document().get_element_by_id("files").unwrap().dyn_into().unwrap();
    let target = input.clone();
    let on_change = Closure::<dyn FnMut()>::new(move || {
        let Some(list) = target.files() else { return };
        if list.length() == 0 {
            return;
        }
        let files: Vec<web_sys::File> = (0..list.length()).filter_map(|i| list.get(i)).collect();
        target.set_value("");
        if !files.iter().any(|f| f.name().eq_ignore_ascii_case("FRONTIER.EXE")) {
            status("FRONTIER.EXE is missing: select every file from the game folder.");
            return;
        }
        target.set_disabled(true);
        let input = target.clone();
        let on_files = on_files.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut out = Vec::new();
            for file in files {
                status(&format!("Loading {}…", file.name()));
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => out.push((file.name(), js_sys::Uint8Array::new(&buf).to_vec())),
                    Err(e) => {
                        log::error!("{}: {e:?}", file.name());
                        status(&format!("Could not read {}. Select the files again.", file.name()));
                        input.set_disabled(false);
                        return;
                    }
                }
            }
            input.set_disabled(false);
            on_files(out);
        });
    });
    input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref()).unwrap();
    on_change.forget();
}

/// WebAudio output: stereo chunks scheduled back to back on one `AudioContext`. Browsers start
/// the context suspended until the page gets a click or key press; [`WebAudio::resume`] is called
/// from input events, and audio produced before that is dropped.
pub struct WebAudio {
    ctx: web_sys::AudioContext,
    /// Context time at which the next chunk starts.
    next: f64,
}

impl WebAudio {
    pub fn new() -> Option<WebAudio> {
        let ctx = web_sys::AudioContext::new().ok()?;
        Some(WebAudio { ctx, next: 0.0 })
    }

    pub fn rate(&self) -> f64 {
        self.ctx.sample_rate() as f64
    }

    pub fn resume(&self) {
        if self.ctx.state() != web_sys::AudioContextState::Running {
            let _ = self.ctx.resume();
        }
    }

    pub fn running(&self) -> bool {
        self.ctx.state() == web_sys::AudioContextState::Running
    }

    /// Seconds of audio scheduled but not yet played.
    pub fn queued(&self) -> f64 {
        (self.next - self.ctx.current_time()).max(0.0)
    }

    /// Schedule interleaved stereo `frames` after what is already queued.
    pub fn play(&mut self, frames: &[f32]) {
        let n = frames.len() / 2;
        if n == 0 || !self.running() {
            return;
        }
        let rate = self.ctx.sample_rate();
        let Ok(buffer) = self.ctx.create_buffer(2, n as u32, rate) else { return };
        let mut left: Vec<f32> = frames.iter().step_by(2).copied().collect();
        let mut right: Vec<f32> = frames.iter().skip(1).step_by(2).copied().collect();
        let _ = buffer.copy_to_channel(&mut left, 0);
        let _ = buffer.copy_to_channel(&mut right, 1);
        let Ok(source) = self.ctx.create_buffer_source() else { return };
        source.set_buffer(Some(&buffer));
        let _ = source.connect_with_audio_node(&self.ctx.destination());
        let now = self.ctx.current_time();
        // Restart the schedule after an underrun (tab in the background, a long frame).
        if self.next < now + 0.01 {
            self.next = now + crate::audio::TARGET_LATENCY;
        }
        let _ = source.start_with_when(self.next);
        self.next += n as f64 / rate as f64;
    }
}
