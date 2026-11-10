use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::{dialogs, lifecycle, player, screensaver::Screensaver, transport, window};

pub fn handle<R: Runtime>(app: &AppHandle<R>, ev: &str, args: Value) {
    let screensaver = app.state::<Screensaver>();
    match ev {
        "app-ready" => transport::flush(app),
        "quit" => lifecycle::quit(app),
        "mpv-command" | "mpv-observe-prop" => player::handle(app, ev, &args),
        "mpv-set-prop" => {
            player::handle(app, ev, &args);
            if args.get(0).and_then(Value::as_str) == Some("pause") {
                let paused = args.get(1).and_then(Value::as_bool).unwrap_or(true);
                screensaver.set_inhibited(!paused);
            }
        }
        "control-event" | "wakeup" => screensaver.wakeup(app),
        "set-window-mode" => screensaver.set_inhibited(args.as_str() == Some("player")),
        "screensaver-toggle" => {
            let disabled = args.get("disabled").and_then(Value::as_bool).unwrap_or(false);
            screensaver.set_inhibited(disabled);
        }
        "open-external" => open_external(app, args.as_str().unwrap_or_default()),
        "win-focus" => {
            if !window::is_visible(app) {
                window::show(app);
            }
        }
        "win-set-visibility" => {
            if let Some(fullscreen) = args.get("fullscreen").and_then(Value::as_bool) {
                window::set_fullscreen(app, fullscreen);
            }
        }
        "file-open" => dialogs::open(app, args),
        "file-close" | "autoupdater-notif-clicked" | "balloon-show" => {}
        _ => log::debug!("unhandled web UI event: {ev}"),
    }
}

fn open_external<R: Runtime>(app: &AppHandle<R>, target: &str) {
    let allowed = url::Url::parse(target).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    if !allowed {
        log::warn!("refusing to open external target: {target}");
        return;
    }
    if let Err(err) = app.opener().open_url(target, None::<&str>) {
        log::warn!("could not open {target}: {err}");
    }
}
