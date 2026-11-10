use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::cli::Options;
use crate::lifecycle::Lifecycle;
use crate::transport::{self, Transport};

const READY_PREFIX: &str = "EngineFS server started at ";
const MAX_REPORTED_CRASHES: u32 = 5;
const RESTART_DELAY: Duration = Duration::from_secs(10);
const STDERR_LINES_KEPT: usize = 200;
const STDERR_LINES_SHOWN: usize = 15;
const CRASH_MESSAGE: &str = "Error while starting streaming server. Please try to restart stremio. If it happens again please contact the Stremio support team for assistance";

#[derive(Default)]
pub struct Server {
    child: Mutex<Option<CommandChild>>,
    stderr: Mutex<VecDeque<String>>,
    reported_crashes: AtomicU32,
}

impl Server {
    fn stderr_log(&self) -> String {
        self.stderr.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n")
    }

    fn push_stderr(&self, line: String) {
        let mut lines = self.stderr.lock().unwrap();
        lines.push_back(line);
        if lines.len() > STDERR_LINES_KEPT {
            lines.pop_front();
        }
    }
}

pub fn start<R: Runtime>(app: &AppHandle<R>) {
    if let Err(err) = spawn(app) {
        log::error!("could not start the streaming server: {err}");
        report_crash(app, None, &err.to_string());
    }
}

pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(child) = app.state::<Server>().child.lock().unwrap().take() {
        if let Err(err) = child.kill() {
            log::warn!("could not stop the streaming server: {err}");
        }
    }
}

fn spawn<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let server_js = app.path().resolve("server.cjs", BaseDirectory::Resource)?;
    let tools_dir = sidecar_dir()?;
    let options = app.state::<Options>();

    let command = app
        .shell()
        .sidecar("node")?
        .arg(server_js)
        .args(&options.server_args)
        .env("FFMPEG_BIN", tools_dir.join(executable("ffmpeg")))
        .env("FFPROBE_BIN", tools_dir.join(executable("ffprobe")));

    let (mut events, child) = command.spawn()?;
    *app.state::<Server>().child.lock().unwrap() = Some(child);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut address_found = false;
        while let Some(event) = events.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    let line = String::from_utf8_lossy(&bytes);
                    let line = line.trim_end();
                    println!("{line}");
                    if !address_found {
                        if let Some(address) = line.strip_prefix(READY_PREFIX) {
                            address_found = true;
                            on_address(&app, address.trim());
                        }
                    }
                }
                CommandEvent::Stderr(bytes) => {
                    let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                    eprintln!("{line}");
                    app.state::<Server>().push_stderr(line);
                }
                CommandEvent::Error(err) => log::error!("streaming server: {err}"),
                CommandEvent::Terminated(status) => {
                    on_exit(&app, status.code, status.signal);
                    break;
                }
                _ => {}
            }
        }
    });
    Ok(())
}

fn on_address<R: Runtime>(app: &AppHandle<R>, address: &str) {
    app.state::<Transport>().set_server_address(address);
    transport::emit(app, "server-address", address);
    crate::web_ui::on_server_ready(app, address);
}

fn on_exit<R: Runtime>(app: &AppHandle<R>, code: Option<i32>, signal: Option<i32>) {
    app.state::<Server>().child.lock().unwrap().take();
    if app.state::<Lifecycle>().is_quitting() {
        return;
    }

    if code != Some(0) || signal.is_some() {
        let log = app.state::<Server>().stderr_log();
        report_crash(app, code.or(signal), &log);
    }

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(RESTART_DELAY);
        if !app.state::<Lifecycle>().is_quitting() {
            start(&app);
        }
    });
}

fn report_crash<R: Runtime>(app: &AppHandle<R>, code: Option<i32>, log: &str) {
    let server = app.state::<Server>();
    if server.reported_crashes.fetch_add(1, Ordering::SeqCst) >= MAX_REPORTED_CRASHES {
        return;
    }
    transport::queue(app, "server-crash", json!({ "code": code, "log": log }));

    let tail: Vec<&str> = log.lines().rev().take(STDERR_LINES_SHOWN).collect();
    let details = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    let code = code.map_or_else(|| "unknown".to_string(), |c| c.to_string());
    app.dialog()
        .message(format!("{CRASH_MESSAGE}\n\nExit code: {code}\n\n{details}"))
        .title("Stremio - Application Error")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

fn sidecar_dir() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe.parent().map(PathBuf::from).unwrap_or_default())
}

fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
