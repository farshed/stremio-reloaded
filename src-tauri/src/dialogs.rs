use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::transport;
use crate::window::MAIN_WINDOW;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileDialogRequest {
    title: String,
    select_existing: bool,
    select_folder: bool,
    select_multiple: bool,
    name_filters: Vec<String>,
    selected_name_filter: String,
    data: Value,
}

impl Default for FileDialogRequest {
    fn default() -> Self {
        Self {
            title: "Please choose".to_string(),
            select_existing: true,
            select_folder: false,
            select_multiple: false,
            name_filters: Vec::new(),
            selected_name_filter: String::new(),
            data: Value::Null,
        }
    }
}

pub fn open<R: Runtime>(app: &AppHandle<R>, args: Value) {
    let request: FileDialogRequest = serde_json::from_value(args).unwrap_or_default();

    let mut dialog = app.dialog().file().set_title(&request.title);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    for (name, extensions) in request.name_filters.iter().filter_map(|f| parse_name_filter(f)) {
        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(name, &extensions);
    }

    let app = app.clone();
    let (folder, existing, multiple) = (request.select_folder, request.select_existing, request.select_multiple);
    let finish = move |paths: Option<Vec<FilePath>>| respond(&app, request, paths);
    match (folder, existing, multiple) {
        (true, _, true) => dialog.pick_folders(finish),
        (true, _, false) => dialog.pick_folder(move |p| finish(p.map(|p| vec![p]))),
        (false, false, _) => dialog.save_file(move |p| finish(p.map(|p| vec![p]))),
        (false, true, true) => dialog.pick_files(finish),
        (false, true, false) => dialog.pick_file(move |p| finish(p.map(|p| vec![p]))),
    }
}

fn respond<R: Runtime>(app: &AppHandle<R>, request: FileDialogRequest, paths: Option<Vec<FilePath>>) {
    let echo = serde_json::to_value(&request).unwrap_or_default();
    match paths {
        Some(paths) => {
            let files: Vec<String> = paths
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let mut payload = echo;
            payload["files"] = json!(files);
            transport::emit(app, "file-selected", payload);
        }
        None => transport::emit(app, "file-rejected", echo),
    }
}

fn parse_name_filter(filter: &str) -> Option<(String, Vec<String>)> {
    let (name, patterns) = match filter.rsplit_once('(') {
        Some((name, rest)) => (name.trim(), rest.trim_end_matches(')')),
        None => (filter.trim(), filter),
    };
    let extensions: Vec<String> = patterns
        .split_whitespace()
        .filter_map(|p| p.strip_prefix("*."))
        .filter(|ext| !ext.is_empty() && *ext != "*")
        .map(str::to_string)
        .collect();
    if extensions.is_empty() {
        return None;
    }
    let name = if name.is_empty() { extensions.join(", ") } else { name.to_string() };
    Some((name, extensions))
}
