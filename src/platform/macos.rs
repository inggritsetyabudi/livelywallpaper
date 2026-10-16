//! macOS backend: wraps the Objective-C layer in `src/native`.
//!
//! No behaviour is reimplemented here — this only translates between the
//! portable [`Platform`] vocabulary and the existing FFI, so the macOS path
//! keeps behaving exactly as it did before the abstraction existed.

use std::ffi::c_void;
use std::path::Path;

use crate::error::{Error, Result};
use crate::ffi;
use crate::platform::{Platform, Playback, PlaybackState, SurfaceKind, TrayCallback, ViewHandle};

pub struct MacPlatform;

impl Platform for MacPlatform {
    const NAME: &'static str = "macOS";

    fn init() -> Result<()> {
        // SAFETY: startup, single-threaded, before any callback is installed.
        if unsafe { ffi::lively_init_app() } {
            Ok(())
        } else {
            Err(Error::Invalid("failed to initialise NSApplication".into()))
        }
    }

    fn pump_events(timeout_secs: f64) {
        // SAFETY: only ever called on the main loop thread.
        unsafe { ffi::lively_pump_events(timeout_secs) }
    }

    fn wake() {
        // SAFETY: dispatching to the main queue is thread-safe by contract.
        unsafe {
            ffi::lively_dispatch_on_main(noop_on_main, std::ptr::null_mut());
        }
    }

    fn screen_count() -> u32 {
        // SAFETY: main thread only; call sites are on the run loop.
        unsafe { ffi::lively_get_screen_count() }
    }

    fn screen_info(index: u32) -> (String, u32, u32) {
        ffi::screen_info(index)
    }

    fn create_surface(screen_index: u32) -> Result<*mut c_void> {
        // SAFETY: main thread, live NSWindow comes back owned.
        let win = unsafe { ffi::lively_create_desktop_window(screen_index) };
        if win.is_null() {
            Err(Error::NativeAllocationFailed("desktop window"))
        } else {
            Ok(win)
        }
    }

    fn destroy_surface(surface: *mut c_void) {
        // SAFETY: called once per surface created above, on the main thread.
        unsafe { ffi::lively_destroy_desktop_window(surface) }
    }

    fn is_main_thread() -> bool {
        // SAFETY: reads thread-local state.
        unsafe { ffi::lively_is_main_thread() }
    }

    fn create_view(surface: *mut c_void, target: &Path, kind: SurfaceKind) -> Result<ViewHandle> {
        use crate::engine::cstring;
        let location = cstring(target)?;

        fn check(ptr: *mut c_void, what: &'static str) -> Result<*mut c_void> {
            if ptr.is_null() {
                Err(Error::NativeAllocationFailed(what))
            } else {
                Ok(ptr)
            }
        }

        let ptr = unsafe {
            match kind {
                SurfaceKind::Video => check(
                    ffi::lively_create_video_view(
                        surface,
                        location.as_ptr(),
                        ffi::LivelyScaleMode::AspectFill,
                    ),
                    "video view",
                )?,
                // `is_local_file` selects the WebKit loader. Passing `true`
                // for a remote URL makes WebKit raise "is not a file URL".
                SurfaceKind::WebFile => check(
                    ffi::lively_create_web_view(surface, location.as_ptr(), true),
                    "web view",
                )?,
                SurfaceKind::WebUrl => check(
                    ffi::lively_create_web_view(surface, location.as_ptr(), false),
                    "web view",
                )?,
                SurfaceKind::Shader => {
                    let source =
                        std::fs::read_to_string(target).map_err(|e| Error::io(target, e))?;
                    let src = std::ffi::CString::new(source).map_err(|_| Error::InteriorNul)?;
                    check(
                        ffi::lively_create_shader_view(surface, src.as_ptr()),
                        "shader view",
                    )?
                }
                SurfaceKind::Image => check(
                    ffi::lively_create_image_view(
                        surface,
                        location.as_ptr(),
                        ffi::LivelyScaleMode::AspectFill,
                    ),
                    "image view",
                )?,
            }
        };

        Ok(ViewHandle { ptr, kind })
    }

    fn swap_view_source(view: &ViewHandle, target: &Path) -> bool {
        use crate::engine::cstring;
        let Ok(path) = cstring(target) else {
            return false;
        };

        // SAFETY: `view.kind` was recorded when the handle was created, so this
        // dispatches to exactly the destructor its creator implies.
        unsafe {
            match view.kind {
                SurfaceKind::Video => ffi::lively_video_set_source(view.ptr, path.as_ptr()),
                SurfaceKind::Image => ffi::lively_image_set_source(view.ptr, path.as_ptr()),
                _ => false,
            }
        }
    }

    fn destroy_view(view: ViewHandle) {
        // SAFETY: one call per handle created by `create_view`, dispatched by
        // the kind recorded at creation.
        unsafe {
            match view.kind {
                SurfaceKind::Video => ffi::lively_destroy_video_view(view.ptr),
                SurfaceKind::WebFile | SurfaceKind::WebUrl => {
                    ffi::lively_destroy_web_view(view.ptr)
                }
                SurfaceKind::Shader => ffi::lively_destroy_shader_view(view.ptr),
                SurfaceKind::Image => ffi::lively_destroy_image_view(view.ptr),
            }
        }
    }

    fn set_playing(view: &ViewHandle, playing: bool) {
        // SAFETY: `view.kind` decides which transport call applies.
        unsafe {
            match view.kind {
                SurfaceKind::Video => {
                    if playing {
                        ffi::lively_video_play(view.ptr)
                    } else {
                        ffi::lively_video_pause(view.ptr)
                    }
                }
                SurfaceKind::WebFile | SurfaceKind::WebUrl => {
                    if playing {
                        ffi::lively_web_resume(view.ptr)
                    } else {
                        ffi::lively_web_pause(view.ptr)
                    }
                }
                SurfaceKind::Shader => ffi::lively_shader_set_pause(view.ptr, !playing),
                SurfaceKind::Image => {}
            }
        }
    }

    fn set_volume(view: &ViewHandle, volume: f32) {
        // SAFETY: as above.
        unsafe { ffi::lively_video_set_volume(view.ptr, volume) }
    }

    fn seek(view: &ViewHandle, percent: f64) {
        // SAFETY: as above.
        unsafe { ffi::lively_video_seek(view.ptr, percent) }
    }

    fn playback(view: &ViewHandle) -> Playback {
        // SAFETY: as above.
        unsafe {
            Playback {
                state: PlaybackState::from_code(ffi::lively_video_get_state(view.ptr)),
                position: ffi::lively_video_get_position(view.ptr),
                duration: ffi::lively_video_get_duration(view.ptr),
            }
        }
    }

    fn send_property(view: &ViewHandle, name: &str, value: &str) -> Result<()> {
        let (n, v) = (
            std::ffi::CString::new(name).map_err(|_| Error::InteriorNul)?,
            std::ffi::CString::new(value).map_err(|_| Error::InteriorNul)?,
        );
        // SAFETY: web views are the only target; the engine checks first.
        unsafe { ffi::lively_web_send_property(view.ptr, n.as_ptr(), v.as_ptr()) };
        Ok(())
    }

    fn reload_web(view: &ViewHandle) {
        // SAFETY: as above.
        unsafe { ffi::lively_web_reload(view.ptr) }
    }

    fn capture(surface: *mut c_void, out_path: &Path) -> Result<()> {
        let path = std::ffi::CString::new(out_path.to_string_lossy().as_bytes())
            .map_err(|_| Error::InteriorNul)?;
        // SAFETY: `path` outlives the call; native only reads it.
        if unsafe { ffi::lively_take_screenshot(surface, path.as_ptr()) } {
            Ok(())
        } else {
            Err(Error::Ipc("screenshot capture failed".into()))
        }
    }

    fn present_ui() -> Result<()> {
        // SAFETY: dispatches to the main queue internally.
        unsafe { ffi::lively_ui_present() };
        Ok(())
    }

    fn register_ui_handler() {
        // SAFETY: `ui_command_entry` is a plain `extern "C"` fn with no captures.
        unsafe { crate::ui::lively_ui_set_handler(crate::ui::ui_command_entry) };
    }

    fn install_tray_menu(callback: TrayCallback) -> bool {
        // SAFETY: `callback` is a plain `extern "C"` fn with no captures;
        // AppKit stores it and calls it on the main thread.
        unsafe { ffi::lively_setup_tray_menu(callback) };
        true
    }

    fn set_tray_title(title: &str) {
        if let Ok(c_title) = std::ffi::CString::new(title) {
            // SAFETY: main thread; AppKit status item.
            unsafe { ffi::lively_update_tray_title(c_title.as_ptr()) };
        }
    }

    fn any_app_fullscreen() -> bool {
        // SAFETY: CoreGraphics window list read; no AppKit, safe off-main.
        unsafe { ffi::lively_is_any_app_fullscreen() }
    }

    fn on_battery_power() -> bool {
        // SAFETY: IOKit power source read; safe off-main.
        unsafe { ffi::lively_is_running_on_battery() }
    }

    fn dispatch_on_main(f: extern "C" fn(*mut c_void), ctx: *mut c_void) {
        // SAFETY: `f` is a plain `extern "C"` fn with no captures, and
        // `lively_dispatch_on_main` takes ownership of neither.
        unsafe { ffi::lively_dispatch_on_main(f, ctx) };
    }
}

#[allow(dead_code)]
extern "C" fn noop_on_main(_ctx: *mut c_void) {}
