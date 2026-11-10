use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use crate::window::{self, MAIN_WINDOW};

pub const DEFAULT_SERVER_ADDRESS: &str = "http://127.0.0.1:11470";

pub struct Transport {
    inner: Mutex<Inner>,
}

struct Inner {
    queued: Option<Vec<(String, Value)>>,
    server_address: String,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                queued: Some(Vec::new()),
                server_address: DEFAULT_SERVER_ADDRESS.to_string(),
            }),
        }
    }

    pub fn set_server_address(&self, address: &str) {
        self.inner.lock().unwrap().server_address = address.to_string();
    }

    pub fn reset_queue(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queued.is_none() {
            inner.queued = Some(Vec::new());
        }
    }
}

pub fn emit<R: Runtime>(app: &AppHandle<R>, ev: &str, args: impl Serialize) {
    let Some(webview) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let (Ok(ev), Ok(args)) = (serde_json::to_string(ev), serde_json::to_string(&args)) else {
        return;
    };
    if let Err(err) = webview.eval(format!("window.__stremioShellEmit && window.__stremioShellEmit({ev}, {args})")) {
        log::warn!("could not send {ev} to the web UI: {err}");
    }
}

pub fn queue<R: Runtime>(app: &AppHandle<R>, ev: &str, args: Value) {
    {
        let transport = app.state::<Transport>();
        let mut inner = transport.inner.lock().unwrap();
        if let Some(queued) = inner.queued.as_mut() {
            queued.push((ev.to_string(), args));
            return;
        }
    }
    emit(app, ev, args);
}

pub fn flush<R: Runtime>(app: &AppHandle<R>) {
    let queued = app.state::<Transport>().inner.lock().unwrap().queued.take();
    for (ev, args) in queued.unwrap_or_default() {
        emit(app, &ev, args);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellProps {
    shell_version: String,
    server_address: String,
    is_fullscreen: bool,
}

#[tauri::command]
pub fn shell_props<R: Runtime>(app: AppHandle<R>, transport: tauri::State<'_, Transport>) -> ShellProps {
    ShellProps {
        shell_version: app.package_info().version.to_string(),
        server_address: transport.inner.lock().unwrap().server_address.clone(),
        is_fullscreen: window::is_fullscreen(&app),
    }
}

#[tauri::command]
pub fn shell_event<R: Runtime>(app: AppHandle<R>, ev: String, args: Value) {
    crate::events::handle(&app, &ev, args);
}
