use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Runtime};
use url::Url;

use crate::cli::Options;
use crate::window::MAIN_WINDOW;

const STATIC_CAPABILITY_ORIGINS: &[&str] = &["https://app.strem.io", "https://staging.strem.io", "http://127.0.0.1:11470"];
const SERVER_WAIT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct WebUi {
    allowed_prefixes: Arc<Mutex<Vec<String>>>,
    granted_origins: Mutex<HashSet<String>>,
    loaded_url: Mutex<Option<Url>>,
}

impl WebUi {
    pub fn navigation_rules(&self) -> Arc<Mutex<Vec<String>>> {
        self.allowed_prefixes.clone()
    }
}

pub fn is_allowed(rules: &Mutex<Vec<String>>, url: &Url) -> bool {
    rules.lock().unwrap().iter().any(|prefix| url.as_str().starts_with(prefix))
}

pub fn initial_url<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Url> {
    let options = app.state::<Options>();
    if options.serve_ui_through_server {
        return Ok(Url::parse("about:blank").expect("about:blank is a valid URL"));
    }
    let url = options.web_ui_url.clone();
    prepare(app, &url, remote_prefix(&url))?;
    Ok(url)
}

pub fn start_fallback_timer<R: Runtime>(app: &AppHandle<R>) {
    if !app.state::<Options>().serve_ui_through_server {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SERVER_WAIT);
        if app.state::<WebUi>().loaded_url.lock().unwrap().is_none() {
            log::warn!("streaming server is not ready; loading the web UI directly");
            let url = app.state::<Options>().web_ui_url.clone();
            load(&app, url.clone(), remote_prefix(&url));
        }
    });
}

pub fn on_server_ready<R: Runtime>(app: &AppHandle<R>, server_address: &str) {
    let options = app.state::<Options>();
    if !options.serve_ui_through_server {
        return;
    }
    let Some((url, prefix)) = proxied(server_address, &options.web_ui_url) else {
        log::error!("could not build a web UI address from {server_address}");
        return;
    };
    let already_loaded = app
        .state::<WebUi>()
        .loaded_url
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|loaded| loaded.as_str().starts_with(&prefix));
    if !already_loaded {
        load(app, url, prefix);
    }
}

fn load<R: Runtime>(app: &AppHandle<R>, url: Url, prefix: String) {
    if let Err(err) = prepare(app, &url, prefix) {
        log::error!("could not allow {url} to talk to the shell: {err}");
        return;
    }
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    log::info!("Loading web UI from URL: {url}");
    *app.state::<WebUi>().loaded_url.lock().unwrap() = Some(url.clone());
    if let Err(err) = window.navigate(url) {
        log::error!("could not load the web UI: {err}");
    }
}

fn prepare<R: Runtime>(app: &AppHandle<R>, url: &Url, prefix: String) -> tauri::Result<()> {
    let state = app.state::<WebUi>();
    {
        let mut prefixes = state.allowed_prefixes.lock().unwrap();
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }

    let origin = url.origin().ascii_serialization();
    if STATIC_CAPABILITY_ORIGINS.contains(&origin.as_str()) {
        return Ok(());
    }
    let mut granted = state.granted_origins.lock().unwrap();
    if granted.contains(&origin) {
        return Ok(());
    }
    app.add_capability(
        CapabilityBuilder::new(format!("web-ui-{}", granted.len()))
            .remote(format!("{origin}/*"))
            .window(MAIN_WINDOW)
            .permission("allow-shell-props")
            .permission("allow-shell-event"),
    )?;
    granted.insert(origin);
    Ok(())
}

fn remote_prefix(url: &Url) -> String {
    format!("{}/", url.origin().ascii_serialization())
}

fn proxied(server_address: &str, remote: &Url) -> Option<(Url, String)> {
    let origin = remote.origin().ascii_serialization();
    let encoded: String = url::form_urlencoded::byte_serialize(origin.as_bytes()).collect();
    let prefix = format!("{}/proxy/d={encoded}/", server_address.trim_end_matches('/'));
    let mut target = format!("{}{}", prefix.trim_end_matches('/'), remote.path());
    if let Some(query) = remote.query() {
        target.push('?');
        target.push_str(query);
    }
    if let Some(fragment) = remote.fragment() {
        target.push('#');
        target.push_str(fragment);
    }
    Url::parse(&target).ok().map(|url| (url, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_proxied_web_ui_address() {
        let remote = Url::parse("https://app.strem.io/shell-v4.4/#?loginFlow=desktop").unwrap();
        let (url, prefix) = proxied("http://127.0.0.1:11470", &remote).unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:11470/proxy/d=https%3A%2F%2Fapp.strem.io/shell-v4.4/#?loginFlow=desktop"
        );
        assert_eq!(prefix, "http://127.0.0.1:11470/proxy/d=https%3A%2F%2Fapp.strem.io/");
    }

    #[test]
    fn only_the_web_ui_is_allowed_through_the_proxy() {
        let rules = Mutex::new(vec!["http://127.0.0.1:11470/proxy/d=https%3A%2F%2Fapp.strem.io/".to_string()]);
        let allowed = |u: &str| is_allowed(&rules, &Url::parse(u).unwrap());
        assert!(allowed("http://127.0.0.1:11470/proxy/d=https%3A%2F%2Fapp.strem.io/shell-v4.4/#/discover"));
        assert!(!allowed("http://127.0.0.1:11470/proxy/d=https%3A%2F%2Fevil.example/"));
        assert!(!allowed("http://127.0.0.1:11470/settings"));
        assert!(!allowed("https://app.strem.io.evil.example/"));
    }
}
