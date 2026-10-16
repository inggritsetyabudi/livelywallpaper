//! Rust side of the Linux C shim (`src/platform/native/lively_linux.c`).
//!
//! Every function here is a thin translation; the logic lives in C where the
//! XCB and Wayland headers are natural to use.
//!
//! # Ownership
//!
//! A surface handle is allocated with `calloc` in C and freed with `free` in C.
//! Rust holds it as an opaque [`NonNull<c_void>`] and must never turn it into
//! a `Box`.
//!
//! This is not a stylistic preference. An earlier revision defined a
//! `#[repr(C)] SurfaceRecord` mirror of the C struct and did
//! `Box::from_raw(handle)` followed by `Some(*record)`. That copied the
//! record out, dropped the `Box` — so Rust's allocator freed memory C still
//! owned — and left the caller holding a stale copy of a struct whose window
//! id and buffers were already gone. Destruction then wrapped the copy in a
//! *new* `Box` and asked C to `free` it, which is the mirror-image mistake:
//! C's `free` on Rust's allocation. Both directions were wrong, and neither
//! can be caught by the compiler.
//!
//! The rule this module exists to enforce: **C allocates, C frees, Rust only
//! passes the pointer back.**

#![cfg(target_os = "linux")]

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr::NonNull;

use crate::platform::{Playback, PlaybackState, SurfaceKind};

use super::linux::Session;

/// An owned surface handle. Opaque by construction: the layout lives in C and
/// may change without touching this file.
pub type SurfaceHandle = NonNull<c_void>;

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

    /// Fd to block on, or a negative value when there is no display. The main
    /// loop polls this instead of sleeping, so an idle daemon costs no CPU
    /// and still wakes the instant the compositor or X server has something
    /// to say.
    fn lively_event_fd() -> c_int;
    fn lively_x11_any_fullscreen() -> bool;
    fn lively_diagnostics() -> *mut std::ffi::c_char;
    fn lively_diagnostics_free(text: *mut std::ffi::c_char);
}

pub fn session_code(session: Session) -> c_int {
    match session {
        Session::Wayland => 1,
        Session::X11 => 2,
        Session::None => 0,
    }
}

pub fn kind_code(kind: SurfaceKind) -> c_int {
    match kind {
        SurfaceKind::Video => 0,
        SurfaceKind::WebFile | SurfaceKind::WebUrl => 1,
        SurfaceKind::Shader => 2,
        SurfaceKind::Image => 3,
    }
}

fn cstring(value: &Path) -> Option<std::ffi::CString> {
    std::ffi::CString::new(value.to_string_lossy().as_bytes()).ok()
}

pub fn screen_count(session: Session) -> u32 {
    unsafe { lively_screen_count(session_code(session)) }
}

pub fn screen_info(session: Session, index: u32) -> (String, u32, u32) {
    unsafe {
        (
            format!("Display {}", index + 1),
            lively_screen_width(session_code(session), index),
            lively_screen_height(session_code(session), index),
        )
    }
}

/// Creates a surface, taking ownership of the handle C returns.
pub fn create_surface(session: Session, index: u32) -> Option<SurfaceHandle> {
    // SAFETY: `lively_create_surface` either returns NULL or a pointer to a
    // `calloc`'d record. The record is ours until `destroy_surface`.
    NonNull::new(unsafe { lively_create_surface(session_code(session), index) })
}

/// Releases a surface. The handle is freed in C, matching how it was made.
pub fn destroy_surface(handle: SurfaceHandle) {
    // SAFETY: called exactly once per handle handed out by `create_surface`.
    unsafe { lively_destroy_surface(handle.as_ptr()) }
}

pub fn create_image_view(handle: SurfaceHandle, path: &Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    // SAFETY: `handle` is a live surface; `path` outlives the call because
    // `CString` is dropped only after it.
    unsafe {
        lively_create_view(
            handle.as_ptr(),
            path.as_ptr(),
            kind_code(SurfaceKind::Image),
        ) == 1
    }
}

pub fn swap_image_source(handle: SurfaceHandle, path: &Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe { lively_swap_view_source(handle.as_ptr(), path.as_ptr()) == 1 }
}

pub fn create_video_view(handle: SurfaceHandle, path: &Path) -> bool {
    // The C side only reports success when GStreamer is actually compiled in
    // and the pipeline reached PLAYING.
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe {
        lively_create_view(
            handle.as_ptr(),
            path.as_ptr(),
            kind_code(SurfaceKind::Video),
        ) == 1
    }
}

pub fn create_web_view(handle: SurfaceHandle, path: &Path) -> bool {
    let Some(path) = cstring(path) else {
        return false;
    };
    unsafe {
        lively_create_view(
            handle.as_ptr(),
            path.as_ptr(),
            kind_code(SurfaceKind::WebUrl),
        ) == 1
    }
}

pub fn web_set_paused(handle: SurfaceHandle, paused: bool) -> bool {
    unsafe { lively_web_set_paused(handle.as_ptr(), c_int::from(paused)) == 1 }
}

pub fn web_send_property(handle: SurfaceHandle, name: &str, value: &str) -> bool {
    let (Ok(n), Ok(v)) = (std::ffi::CString::new(name), std::ffi::CString::new(value)) else {
        return false;
    };
    unsafe { lively_web_send_property(handle.as_ptr(), n.as_ptr(), v.as_ptr()) == 1 }
}

pub fn create_shader_view(_handle: SurfaceHandle, _path: &Path) -> bool {
    false
}

pub fn set_video_playing(handle: SurfaceHandle, playing: bool) {
    unsafe { lively_set_playing(handle.as_ptr(), c_int::from(playing)) }
}

pub fn set_shader_paused(handle: SurfaceHandle, paused: bool) {
    unsafe { lively_set_playing(handle.as_ptr(), c_int::from(!paused)) }
}

pub fn set_video_volume(handle: SurfaceHandle, volume: f32) {
    unsafe { lively_set_volume(handle.as_ptr(), f64::from(volume)) }
}

pub fn seek_video(handle: SurfaceHandle, percent: f64) {
    unsafe { lively_seek(handle.as_ptr(), percent) }
}

pub fn video_playback(handle: SurfaceHandle) -> Playback {
    // Position and duration are queried from the live pipeline rather than
    // cached, so the UI cannot report a stale value.
    unsafe {
        lively_refresh_video(handle.as_ptr());
        Playback {
            state: PlaybackState::from_code(lively_playback_state(handle.as_ptr())),
            position: lively_playback_position(handle.as_ptr()),
            duration: lively_playback_duration(handle.as_ptr()),
        }
    }
}

pub fn capture(_handle: SurfaceHandle, _out_path: &Path) -> crate::error::Result<()> {
    Err(crate::error::Error::Invalid(
        "screenshots are not implemented on Linux yet".into(),
    ))
}

/// Whether any window is currently fullscreen under X11.
pub fn x11_any_fullscreen() -> bool {
    // SAFETY: no arguments, no ownership transfer; the shim only reads EWMH
    // properties off the cached connection.
    unsafe { lively_x11_any_fullscreen() }
}

/// A capability report, one fact per line.
///
/// Exists so someone testing on real hardware can hand over something useful
/// instead of "it does not work" -- the session type, which globals the
/// compositor offered, the detected displays, and which renderers are compiled
/// in each have different fixes.
pub fn diagnostics() -> String {
    // SAFETY: the shim returns a malloc'd NUL-terminated string, or NULL on
    // allocation failure. Copied out and released immediately.
    unsafe {
        let ptr = lively_diagnostics();
        if ptr.is_null() {
            return "could not allocate a diagnostics report".into();
        }
        let text = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
        lively_diagnostics_free(ptr);
        text
    }
}

/// The display fd to block on, or `None` when there is no display server.
pub fn event_fd() -> Option<i32> {
    let fd = unsafe { lively_event_fd() };
    (fd >= 0).then_some(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    /// The whole point of making the handle opaque is that the C struct layout
    /// is Rust's business only if it says so. If someone reintroduces a
    /// `#[repr(C)]` mirror, this documents why the size assumption must stay
    /// out of the Rust side.
    #[test]
    fn handle_is_a_bare_pointer() {
        assert_eq!(
            mem::size_of::<SurfaceHandle>(),
            mem::size_of::<*mut c_void>()
        );
    }

    #[test]
    fn session_codes_match_the_c_header() {
        assert_eq!(session_code(Session::None), 0);
        assert_eq!(session_code(Session::Wayland), 1);
        assert_eq!(session_code(Session::X11), 2);
    }

    #[test]
    fn kind_codes_match_the_c_header() {
        assert_eq!(kind_code(SurfaceKind::Video), 0);
        assert_eq!(kind_code(SurfaceKind::WebFile), 1);
        assert_eq!(kind_code(SurfaceKind::WebUrl), 1);
        assert_eq!(kind_code(SurfaceKind::Shader), 2);
        assert_eq!(kind_code(SurfaceKind::Image), 3);
    }

    #[test]
    fn headless_session_reports_no_screens() {
        // No DISPLAY or WAYLAND_DISPLAY in the test environment, so the C
        // side must answer zero rather than crash on a null connection.
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            assert_eq!(screen_count(Session::None), 0);
            assert!(event_fd().is_none());
        }
    }

    #[test]
    fn paths_containing_nul_are_rejected_rather_than_truncated() {
        // A path with an interior NUL must not silently become a shorter,
        // still-valid path pointing somewhere else.
        let bad = Path::new("bad\0path.png");
        assert!(cstring(bad).is_none());
        assert!(!create_image_view(NonNull::dangling(), bad));
    }
}
