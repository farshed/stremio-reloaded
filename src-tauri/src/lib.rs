mod cli;
mod dialogs;
mod events;
mod lifecycle;
mod player;
mod screensaver;
mod server;
mod transport;
mod tray;
mod web_ui;
mod window;

use tauri::{Manager, RunEvent};

use cli::Options;

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let options = Options::from_args(&args);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            window::on_second_instance(app, argv);
        }))
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(transport::Transport::new())
        .manage(lifecycle::Lifecycle::default())
        .manage(screensaver::Screensaver::default())
        .manage(server::Server::default())
        .manage(window::WindowState::default())
        .manage(web_ui::WebUi::default())
        .invoke_handler(tauri::generate_handler![transport::shell_props, transport::shell_event])
        .setup(move |app| {
            log::info!("Stremio Shell version: {}", app.package_info().version);
            let launch_server = options.launch_server;
            let media_to_open = options.media_to_open.clone();
            app.manage(options);

            let main_window = window::create(app.handle())?;
            tray::create(app.handle())?;
            if let Err(err) = player::init(app.handle(), &main_window) {
                log::error!("could not start the video player: {err}");
            }

            web_ui::start_fallback_timer(app.handle());
            if launch_server {
                server::start(app.handle());
            } else {
                log::info!("Skipping launch of streaming server under --development");
            }
            if let Some(target) = media_to_open {
                window::open_media(app.handle(), &target);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Stremio shell");

    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            window::show(app);
            for url in urls {
                window::open_media(app, url.as_str());
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => window::show(app),
        RunEvent::Exit => lifecycle::shutdown(app),
        _ => {}
    });
}
