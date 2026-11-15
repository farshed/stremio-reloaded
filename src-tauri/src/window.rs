use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::window::Color;
use tauri::{
    AppHandle, DragDropEvent, LogicalSize, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::cli;
use crate::transport::{self, Transport};
use crate::web_ui::{self, WebUi};
use crate::{player, tray};

pub const MAIN_WINDOW: &str = "main";
const APP_TITLE: &str = "Stremio Reloaded";
const BRIDGE_SCRIPT: &str = include_str!("bridge.js");
const WEB_UI_TWEAKS_SCRIPT: &str = include_str!("web_ui_tweaks.js");
#[cfg(any(target_os = "macos", target_os = "linux"))]
const WEBVIEW_BACKGROUND: Color = Color(0, 0, 0, 0);
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const WEBVIEW_BACKGROUND: Color = Color(0x0c, 0x0b, 0x11, 0xff);
const MIN_SIZE: (f64, f64) = (1000.0, 650.0);
const MAX_INITIAL_SIZE: (f64, f64) = (1600.0, 1000.0);
const EMBEDDABLE_HOSTS: &[&str] = &["www.youtube.com"];
const INTERNAL_SCHEMES: &[&str] = &["about", "data", "blob", "tauri", "ipc"];
const CLIPBOARD_SCHEMES: &[&str] = &["magnet:", "http:", "https:", "file:", "stremio:", "ipfs:"];

mod qt {
    pub const VISIBILITY_HIDDEN: u8 = 0;
    pub const VISIBILITY_WINDOWED: u8 = 2;
    pub const VISIBILITY_MINIMIZED: u8 = 3;
    pub const VISIBILITY_MAXIMIZED: u8 = 4;
    pub const VISIBILITY_FULLSCREEN: u8 = 5;

    pub const STATE_NONE: u8 = 0;
    pub const STATE_MINIMIZED: u8 = 1;
    pub const STATE_MAXIMIZED: u8 = 2;
    pub const STATE_FULLSCREEN: u8 = 4;

    pub const APP_INACTIVE: u8 = 2;
    pub const APP_ACTIVE: u8 = 4;
}

#[derive(Default)]
pub struct WindowState {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    last: Option<Status>,
    was_fullscreen: bool,
    always_on_top: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub visible: bool,
    pub fullscreen: bool,
    pub always_on_top: bool,
    visibility: u8,
    state: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStateChanged {
    state: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    clipboard: Option<String>,
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    let first_url = web_ui::initial_url(app)?;
    let rules = app.state::<WebUi>().navigation_rules();
    let (width, height) = initial_size(app);

    let window = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::External(first_url))
        .title(APP_TITLE)
        .inner_size(width, height)
        .min_inner_size(MIN_SIZE.0, MIN_SIZE.1)
        .center()
        .background_color(WEBVIEW_BACKGROUND)
        .transparent(cfg!(target_os = "linux"))
        .initialization_script(BRIDGE_SCRIPT)
        .initialization_script(WEB_UI_TWEAKS_SCRIPT)
        .on_navigation(move |url| is_navigation_allowed(&rules, url))
        .on_new_window({
            let app = app.clone();
            move |url, _| {
                if matches!(url.scheme(), "http" | "https") {
                    let _ = app.opener().open_url(url.as_str(), None::<&str>);
                }
                NewWindowResponse::Deny
            }
        })
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Started {
                window.app_handle().state::<Transport>().reset_queue();
            }
        })
        .build()?;

    let app = app.clone();
    let video_window = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&app);
        }
        WindowEvent::Resized(_) => {
            report_status(&app);
            player::on_window_changed(&video_window);
        }
        WindowEvent::ScaleFactorChanged { .. } => player::on_window_changed(&video_window),
        WindowEvent::Focused(focused) => {
            report_status(&app);
            report_app_state(&app, *focused);
        }
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
            let urls: Vec<String> = paths
                .iter()
                .filter_map(|p| Url::from_file_path(p).ok())
                .map(String::from)
                .collect();
            transport::emit(&app, "dragdrop", urls);
        }
        _ => {}
    });

    Ok(window)
}

pub fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let was_fullscreen = { app.state::<WindowState>().inner.lock().unwrap().was_fullscreen };
    let _ = window.show();
    let _ = window.unminimize();
    if was_fullscreen {
        let _ = window.set_fullscreen(true);
    }
    let _ = window.set_focus();
    report_status(app);
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
    report_status(app);
}

pub fn toggle_visible<R: Runtime>(app: &AppHandle<R>) {
    if is_visible(app) {
        hide(app);
    } else {
        show(app);
    }
}

pub fn set_fullscreen<R: Runtime>(app: &AppHandle<R>, fullscreen: bool) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.set_fullscreen(fullscreen);
    }
    app.state::<WindowState>().inner.lock().unwrap().was_fullscreen = fullscreen;
    report_status(app);
}

pub fn toggle_always_on_top<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let state = app.state::<WindowState>();
    let on_top = {
        let mut inner = state.inner.lock().unwrap();
        inner.always_on_top = !inner.always_on_top;
        inner.always_on_top
    };
    let _ = window.set_always_on_top(on_top);
    let _ = window.set_focus();
    report_status(app);
}

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

pub fn is_fullscreen<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .and_then(|w| w.is_fullscreen().ok())
        .unwrap_or(false)
}

pub fn open_media<R: Runtime>(app: &AppHandle<R>, target: &str) {
    transport::queue(app, "open-media", json!(cli::media_url(target)));
}

pub fn on_second_instance<R: Runtime>(app: &AppHandle<R>, argv: Vec<String>) {
    show(app);
    if let Some(target) = argv.get(1).filter(|a| !a.starts_with('-')) {
        open_media(app, target);
    }
}

fn report_status<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let state = app.state::<WindowState>();
    let mut inner = state.inner.lock().unwrap();

    let visible = window.is_visible().unwrap_or(false);
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let maximized = window.is_maximized().unwrap_or(false);

    if inner.always_on_top && (!visible || fullscreen) {
        inner.always_on_top = false;
        let _ = window.set_always_on_top(false);
    }

    let status = Status {
        visible,
        fullscreen,
        always_on_top: inner.always_on_top,
        visibility: match () {
            _ if !visible => qt::VISIBILITY_HIDDEN,
            _ if fullscreen => qt::VISIBILITY_FULLSCREEN,
            _ if minimized => qt::VISIBILITY_MINIMIZED,
            _ if maximized => qt::VISIBILITY_MAXIMIZED,
            _ => qt::VISIBILITY_WINDOWED,
        },
        state: match () {
            _ if fullscreen => qt::STATE_FULLSCREEN,
            _ if minimized => qt::STATE_MINIMIZED,
            _ if maximized => qt::STATE_MAXIMIZED,
            _ => qt::STATE_NONE,
        },
    };
    let previous = inner.last.replace(status);
    drop(inner);

    tray::update(app, status);
    if previous.map(|p| p.state) != Some(status.state) {
        transport::emit(app, "win-state-changed", json!({ "state": status.state }));
    }
    if previous.map(|p| (p.visible, p.visibility)) != Some((status.visible, status.visibility)) {
        transport::emit(
            app,
            "win-visibility-changed",
            json!({ "visible": status.visible, "visibility": status.visibility, "isFullscreen": status.fullscreen }),
        );
    }
}

fn report_app_state<R: Runtime>(app: &AppHandle<R>, focused: bool) {
    let clipboard = app
        .clipboard()
        .read_text()
        .ok()
        .filter(|text| CLIPBOARD_SCHEMES.iter().any(|scheme| text.starts_with(scheme)));
    let state = if focused { qt::APP_ACTIVE } else { qt::APP_INACTIVE };
    transport::emit(app, "app-state-changed", AppStateChanged { state, clipboard });
}

fn initial_size<R: Runtime>(app: &AppHandle<R>) -> (f64, f64) {
    let available = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.work_area().size.to_logical::<f64>(m.scale_factor()))
        .unwrap_or(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1));
    let width = (available.width * 0.8).min(MAX_INITIAL_SIZE.0).max(MIN_SIZE.0);
    let height = (available.height * 0.8).min(MAX_INITIAL_SIZE.1).max(MIN_SIZE.1);
    (width, height)
}

fn host_of(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        _ => String::new(),
    }
}

fn is_navigation_allowed(rules: &Mutex<Vec<String>>, url: &Url) -> bool {
    let allowed = match url.scheme() {
        "http" | "https" => web_ui::is_allowed(rules, url) || EMBEDDABLE_HOSTS.contains(&host_of(url).as_str()),
        scheme => INTERNAL_SCHEMES.contains(&scheme),
    };
    if !allowed {
        log::info!("blocked navigation to {url}");
    }
    allowed
}
