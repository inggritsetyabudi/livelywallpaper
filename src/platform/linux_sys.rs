//! Rust side of the Linux C shim (`src/platform/native/lively_linux.c`).
//!
//! Every function here is a thin translation; the logic lives in C where the
//! XCB and Wayland headers are natural to use.

#![cfg(target_os = "linux")]

use std::ffi::{c_char, c_int, c_void};

extern "C" {
    fn lively_screen_count(session: c_int) -> u32;
    fn lively_screen_width(session: c_int, index: u32) -> u32;
    fn lively_screen_height(session: c_int, index: u32) -> u32;
    fn lively_create_surface(session: c_int, index: u32) -> *mut c_void;
    fn lively_destroy_surface(handle: *mut c_void);
    fn lively_create_view(handle: *mut c_void, path: *const c_char, kind: c_int) -> c_int;
    fn lively_swap_view_source(handle: *mut c_void, path: *const c_char) -> c_int;
    fn lively_destroy_view(handle: *mut c_void);
    fn lively_web_set_paused(handle: *mut c_void, paused: c_int) -> c_int;
    fn lively_web_send_property(
        handle: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    fn lively_set_playing(handle: *mut c_void, playing: c_int);
    fn lively_refresh_video(handle: *mut c_void);
    fn lively_set_volume(handle: *mut c_void, volume: f64);
    fn lively_seek(handle: *mut c_void, percent: f64);
    fn lively_playback_state(handle: *mut c_void) -> c_int;
    fn lively_playback_position(handle: *mut c_void) -> f64;
    fn lively_playback_duration(handle: *mut c_void) -> f64;
    fn lively_capture(handle: *mut c_void, out_path: *const c_char) -> c_int;
}

use crate::platform::{Playback, PlaybackState, SurfaceKind};

pub fn session_code(session: super::Session) -> c_int {
    match session {
        super::Session::Wayland => 1,
        super::Session::X11 => 2,
        super::Session::None => 0,
    }
}

pub fn kind_code(kind: SurfaceKind) -> c_int {
    match kind {
        SurfaceKind::Video => 0,
        SurfaceKind::Web => 1,
        SurfaceKind::Shader => 2,
        SurfaceKind::Image => 3,
    }
}

fn cstring(value: &std::path::Path) -> Option<std::ffi::CString> {
    std::ffi::CString::new(value.to_string_lossy().as_bytes()).ok()
}

pub fn screen_count(session: super::Session) -> u32 {
    unsafe { lively_screen_count(session_code(session)) }
}

pub fn screen_info(session: super::Session, index: u32) -> (String, u32, u32) {
    unsafe {
        (
            format!("Display {}", index + 1),
            lively_screen_width(session_code(session), index),
            lively_screen_height(session_code(session), index),
        )
    }
}

pub fn create_surface(session: super::Session, index: u32) -> Option<super::SurfaceRecord> {
    let handle = unsafe { lively_create_surface(session_code(session), index) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: the shim returns a malloc'd record; ownership transfers here and
    // is released exactly once by destroy_surface.
    let record = unsafe { Box::from_raw(handle.cast::<super::SurfaceRecord>()) };
    Some(*record)
}

pub fn destroy_surface(session: super::Session, record: super::SurfaceRecord) {
    // Put the record back so the shim can free it, keeping a single owner.
    let boxed = Box::new(record);
    unsafe { lively_destroy_surface(Box::into_raw(boxed).cast()) };
    let _ = session;
}

pub fn create_image_view(handle: *mut c_void, path: &std::path::Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe { lively_create_view(handle, path.as_ptr(), kind_code(SurfaceKind::Image)) == 1 }
}

pub fn swap_image_source(handle: *mut c_void, path: &std::path::Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe { lively_swap_view_source(handle, path.as_ptr()) == 1 }
}

pub fn create_video_view(handle: *mut c_void, path: &std::path::Path) -> bool {
    // The C side only reports success when GStreamer is actually compiled in
    // and the pipeline reached PLAYING.
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe { lively_create_view(handle, path.as_ptr(), kind_code(SurfaceKind::Video)) == 1 }
}

pub fn create_web_view(handle: *mut c_void, path: &std::path::Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe { lively_create_view(handle, path.as_ptr(), kind_code(SurfaceKind::Web)) == 1 }
}

pub fn web_set_paused(handle: *mut c_void, paused: bool) -> bool {
    unsafe { lively_web_set_paused(handle, c_int::from(paused)) == 1 }
}

pub fn web_send_property(handle: *mut c_void, name: &str, value: &str) -> bool {
    let (Ok(n), Ok(v)) = (std::ffi::CString::new(name), std::ffi::CString::new(value)) else {
        return false;
    };
    unsafe { lively_web_send_property(handle, n.as_ptr(), v.as_ptr()) == 1 }
}

pub fn create_shader_view(_handle: *mut c_void, _path: &std::path::Path) -> bool {
    false
}

pub fn set_video_playing(handle: *mut c_void, playing: bool) {
    unsafe { lively_set_playing(handle, c_int::from(playing)) }
}

pub fn set_shader_paused(handle: *mut c_void, paused: bool) {
    unsafe { lively_set_playing(handle, c_int::from(!paused)) }
}

pub fn set_video_volume(handle: *mut c_void, volume: f32) {
    unsafe { lively_set_volume(handle, f64::from(volume)) }
}

pub fn seek_video(handle: *mut c_void, percent: f64) {
    unsafe { lively_seek(handle, percent) }
}

pub fn video_playback(handle: *mut c_void) -> Playback {
    // Position and duration are queried from the live pipeline rather than
    // cached, so the UI cannot report a stale value.
    unsafe {
        lively_refresh_video(handle);
        Playback {
            state: PlaybackState::from_code(lively_playback_state(handle)),
            position: lively_playback_position(handle),
            duration: lively_playback_duration(handle),
        }
    }
}

pub fn capture(_handle: *mut c_void, _out_path: &std::path::Path) -> crate::error::Result<()> {
    Err(crate::error::Error::Invalid(
        "screenshots are not implemented on Linux yet".into(),
    ))
}
