use libmpv2_sys::mpv_handle;
use tauri::{Runtime, WebviewWindow};

pub fn attach<R: Runtime>(_window: &WebviewWindow<R>, _mpv: *mut mpv_handle) -> Result<(), String> {
    Err("video output is only implemented on macOS and Linux so far".into())
}

pub fn set_has_video(_has_video: bool) {}

pub fn refresh<R: Runtime>(_window: &WebviewWindow<R>) {}
