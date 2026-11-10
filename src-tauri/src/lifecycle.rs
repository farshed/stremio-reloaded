use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, Runtime};

use crate::server;

#[derive(Default)]
pub struct Lifecycle {
    quitting: AtomicBool,
}

impl Lifecycle {
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    pub fn mark_quitting(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }
}

pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    shutdown(app);
    app.exit(0);
}

pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Lifecycle>().mark_quitting();
    server::stop(app);
}
