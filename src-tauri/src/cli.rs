use url::Url;

const LOGIN_FLOW: &str = "#?loginFlow=desktop";
const WEBUI_ARG: &str = "--webui-url=";

pub struct Options {
    pub web_ui_url: Url,
    pub launch_server: bool,
    pub serve_ui_through_server: bool,
    pub server_args: Vec<String>,
    pub media_to_open: Option<String>,
}

impl Options {
    pub fn from_args(args: &[String]) -> Self {
        let rest = args.get(1..).unwrap_or_default();
        let has = |flag: &str| rest.iter().any(|a| a == flag);
        let development = has("--development");

        let custom = rest.iter().find_map(|a| a.strip_prefix(WEBUI_ARG)).and_then(|u| Url::parse(u).ok());
        let is_custom = custom.is_some();
        let launch_server = !development || has("--streaming-server");
        let web_ui_url = custom.unwrap_or_else(|| {
            let url = if development {
                format!("http://127.0.0.1:11470/{LOGIN_FLOW}")
            } else if has("--staging") {
                format!("https://staging.strem.io/{LOGIN_FLOW}")
            } else {
                format!("https://app.strem.io/shell-v{}/{LOGIN_FLOW}", short_version())
            };
            Url::parse(&url).expect("built-in web UI URL is valid")
        });

        Self {
            web_ui_url,
            launch_server,
            serve_ui_through_server: launch_server && !development && !is_custom,
            server_args: rest.to_vec(),
            media_to_open: rest.first().filter(|a| !a.starts_with('-')).cloned(),
        }
    }
}

fn short_version() -> String {
    env!("CARGO_PKG_VERSION").split('.').take(2).collect::<Vec<_>>().join(".")
}

pub fn media_url(target: &str) -> String {
    if target.contains("://") || target.starts_with("magnet:") {
        target.to_string()
    } else {
        format!("file://{target}")
    }
}
