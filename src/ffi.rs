//! Raw bindings to the Objective-C layer in `src/native/lively_native.m`.
//!
//! # Thread safety
//!
//! These declarations carry no safety contract of their own — that lives at each
//! call site. The broad rule for this crate:
//!
//! * **Main thread only** (AppKit: `NSWindow`, `NSStatusItem`, `NSEvent`):
//!   everything not called out below.
//! * **Any thread**: `lively_is_any_app_fullscreen`, `lively_is_running_on_battery`.
//!
//! Guard the main-thread-only calls with [`crate::main::ensure_main_thread`]
//! when the caller might be a worker.
//!
//! # Unused bindings
//!
//! This module mirrors the C header one-to-one. Several functions have no Rust
//! caller yet (shaders, audio spectrum, layout modes); they are part of the
//! native surface and will be used as those features land, so they stay
//! declared rather than being deleted and re-added.

#![allow(dead_code)]

use std::ffi::{c_char, c_void};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivelyWallpaperType {
    Video = 0,
    Web = 1,
    WebGL = 2,
    App = 3,
    Gif = 4,
    Picture = 5,
    Shader = 6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivelyScaleMode {
    AspectFill = 0,
    AspectFit = 1,
    Stretch = 2,
    Center = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivelyLayoutMode {
    PerScreen = 0,
    Span = 1,
    Duplicate = 2,
}

pub type TrayCallback = extern "C" fn(command_id: i32);

extern "C" {
    pub fn lively_init_app() -> bool;
    pub fn lively_run_loop_run();
    pub fn lively_run_loop_stop();
    pub fn lively_pump_events(timeout_secs: f64);
    pub fn lively_is_main_thread() -> bool;

    pub fn lively_get_screen_count() -> u32;
    pub fn lively_get_screen_bounds(
        index: u32,
        x: *mut f64,
        y: *mut f64,
        w: *mut f64,
        h: *mut f64,
    ) -> bool;
    pub fn lively_get_screen_name(index: u32) -> *mut c_char;
    pub fn lively_free_string(s: *mut c_char);

    pub fn lively_create_desktop_window(screen_index: u32) -> *mut c_void;
    pub fn lively_destroy_desktop_window(win_ptr: *mut c_void);
    pub fn lively_set_window_level(win_ptr: *mut c_void, level_offset: i32);
    pub fn lively_set_window_interactive(win_ptr: *mut c_void, interactive: bool);

    pub fn lively_video_set_source(player_ptr: *mut c_void, file_path: *const c_char) -> bool;
    pub fn lively_create_video_view(
        win_ptr: *mut c_void,
        file_path: *const c_char,
        scale_mode: LivelyScaleMode,
    ) -> *mut c_void;
    pub fn lively_video_set_volume(player_ptr: *mut c_void, volume: f32);
    pub fn lively_video_set_playback_rate(player_ptr: *mut c_void, rate: f32);
    pub fn lively_video_seek(player_ptr: *mut c_void, percent: f64);
    pub fn lively_video_play(player_ptr: *mut c_void);
    pub fn lively_video_get_state(player_ptr: *mut c_void) -> i32;
    pub fn lively_video_get_position(player_ptr: *mut c_void) -> f64;
    pub fn lively_video_get_duration(player_ptr: *mut c_void) -> f64;
    pub fn lively_video_pause(player_ptr: *mut c_void);
    pub fn lively_destroy_video_view(player_ptr: *mut c_void);

    pub fn lively_create_web_view(
        win_ptr: *mut c_void,
        url_or_path: *const c_char,
        is_local_file: bool,
    ) -> *mut c_void;
    pub fn lively_web_eval_js(web_ptr: *mut c_void, js_code: *const c_char);
    pub fn lively_web_send_audio_spectrum(web_ptr: *mut c_void, bands: *const f32, count: u32);
    pub fn lively_web_send_system_data(web_ptr: *mut c_void, json_data: *const c_char);
    pub fn lively_web_send_property(
        web_ptr: *mut c_void,
        prop_name: *const c_char,
        prop_value: *const c_char,
    );
    pub fn lively_web_reload(web_ptr: *mut c_void);
    pub fn lively_web_pause(web_ptr: *mut c_void);
    pub fn lively_web_resume(web_ptr: *mut c_void);
    pub fn lively_destroy_web_view(web_ptr: *mut c_void);

    pub fn lively_create_shader_view(
        win_ptr: *mut c_void,
        shader_source: *const c_char,
    ) -> *mut c_void;
    pub fn lively_shader_set_pause(shader_ptr: *mut c_void, paused: bool);
    pub fn lively_shader_update_audio(shader_ptr: *mut c_void, bands: *const f32, count: u32);
    pub fn lively_destroy_shader_view(shader_ptr: *mut c_void);

    pub fn lively_image_set_source(img_ptr: *mut c_void, file_path: *const c_char) -> bool;
    pub fn lively_create_image_view(
        win_ptr: *mut c_void,
        file_path: *const c_char,
        scale_mode: LivelyScaleMode,
    ) -> *mut c_void;
    pub fn lively_destroy_image_view(img_ptr: *mut c_void);

    pub fn lively_is_any_app_fullscreen() -> bool;
    pub fn lively_is_running_on_battery() -> bool;

    pub fn lively_setup_tray_menu(callback: TrayCallback);
    pub fn lively_update_tray_title(title: *const c_char);
    pub fn lively_open_file_picker(out_path: *mut c_char, max_len: u32);

    pub fn lively_create_screensaver_window(screen_index: u32) -> *mut c_void;
    pub fn lively_close_screensaver_window(win_ptr: *mut c_void);
    pub fn lively_start_screensaver_monitor(dismiss_cb: extern "C" fn());
    pub fn lively_stop_screensaver_monitor();

    pub fn lively_take_screenshot(win_ptr: *mut c_void, out_path: *const c_char) -> bool;

    pub fn lively_dispatch_on_main(func: extern "C" fn(*mut c_void), ctx: *mut c_void);
}

// ---------------------------------------------------------------------------
// Settings UI (implemented in src/ui/*.swift)
// ---------------------------------------------------------------------------

extern "C" {
    /// Show the settings window and activate the app.
    pub fn lively_ui_present();

    /// Ask the UI to re-fetch state and redraw. Thread-safe.
    pub fn lively_ui_refresh();

    /// True when the Swift half is linked in.
    pub fn lively_ui_available() -> bool;
}

/// Function pointer Rust registers so Swift can send user intents back.
pub type LivelyUiCommandHandler = unsafe extern "C" fn(*const c_char) -> *mut c_char;

/// Screen name and pixel dimensions, copied out of the native layer.
///
/// `lively_get_screen_name` returns a pointer into an autoreleased buffer that
/// the next native call could invalidate, so the string is copied immediately
/// rather than handed to the caller as a raw pointer.
pub fn screen_info(index: u32) -> (String, u32, u32) {
    // The native side hands over a malloc'd copy. Copy it out and release it
    // immediately: leaking here would cost a few bytes on *every* UI refresh.
    let name = unsafe {
        let ptr = lively_get_screen_name(index);
        if ptr.is_null() {
            String::from("unknown")
        } else {
            let copied = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            lively_free_string(ptr);
            copied
        }
    };

    let (mut w, mut h) = (0.0f64, 0.0f64);
    unsafe {
        lively_get_screen_bounds(
            index,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut w,
            &mut h,
        );
    }

    (name, w.max(0.0) as u32, h.max(0.0) as u32)
}

/// Assert that the caller is on the main thread.
///
/// Calling AppKit off-thread is undefined behaviour rather than a clean failure,
/// so this is checked at every boundary where Rust code could have been handed a
/// worker thread. It is cheap (`pthread_main_np`), and it converts a silent
/// corruption into a diagnosable error.
pub fn ensure_main_thread(what: &'static str) -> crate::error::Result<()> {
    // SAFETY: `isMainThread` reads thread-local state and has no preconditions.
    if unsafe { lively_is_main_thread() } {
        Ok(())
    } else {
        Err(crate::error::Error::NotOnMainThread(what))
    }
}
