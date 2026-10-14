//! Platform abstraction.
//!
//! Everything OS-specific lives behind [`Platform`]. The rest of the daemon —
//! CLI, IPC, library, config, wallpaper routing — is plain Rust and compiles
//! everywhere.
//!
//! # Why this shape
//!
//! macOS and Linux are not variants of the same thing, they are different
//! mechanisms:
//!
//! | | macOS | X11 | Wayland |
//! |---|---|---|---|
//! | Surface | `NSWindow` below other windows | `_NET_WM_WINDOW_TYPE_DESKTOP` | `zwlr_layer_shell_v1` |
//! | Needs | AppKit | Xlib/XCB | `wlr-layer-shell-unstable-v1` |
//!
//! Wayland has no notion of "a window beneath other windows" in the base
//! protocol. The layer-shell extension is the closest thing, and it covers
//! Sway, Hyprland, river, niri, wayfire and other wlroots/KMS compositors.
//! GNOME/Mutter implements no equivalent, so wallpaper support there is not
//! possible without private, unstable hacks — see `docs/linux.md`.

use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacPlatform as ActivePlatform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) mod linux_sys;
#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as ActivePlatform;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!(
    "Lively supports macOS and Linux. Other targets have no wallpaper surface \
     implementation; see src/platform/mod.rs."
);

/// What a window surface can draw, chosen per wallpaper rather than per platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Video,
    Web,
    Shader,
    Image,
}

/// Playback state of a video, mirroring the macOS `AVPlayer` states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Unknown,
    Loading,
    Ready,
    Playing,
    Paused,
    Failed,
    Ended,
}

impl PlaybackState {
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => PlaybackState::Loading,
            2 => PlaybackState::Ready,
            3 => PlaybackState::Playing,
            4 => PlaybackState::Paused,
            5 => PlaybackState::Failed,
            6 => PlaybackState::Ended,
            _ => PlaybackState::Unknown,
        }
    }

    pub fn label(self) -> Option<&'static str> {
        match self {
            PlaybackState::Playing => None,
            PlaybackState::Unknown => Some("Idle"),
            PlaybackState::Loading => Some("Loading…"),
            PlaybackState::Ready => Some("Ready"),
            PlaybackState::Paused => Some("Paused"),
            PlaybackState::Failed => Some("Failed to load"),
            PlaybackState::Ended => Some("Ended"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Playback {
    pub state: PlaybackState,
    pub position: f64,
    pub duration: f64,
}

/// An opaque handle to a mounted view inside a surface window.
///
/// The `kind` travels with the pointer so teardown and transport calls know
/// which backend function to dispatch to, without the caller having to keep
/// re-checking the type it inferred earlier.
///
/// Deliberately `Send`: it is only ever touched under the engine lock, and on
/// the main thread, which is where the compositor APIs require.
pub struct ViewHandle {
    pub ptr: *mut std::ffi::c_void,
    pub kind: SurfaceKind,
}

// SAFETY: a handle is moved between threads only while the engine lock is held,
// and is only dereferenced on the thread that owns the surface. The pointer
// never leaves the `EngineManager` that created it.
unsafe impl Send for ViewHandle {}

/// Where the platform draws wallpaper surfaces, and how it draws into them.
///
/// Implementations are per-OS and unsafe-free from the caller's perspective:
/// every method either succeeds or returns a typed error.
///
/// Not every method is called on every platform path — a few exist for a single
/// backend's needs — so unused-trait-item warnings are suppressed here rather
/// than sprinkled through each implementation.
#[allow(dead_code)]
pub trait Platform: Sized {
    /// Human-readable name, used in logs and `--version` output.
    const NAME: &'static str;

    /// Bring up the platform's main loop and window server connection.
    fn init() -> Result<()>;

    /// Run one turn of the platform's event loop, blocking up to `timeout_secs`.
    fn pump_events(timeout_secs: f64);

    /// Ask the event loop to wake and return, used to break out of `pump_events`.
    fn wake();

    /// Number of attached screens.
    fn screen_count() -> u32;

    /// Screen name and pixel dimensions.
    fn screen_info(index: u32) -> (String, u32, u32);

    /// Create the desktop-level surface that wallpapers are drawn into.
    fn create_surface(screen_index: u32) -> Result<*mut std::ffi::c_void>;

    /// Destroy a surface created by [`Platform::create_surface`].
    fn destroy_surface(surface: *mut std::ffi::c_void);

    /// True when the caller is on the thread that owns the UI/event loop.
    fn is_main_thread() -> bool;

    /// Mount a wallpaper into a surface.
    fn create_view(
        surface: *mut std::ffi::c_void,
        target: &Path,
        kind: SurfaceKind,
    ) -> Result<ViewHandle>;

    /// Point an existing view at a different file, reusing the expensive
    /// underlying objects. Returns false when the platform cannot reuse and the
    /// caller should rebuild instead.
    fn swap_view_source(view: &ViewHandle, target: &Path) -> bool;

    fn destroy_view(view: ViewHandle);

    /// Play/pause every mounted video.
    fn set_playing(view: &ViewHandle, playing: bool);

    /// Set volume on a video, 0..1.
    fn set_volume(view: &ViewHandle, volume: f32);

    /// Seek a video to a percentage, 0..100.
    fn seek(view: &ViewHandle, percent: f64);

    /// Current playback position.
    fn playback(view: &ViewHandle) -> Playback;

    /// Deliver a user property to a web wallpaper.
    fn send_property(view: &ViewHandle, name: &str, value: &str) -> Result<()>;

    /// Reload a web wallpaper.
    fn reload_web(view: &ViewHandle);

    /// Capture a surface to an image file.
    fn capture(surface: *mut std::ffi::c_void, out_path: &Path) -> Result<()>;

    /// Open the settings window. A no-op on platforms with no bundled UI.
    fn present_ui() -> Result<()> {
        Ok(())
    }

    /// Register the handler the settings UI calls back with.
    fn register_ui_handler() {}
}
