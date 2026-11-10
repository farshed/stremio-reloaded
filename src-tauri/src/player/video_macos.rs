#![allow(deprecated)]

use std::ffi::{c_char, c_int, c_void};
use std::ptr::{self, NonNull};
use std::sync::{Mutex, OnceLock};

use dispatch2::{DispatchQueue, MainThreadBound};
use libmpv2_sys as sys;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSView, NSWindow, NSWindowOrderingMode};
use objc2_core_foundation::CFTimeInterval;
use objc2_core_video::CVTimeStamp;
use objc2_open_gl::{
    CGLChoosePixelFormat, CGLContextObj, CGLOpenGLProfile, CGLPixelFormatAttribute, CGLPixelFormatObj,
};
use objc2_quartz_core::{CALayer, CAOpenGLLayer};
use tauri::{Runtime, WebviewWindow};

type GLint = c_int;

const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_VIEWPORT: u32 = 0x0BA2;
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

#[link(name = "OpenGL", kind = "framework")]
extern "C" {
    fn glGetIntegerv(pname: u32, data: *mut GLint);
    fn glClearColor(red: f32, green: f32, blue: f32, alpha: f32);
    fn glClear(mask: u32);
}

struct RenderState {
    mpv: *mut sys::mpv_handle,
    context: *mut sys::mpv_render_context,
    has_video: bool,
}

unsafe impl Send for RenderState {}

static STATE: Mutex<RenderState> = Mutex::new(RenderState {
    mpv: ptr::null_mut(),
    context: ptr::null_mut(),
    has_video: false,
});

static LAYER: OnceLock<MainThreadBound<Retained<VideoLayer>>> = OnceLock::new();

define_class!(
    #[unsafe(super(CAOpenGLLayer, CALayer, objc2_foundation::NSObject))]
    #[name = "StremioVideoLayer"]
    struct VideoLayer;

    impl VideoLayer {
        #[unsafe(method(copyCGLPixelFormatForDisplayMask:))]
        fn copy_pixel_format(&self, mask: u32) -> CGLPixelFormatObj {
            choose_pixel_format(mask)
                .unwrap_or_else(|| unsafe { msg_send![super(self), copyCGLPixelFormatForDisplayMask: mask] })
        }

        #[unsafe(method(canDrawInCGLContext:pixelFormat:forLayerTime:displayTime:))]
        fn can_draw(
            &self,
            _ctx: CGLContextObj,
            _pf: CGLPixelFormatObj,
            _time: CFTimeInterval,
            _ts: *const CVTimeStamp,
        ) -> bool {
            true
        }

        #[unsafe(method(drawInCGLContext:pixelFormat:forLayerTime:displayTime:))]
        fn draw(&self, ctx: CGLContextObj, pf: CGLPixelFormatObj, time: CFTimeInterval, ts: *const CVTimeStamp) {
            render_frame();
            unsafe {
                let _: () = msg_send![
                    super(self),
                    drawInCGLContext: ctx,
                    pixelFormat: pf,
                    forLayerTime: time,
                    displayTime: ts
                ];
            }
        }
    }
);

pub fn attach<R: Runtime>(window: &WebviewWindow<R>, mpv: *mut sys::mpv_handle) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("video output must be set up on the main thread")?;
    let ns_window = window.ns_window().map_err(|e| e.to_string())?;
    let ns_window: &NSWindow = unsafe { &*ns_window.cast::<NSWindow>() };
    let content = ns_window.contentView().ok_or("window has no content view")?;
    let webview = content.subviews().firstObject();

    ns_window.setOpaque(true);
    ns_window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        12.0 / 255.0,
        11.0 / 255.0,
        17.0 / 255.0,
        1.0,
    )));

    let layer: Retained<VideoLayer> = unsafe { msg_send![VideoLayer::class(), layer] };
    layer.setAsynchronous(false);
    layer.setNeedsDisplayOnBoundsChange(true);
    layer.setContentsScale(ns_window.backingScaleFactor());

    let view = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, webview.as_deref());

    STATE.lock().unwrap().mpv = mpv;
    layer.setNeedsDisplay();
    let _ = LAYER.set(MainThreadBound::new(layer, mtm));
    Ok(())
}

pub fn set_has_video(has_video: bool) {
    STATE.lock().unwrap().has_video = has_video;
    request_redraw();
}

pub fn refresh<R: Runtime>(window: &WebviewWindow<R>) {
    let (Some(mtm), Some(layer)) = (MainThreadMarker::new(), LAYER.get()) else {
        return;
    };
    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window: &NSWindow = unsafe { &*ns_window.cast::<NSWindow>() };
    let layer = layer.get(mtm);
    layer.setContentsScale(ns_window.backingScaleFactor());
    layer.setNeedsDisplay();
}

fn request_redraw() {
    DispatchQueue::main().exec_async(|| {
        if let (Some(mtm), Some(layer)) = (MainThreadMarker::new(), LAYER.get()) {
            layer.get(mtm).setNeedsDisplay();
        }
    });
}

extern "C" fn on_render_update(_ctx: *mut c_void) {
    request_redraw();
}

unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    libc::dlsym(libc::RTLD_DEFAULT, name)
}

fn render_frame() {
    let mut state = STATE.lock().unwrap();
    if state.context.is_null() && !state.mpv.is_null() {
        state.context = create_render_context(state.mpv);
    }

    let (mut fbo, mut viewport) = (0 as GLint, [0 as GLint; 4]);
    unsafe {
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo);
        glGetIntegerv(GL_VIEWPORT, viewport.as_mut_ptr());
    }

    if state.context.is_null() || !state.has_video {
        unsafe {
            glClearColor(0.0, 0.0, 0.0, 0.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        return;
    }

    unsafe { sys::mpv_render_context_update(state.context) };
    let mut target = sys::mpv_opengl_fbo {
        fbo,
        w: viewport[2],
        h: viewport[3],
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

fn choose_pixel_format(display_mask: u32) -> Option<CGLPixelFormatObj> {
    let attributes = [
        CGLPixelFormatAttribute::CGLPFAOpenGLProfile,
        CGLPixelFormatAttribute(CGLOpenGLProfile::CGLOGLPVersion_3_2_Core.0),
        CGLPixelFormatAttribute::CGLPFAAccelerated,
        CGLPixelFormatAttribute::CGLPFADoubleBuffer,
        CGLPixelFormatAttribute::CGLPFAColorSize,
        CGLPixelFormatAttribute(24),
        CGLPixelFormatAttribute::CGLPFAAlphaSize,
        CGLPixelFormatAttribute(8),
        CGLPixelFormatAttribute::CGLPFAAllowOfflineRenderers,
        CGLPixelFormatAttribute::CGLPFADisplayMask,
        CGLPixelFormatAttribute(display_mask),
        CGLPixelFormatAttribute(0),
    ];
    let mut format: CGLPixelFormatObj = ptr::null_mut();
    let mut count: GLint = 0;
    let error = unsafe {
        CGLChoosePixelFormat(
            NonNull::from(&attributes[0]),
            NonNull::from(&mut format),
            NonNull::from(&mut count),
        )
    };
    (error.0 == 0 && !format.is_null()).then_some(format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_open_gl::{CGLCreateContext, CGLSetCurrentContext};
    use serde_json::json;

    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_RGBA8: i32 = 0x8058;
    const GL_RGBA: u32 = 0x1908;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    const SIZE: (i32, i32) = (320, 240);

    #[link(name = "OpenGL", kind = "framework")]
    extern "C" {
        fn glGenFramebuffers(n: i32, ids: *mut u32);
        fn glBindFramebuffer(target: u32, id: u32);
        fn glGenTextures(n: i32, ids: *mut u32);
        fn glBindTexture(target: u32, id: u32);
        #[allow(clippy::too_many_arguments)]
        fn glTexImage2D(target: u32, level: i32, internal: i32, w: i32, h: i32, border: i32, format: u32, ty: u32, data: *const c_void);
        fn glFramebufferTexture2D(target: u32, attachment: u32, textarget: u32, texture: u32, level: i32);
        fn glCheckFramebufferStatus(target: u32) -> u32;
        fn glViewport(x: i32, y: i32, w: i32, h: i32);
        fn glReadPixels(x: i32, y: i32, w: i32, h: i32, format: u32, ty: u32, data: *mut c_void);
        fn glFinish();
    }

    fn center_pixel(fbo: u32) -> [u8; 4] {
        let mut pixel = [0u8; 4];
        unsafe {
            glFinish();
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glReadPixels(SIZE.0 / 2, SIZE.1 / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, pixel.as_mut_ptr().cast());
        }
        pixel
    }

    #[test]
    fn video_layer_renders_mpv_frames() {
        let layer: Retained<VideoLayer> = unsafe { msg_send![VideoLayer::class(), layer] };
        let format = layer.copyCGLPixelFormatForDisplayMask(u32::MAX);
        assert!(!format.is_null(), "no OpenGL pixel format");

        let mut context: CGLContextObj = ptr::null_mut();
        let (mut fbo, mut texture) = (0u32, 0u32);
        unsafe {
            CGLCreateContext(format, ptr::null_mut(), NonNull::from(&mut context));
            assert!(!context.is_null(), "no OpenGL context");
            CGLSetCurrentContext(context);

            glGenTextures(1, &mut texture);
            glBindTexture(GL_TEXTURE_2D, texture);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, SIZE.0, SIZE.1, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, SIZE.0, SIZE.1);
        }

        let mpv = super::super::mpv::Mpv::new(&[("vo", "libmpv"), ("ao", "null")]).expect("mpv starts");
        STATE.lock().unwrap().mpv = mpv.raw();
        render_frame();
        assert!(!STATE.lock().unwrap().context.is_null(), "render context was not created");
        assert_eq!(center_pixel(fbo), [0, 0, 0, 0], "idle layer should be transparent");

        mpv.command(&json!(["loadfile", "av://lavfi:testsrc=duration=3:size=320x240:rate=25"]))
            .expect("loadfile accepted");
        STATE.lock().unwrap().has_video = true;

        let mut pixel = [0u8; 4];
        for _ in 0..250 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            render_frame();
            pixel = center_pixel(fbo);
            if pixel[..3].iter().any(|&c| c > 0) {
                break;
            }
        }
        assert!(pixel[..3].iter().any(|&c| c > 0), "no video frame was drawn: {pixel:?}");
        assert_eq!(pixel[3], 255, "video frame should be opaque: {pixel:?}");

        let mut state = STATE.lock().unwrap();
        unsafe { sys::mpv_render_context_free(state.context) };
        state.context = ptr::null_mut();
        state.mpv = ptr::null_mut();
        state.has_video = false;
    }
}
