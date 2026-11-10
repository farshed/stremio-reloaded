use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::{lifecycle, window};

const SHOW_ID: &str = "show-window";
const ON_TOP_ID: &str = "always-on-top";
const QUIT_ID: &str = "quit";

pub struct Tray<R: Runtime> {
    show_window: CheckMenuItem<R>,
    always_on_top: CheckMenuItem<R>,
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show_window = CheckMenuItem::with_id(app, SHOW_ID, "Show window", true, true, None::<&str>)?;
    let always_on_top = CheckMenuItem::with_id(app, ON_TOP_ID, "Always on top", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_window, &always_on_top, &quit])?;

    let builder = TrayIconBuilder::with_id("main")
        .tooltip("Stremio")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => window::toggle_visible(app),
            ON_TOP_ID => window::toggle_always_on_top(app),
            QUIT_ID => lifecycle::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if !cfg!(target_os = "macos") {
                    window::show(tray.app_handle());
                }
            }
        });

    let builder = if cfg!(target_os = "macos") {
        builder
            .icon(Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
            .icon_as_template(true)
    } else if let Some(icon) = app.default_window_icon() {
        builder.icon(icon.clone())
    } else {
        builder
    };
    builder.build(app)?;

    app.manage(Tray { show_window, always_on_top });
    Ok(())
}

pub fn update<R: Runtime>(app: &AppHandle<R>, status: window::Status) {
    let Some(tray) = app.try_state::<Tray<R>>() else {
        return;
    };
    let _ = tray.show_window.set_checked(status.visible);
    let _ = tray.always_on_top.set_enabled(status.visible && !status.fullscreen);
    let _ = tray.always_on_top.set_checked(status.always_on_top);
}
