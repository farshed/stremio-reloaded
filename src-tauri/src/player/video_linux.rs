use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use gtk::glib;
use gtk::prelude::*;
use libmpv2_sys as sys;
use tauri::{Runtime, WebviewWindow};

type GLint = c_int;

const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const BACKGROUND: [f32; 3] = [12.0 / 255.0, 11.0 / 255.0, 17.0 / 255.0];

#[derive(Clone, Copy)]
struct Gl {
    get_integerv: unsafe extern "C" fn(u32, *mut GLint),
    bind_framebuffer: unsafe extern "C" fn(u32, u32),
    clear_color: unsafe extern "C" fn(f32, f32, f32, f32),
    clear: unsafe extern "C" fn(u32),
}

struct RenderState {
    mpv: *mut sys::mpv_handle,
    context: *mut sys::mpv_render_context,
    gl: Option<Gl>,
    has_video: bool,
}

unsafe impl Send for RenderState {}

static STATE: Mutex<RenderState> = Mutex::new(RenderState {
    mpv: ptr::null_mut(),
    context: ptr::null_mut(),
    gl: None,
    has_video: false,
});

type ProcLookup = unsafe extern "C" fn(*const c_char) -> *mut c_void;
static PROC_LOOKUP: OnceLock<Option<ProcLookup>> = OnceLock::new();

thread_local! {
    static AREA: RefCell<Option<gtk::GLArea>> = const { RefCell::new(None) };
}

pub fn attach<R: Runtime>(window: &WebviewWindow<R>, mpv: *mut sys::mpv_handle) -> Result<(), String> {
    STATE.lock().unwrap().mpv = mpv;
    window
        .with_webview(|webview| {
            if let Err(err) = put_video_under(webview.inner().upcast_ref::<gtk::Widget>()) {
                log::error!("video output unavailable: {err}");
            }
        })
        .map_err(|e| e.to_string())
}

fn put_video_under(webview: &gtk::Widget) -> Result<(), String> {
    let parent = webview
        .parent()
        .and_then(|p| p.downcast::<gtk::Box>().ok())
        .ok_or("the web view is not inside a GtkBox")?;

    let area = new_video_area();
    let overlay = gtk::Overlay::new();
    overlay.add(&area);
    parent.remove(webview);
    overlay.add_overlay(webview);
    parent.pack_start(&overlay, true, true, 0);
    overlay.show_all();

    AREA.with(|cell| *cell.borrow_mut() = Some(area));
    Ok(())
}

fn new_video_area() -> gtk::GLArea {
    let area = gtk::GLArea::new();
    area.set_has_alpha(false);
    area.set_auto_render(false);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.connect_realize(|area| {
        area.make_current();
        if let Some(err) = area.error() {
            log::error!("could not create an OpenGL context for video: {err}");
        }
    });
    area.connect_unrealize(|area| {
        area.make_current();
        free_render_context();
    });
    area.connect_render(|area, _| {
        let scale = area.scale_factor();
        render_frame(area.allocated_width() * scale, area.allocated_height() * scale);
        // mpv can leave its own framebuffer bound.
        area.attach_buffers();
        glib::Propagation::Proceed
    });
    area
}

pub fn set_has_video(has_video: bool) {
    STATE.lock().unwrap().has_video = has_video;
    request_redraw();
}

pub fn refresh<R: Runtime>(_window: &WebviewWindow<R>) {
    request_redraw();
}

fn request_redraw() {
    glib::MainContext::default().invoke(|| {
        AREA.with(|cell| {
            if let Some(area) = cell.borrow().as_ref() {
                area.queue_render();
            }
        });
    });
}

extern "C" fn on_render_update(_ctx: *mut c_void) {
    request_redraw();
}

fn find_proc_lookup() -> Option<ProcLookup> {
    let wayland = gtk::gdk::Display::default().is_some_and(|d| d.type_().name().contains("Wayland"));
    let (library, symbol) = if wayland {
        (c"libEGL.so.1", c"eglGetProcAddress")
    } else {
        (c"libGL.so.1", c"glXGetProcAddressARB")
    };
    unsafe {
        let handle = libc::dlopen(library.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        if handle.is_null() {
            return None;
        }
        let lookup = libc::dlsym(handle, symbol.as_ptr());
        (!lookup.is_null()).then(|| std::mem::transmute::<*mut c_void, ProcLookup>(lookup))
    }
}

unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    match PROC_LOOKUP.get_or_init(find_proc_lookup) {
        Some(lookup) => lookup(name),
        None => libc::dlsym(libc::RTLD_DEFAULT, name),
    }
}

fn load_gl() -> Option<Gl> {
    unsafe fn load<T>(name: &CStr) -> Option<T> {
        let address = get_proc_address(ptr::null_mut(), name.as_ptr());
        (!address.is_null()).then(|| std::mem::transmute_copy(&address))
    }
    unsafe {
        Some(Gl {
            get_integerv: load(c"glGetIntegerv")?,
            bind_framebuffer: load(c"glBindFramebuffer")?,
            clear_color: load(c"glClearColor")?,
            clear: load(c"glClear")?,
        })
    }
}

fn render_frame(width: i32, height: i32) {
    let mut state = STATE.lock().unwrap();
    if state.gl.is_none() {
        state.gl = load_gl();
    }
    let Some(gl) = state.gl else {
        return;
    };
    let mut fbo: GLint = 0;
    unsafe { (gl.get_integerv)(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo) };

    if state.context.is_null() && !state.mpv.is_null() {
        state.context = create_render_context(state.mpv);
        // Creating the render context binds mpv's own framebuffer.
        unsafe { (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo as u32) };
    }

    if state.context.is_null() || !state.has_video {
        unsafe {
            (gl.clear_color)(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 1.0);
            (gl.clear)(GL_COLOR_BUFFER_BIT);
        }
        return;
    }

    unsafe { sys::mpv_render_context_update(state.context) };
    let mut target = sys::mpv_opengl_fbo {
        fbo,
        w: width,
        h: height,
        internal_format: 0,
    };
    let mut flip_y: c_int = 1;
    let mut params = [
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
            data: (&mut target as *mut sys::mpv_opengl_fbo).cast(),
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
            data: (&mut flip_y as *mut c_int).cast(),
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    unsafe { sys::mpv_render_context_render(state.context, params.as_mut_ptr()) };
}

fn free_render_context() {
    let mut state = STATE.lock().unwrap();
    if !state.context.is_null() {
        unsafe { sys::mpv_render_context_free(state.context) };
        state.context = ptr::null_mut();
    }
    state.gl = None;
}

fn create_render_context(mpv: *mut sys::mpv_handle) -> *mut sys::mpv_render_context {
    let mut init = sys::mpv_opengl_init_params {
        get_proc_address: Some(get_proc_address),
        get_proc_address_ctx: ptr::null_mut(),
    };
    let mut params = [
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
            data: sys::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
            data: (&mut init as *mut sys::mpv_opengl_init_params).cast(),
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let mut context: *mut sys::mpv_render_context = ptr::null_mut();
    let code = unsafe { sys::mpv_render_context_create(&mut context, mpv, params.as_mut_ptr()) };
    if code < 0 {
        log::error!("could not create the mpv render context: {}", super::mpv::error_string(code));
        return ptr::null_mut();
    }
    unsafe { sys::mpv_render_context_set_update_callback(context, Some(on_render_update), ptr::null_mut()) };
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    const GL_RGBA: u32 = 0x1908;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const SIZE: (i32, i32) = (320, 240);

    type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

    fn wait_for(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn video_area_renders_mpv_frames() {
        if gtk::init().is_err() {
            eprintln!("skipped: no display");
            return;
        }
        let mpv = super::super::mpv::Mpv::new(&[("vo", "libmpv"), ("ao", "null")]).expect("mpv starts");
        STATE.lock().unwrap().mpv = mpv.raw();

        let pixel = Rc::new(RefCell::new(None::<[u8; 4]>));
        let area = new_video_area();
        area.connect_render({
            let pixel = pixel.clone();
            move |area, _| {
                let read: ReadPixels = unsafe {
                    std::mem::transmute(get_proc_address(ptr::null_mut(), c"glReadPixels".as_ptr()))
                };
                let mut rgba = [0u8; 4];
                let scale = area.scale_factor();
                unsafe { read(SIZE.0 * scale / 2, SIZE.1 * scale / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_mut_ptr().cast()) };
                *pixel.borrow_mut() = Some(rgba);
                glib::Propagation::Stop
            }
        });
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(SIZE.0, SIZE.1);
        window.add(&area);
        window.show_all();
        AREA.with(|cell| *cell.borrow_mut() = Some(area.clone()));

        assert!(wait_for(10, || pixel.borrow().is_some()), "the video area never rendered");
        assert!(area.error().is_none(), "OpenGL error: {:?}", area.error());
        assert!(!STATE.lock().unwrap().context.is_null(), "render context was not created");
        assert_eq!(pixel.borrow().unwrap(), [12, 11, 17, 255], "idle area should show the background");

        mpv.command(&json!(["loadfile", "av://lavfi:testsrc=duration=3:size=320x240:rate=25"]))
            .expect("loadfile accepted");
        set_has_video(true);
        let drew_video = wait_for(10, || {
            pixel.borrow().is_some_and(|p| p != [12, 11, 17, 255] && p[..3].iter().any(|&c| c > 0))
        });
        assert!(drew_video, "no video frame was drawn: {:?}", pixel.borrow());

        window.close();
        while glib::MainContext::default().iteration(false) {}
        let mut state = STATE.lock().unwrap();
        state.mpv = ptr::null_mut();
        state.has_video = false;
    }
}
