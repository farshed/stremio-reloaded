use std::sync::Mutex;
use std::time::Duration;

use keepawake::KeepAwake;
use tauri::{AppHandle, Manager, Runtime};

use crate::{player, window};

const WAKEUP_HOLD: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct Screensaver {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    guard: Option<KeepAwake>,
    wakeup_generation: u64,
}

impl Screensaver {
    pub fn set_inhibited(&self, inhibit: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inhibit == inner.guard.is_some() {
            return;
        }
        inner.guard = if inhibit {
            keepawake::Builder::default()
                .display(true)
                .idle(true)
                .reason("Playing video")
                .app_name("Stremio")
                .app_reverse_domain("io.github.farshed.stremio-reloaded")
                .create()
                .map_err(|err| log::warn!("could not block the screensaver: {err}"))
                .ok()
        } else {
            None
        };
    }

    pub fn wakeup<R: Runtime>(&self, app: &AppHandle<R>) {
        self.set_inhibited(true);
        let generation = {
            let mut inner = self.inner.lock().unwrap();
            inner.wakeup_generation += 1;
            inner.wakeup_generation
        };
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(WAKEUP_HOLD);
            let screensaver = app.state::<Screensaver>();
            let latest = screensaver.inner.lock().unwrap().wakeup_generation;
            if latest == generation {
                screensaver.set_inhibited(window::is_visible(&app) && player::is_playing(&app));
            }
        });
    }
}
