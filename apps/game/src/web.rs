//! Browser glue: canvas, status panel, and loading the player's game files (fetched from the dev
//! server with `?data=` or chosen with the file picker). Files never leave the browser.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Names the DOS release ships (docs/FINDINGS.md §1), for the dev-server fetch.
const SHIPPED: &[&str] = &[
    "FRONTIER.EXE", "EL2SETUP.OVL", "EL2MATH3.OVL", "EL2MCGA3.OVL", "EL2ADLIB.OVL", "EL2ROL.OVL", "EL2SPK.OVL",
    "EL2M00.OVL", "EL2M01.OVL", "EL2M02.OVL", "EL2M03.OVL", "EL2M04.OVL", "EL2M05.OVL", "EL2M06.OVL", "EL2M07.OVL",
    "EL2M08.OVL", "EL2M09.OVL", "EL2M10.OVL", "EL2M11.OVL", "EL2M12.OVL", "EL2M13.OVL", "EL2M14.OVL", "EL2M15.OVL",
    "EL2M16.OVL", "EL2M17.OVL", "EL2M18.OVL", "EL2M19.OVL", "EL2G.GRF", "PIC000.GRF", "INTRO.ROL", "JUPALT.ROL",
    "VALKRIES.ROL", "Babayaga.rol", "Barem.rol", "Bluedan.rol", "Elitet.rol", "Ggofkiev.rol", "Hallking.rol",
    "LAVE", "MARS", "MOON", "VTOL", "BATTLE",
];

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Whether the canvas currently holds the browser's pointer lock.
pub fn pointer_locked() -> bool {
    document().pointer_lock_element().is_some()
}

pub fn canvas() -> web_sys::HtmlCanvasElement {
    document().get_element_by_id("game").unwrap().dyn_into().unwrap()
}

/// Show loading progress in the HTML panel; hide the panel once the game is running.
pub fn set_status(text: &str, loaded: bool) {
    if let Some(el) = document().get_element_by_id("status") {
        el.set_text_content(Some(text));
    }
    if loaded {
        if let Some(panel) = document().get_element_by_id("panel") {
            let _ = panel.set_attribute("hidden", "");
        }
        let _ = canvas().focus();
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

pub fn load_from_query(proxy: EventLoopProxy<UserEvent>) {
    let Some(base) = data_base() else {
        set_status("Select your game files to start.", false);
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let mut files = Vec::new();
        for name in SHIPPED {
            set_status(&format!("Fetching {name}…"), false);
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
        if !files.iter().any(|(n, _)| n == "FRONTIER.EXE") {
            set_status("Could not fetch the game files. Select them to continue.", false);
            return;
        }
        let _ = proxy.send_event(UserEvent::Files(files));
    });
}

/// Reads the files chosen in the `#files` input and hands them to the app.
pub fn install_file_picker(proxy: EventLoopProxy<UserEvent>) {
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
            set_status("FRONTIER.EXE is missing: select every file from the game folder.", false);
            return;
        }
        target.set_disabled(true);
        let input = target.clone();
        let proxy = proxy.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut out = Vec::new();
            for file in files {
                set_status(&format!("Loading {}…", file.name()), false);
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => out.push((file.name(), js_sys::Uint8Array::new(&buf).to_vec())),
                    Err(e) => {
                        log::error!("{}: {e:?}", file.name());
                        set_status(&format!("Could not read {}. Select the files again.", file.name()), false);
                        input.set_disabled(false);
                        return;
                    }
                }
            }
            input.set_disabled(false);
            let _ = proxy.send_event(UserEvent::Files(out));
        });
    });
    input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref()).unwrap();
    on_change.forget();
}
