mod mpv;

#[cfg(target_os = "macos")]
#[path = "video_macos.rs"]
mod video;
#[cfg(not(target_os = "macos"))]
#[path = "video_unsupported.rs"]
mod video;

use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

use crate::transport;
use mpv::{Event, Mpv};

const OPTIONS: &[(&str, &str)] = &[
    ("terminal", "yes"),
    ("msg-level", "all=info"),
    ("vo", "libmpv"),
    ("gpu-hwdec-interop", "auto"),
    ("cache-secs", "10"),
    ("audio-client-name", "Stremio"),
    ("title", "Stremio"),
];

const BLOCKED_COMMANDS: &[&str] = &[
    "run",
    "subprocess",
    "load-script",
    "load-config-file",
    "load-input-conf",
    "quit",
    "quit-watch-later",
];

const BLOCKED_PROPERTIES: &[&str] = &["scripts", "script-opts", "input-ipc-server", "include", "input-conf"];

pub struct Player {
    mpv: Mpv,
}

pub fn init<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) -> Result<(), Box<dyn std::error::Error>> {
    let mpv = Mpv::new(OPTIONS)?;
    if let Err(err) = video::attach(window, mpv.raw()) {
        log::error!("video output unavailable: {err}");
    }
    if let Err(err) = mpv.observe_property("vid") {
        log::warn!("could not watch the video track: {err}");
    }
    app.manage(Player { mpv });

    let app = app.clone();
    std::thread::Builder::new()
        .name("mpv-events".into())
        .spawn(move || run_event_loop(&app))?;
    Ok(())
}

pub fn handle<R: Runtime>(app: &AppHandle<R>, ev: &str, args: &Value) {
    let Some(player) = app.try_state::<Player>() else {
        return;
    };
    let result = match ev {
        "mpv-command" => {
            let name = args.get(0).and_then(Value::as_str).unwrap_or_default();
            if BLOCKED_COMMANDS.contains(&name) {
                log::warn!("blocked mpv command from the web UI: {name}");
                return;
            }
            player.mpv.command(args)
        }
        "mpv-set-prop" => {
            let name = args.get(0).and_then(Value::as_str).unwrap_or_default();
            if BLOCKED_PROPERTIES.contains(&name) {
                log::warn!("blocked mpv property from the web UI: {name}");
                return;
            }
            player.mpv.set_property(name, args.get(1).unwrap_or(&Value::Null))
        }
        "mpv-observe-prop" => player.mpv.observe_property(args.as_str().unwrap_or_default()),
        _ => return,
    };
    if let Err(err) = result {
        log::warn!("mpv {ev} {args} failed: {err}");
    }
}

pub fn is_playing<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(player) = app.try_state::<Player>() else {
        return false;
    };
    let has_file = matches!(player.mpv.get_property("path"), Some(Value::String(_)));
    let paused = player.mpv.get_property("pause").and_then(|v| v.as_bool()).unwrap_or(true);
    has_file && !paused
}

pub fn on_window_changed<R: Runtime>(window: &WebviewWindow<R>) {
    video::refresh(window);
}

fn run_event_loop<R: Runtime>(app: &AppHandle<R>) {
    let player = app.state::<Player>();
    loop {
        match player.mpv.wait_event(-1.0) {
            Event::Shutdown => break,
            Event::PropertyChange { id, name, data, error } => {
                if name == "vid" {
                    video::set_has_video(matches!(data, Some(Value::Number(ref n)) if n.is_i64()));
                }
                let mut payload = Map::new();
                payload.insert("id".into(), json!(id));
                payload.insert("name".into(), json!(name));
                if let Some(data) = data {
                    payload.insert("data".into(), data);
                }
                if let Some(error) = error {
                    payload.insert("error".into(), json!(error));
                }
                transport::emit(app, "mpv-prop-change", Value::Object(payload));
            }
            Event::EndFile { id, reason, error } => {
                video::set_has_video(false);
                let mut payload = Map::new();
                payload.insert("id".into(), json!(id));
                payload.insert("reason".into(), json!(reason));
                if let Some(error) = error {
                    payload.insert("error".into(), json!(error));
                }
                transport::emit(app, "mpv-event-ended", Value::Object(payload));
            }
            Event::None | Event::Other => {}
        }
    }
}
