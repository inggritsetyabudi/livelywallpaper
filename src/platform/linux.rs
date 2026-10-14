//! Linux backend: X11 desktop windows and Wayland layer-shell surfaces.
//!
//! # The compositor problem
//!
//! macOS makes this easy — an `NSWindow` placed below other windows. Linux has
//! no equivalent in the base protocols, so there are two paths:
//!
//! * **X11 / XWayland** — a window with `_NET_WM_WINDOW_TYPE_DESKTOP`, plus
//!   `_NET_WM_STATE_BELOW` and a raise/lower cycle so it sits at the bottom of
//!   the stack. Works on any X11 session, including XWayland under Wayland,
//!   but is window-manager dependent: a compositor that manages its own
//!   wallpaper may paint over it.
//!
//! * **Wayland** — `zwlr_layer_shell_v1` with `layer = "bottom"`. This is the
//!   only supported way to put a surface beneath everything else, and it is
//!   implemented by Sway, Hyprland, river, niri, wayfire, KDE Plasma 6 and the
//!   other wlroots/KMS compositors.
//!
//! **GNOME/Mutter implements neither.** It has no layer-shell and no public
//! wallpaper API, so a background surface there is not achievable without
//! private hacks that break between updates. GNOME users should expect the
//! desktop to keep its own wallpaper; see `docs/linux.md`.
//!
//! # Renderers
//!
//! Implemented here: images (via Cairo), and a GL shader path.
//!
//! Video and web need a media stack (GStreamer, WebKitGTK) that is not vendored
//! into this repository. [`VideoSupport`] reports honestly which of those are
//! compiled in rather than pretending the daemon has full coverage.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::{Error, Result};
use crate::platform::{Platform, Playback, PlaybackState, SurfaceKind, ViewHandle};

/// Which window system this process is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
    /// No display reachable — headless, or a Wayland session without XWayland.
    None,
}

pub(crate) fn detect_session() -> Session {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Session::Wayland;
    }
    if std::env::var_os("DISPLAY").is_some() {
        return Session::X11;
    }
    Session::None
}

/// What this build can actually render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSupport {
    pub video: bool,
    pub web: bool,
}

impl VideoSupport {
    const fn detect() -> Self {
        // Gated by cargo features in `build.rs`; off by default so a plain
        // `cargo build` on a bare machine still succeeds.
        Self {
            video: cfg!(feature = "linux-video"),
            web: cfg!(feature = "linux-web"),
        }
    }
}

/// Monotonic id source for layer-shell surfaces.
static NEXT_SURFACE: AtomicU32 = AtomicU32::new(1);

/// A desktop-level window on X11 or Wayland.
///
/// The pointer is an owned allocation holding a stable id; the real surface
/// lives inside the platform backend, which owns its own bookkeeping.
#[repr(C)]
pub struct SurfaceRecord {
    pub screen_index: u32,
    pub id: u32,
    /// X11 window id, or 0 on Wayland where the handle is not an XID.
    pub x11_window: u64,
}

pub struct LinuxPlatform;

impl LinuxPlatform {
    pub(crate) fn session() -> Session {
        detect_session()
    }
}

impl Platform for LinuxPlatform {
    const NAME: &'static str = "Linux";

    fn init() -> Result<()> {
        match Self::session() {
            Session::None => {
                // Not fatal: the CLI, library and screensaver path still work,
                // and wallpaper commands will report a clear error.
                log::warn!(
                    "neither WAYLAND_DISPLAY nor DISPLAY is set: running without a \
                     display server, so wallpapers cannot be shown"
                );
                Ok(())
            }
            session => {
                log::info!("Linux session detected: {session:?}");
                Ok(())
            }
        }
    }

    fn pump_events(timeout_secs: f64) {
        // No blocking run loop on Linux yet; sleep for the requested slice so
        // callers keep their pacing and the process stays responsive to signals.
        let capped = timeout_secs.clamp(0.0, 0.25);
        std::thread::sleep(std::time::Duration::from_secs_f64(capped));
    }

    fn wake() {
        // The main loop is time-sliced rather than blocked, so there is nothing
        // to interrupt.
    }

    fn screen_count() -> u32 {
        crate::platform::linux_sys::screen_count(Self::session())
    }

    fn screen_info(index: u32) -> (String, u32, u32) {
        crate::platform::linux_sys::screen_info(Self::session(), index)
    }

    fn create_surface(screen_index: u32) -> Result<*mut c_void> {
        let session = Self::session();
        let record =
            crate::platform::linux_sys::create_surface(session, screen_index).ok_or_else(|| {
                Error::Invalid(match session {
                    Session::Wayland => {
                        "could not create a layer-shell surface. This compositor does \
                         not implement wlr-layer-shell (GNOME/Mutter does not)."
                            .into()
                    }
                    Session::X11 => "could not open the X display. Is DISPLAY set, and is the X \
                         server reachable?"
                        .into(),
                    Session::None => "no display server is available, so no wallpaper surface can \
                         be created."
                        .into(),
                })
            })?;

        let boxed = Box::into_raw(Box::new(record));
        Ok(boxed.cast())
    }

    fn destroy_surface(surface: *mut c_void) {
        if surface.is_null() {
            return;
        }
        // SAFETY: every pointer handed out by `create_surface` came from
        // `Box::into_raw` and is returned exactly once.
        let record = unsafe { Box::from_raw(surface.cast::<SurfaceRecord>()) };
        crate::platform::linux_sys::destroy_surface(Self::session(), record);
    }

    fn is_main_thread() -> bool {
        // The Linux backend does not run a GUI loop yet, so the process's main
        // thread is identified the same way Rust does elsewhere.
        true
    }

    fn create_view(surface: *mut c_void, target: &Path, kind: SurfaceKind) -> Result<ViewHandle> {
        if surface.is_null() {
            return Err(Error::Invalid("surface has already been destroyed".into()));
        }
        if !target.exists() {
            return Err(Error::PathNotFound(target.to_path_buf()));
        }

        let support = VideoSupport::detect();
        match kind {
            SurfaceKind::Image => {
                crate::platform::linux_sys::create_image_view(surface, target)?;
                Ok(ViewHandle { ptr: surface, kind })
            }
            SurfaceKind::Video if support.video => {
                crate::platform::linux_sys::create_video_view(surface, target)?;
                Ok(ViewHandle { ptr: surface, kind })
            }
            SurfaceKind::Web if support.web => {
                crate::platform::linux_sys::create_web_view(surface, target)?;
                Ok(ViewHandle { ptr: surface, kind })
            }
            SurfaceKind::Shader => {
                crate::platform::linux_sys::create_shader_view(surface, target)?;
                Ok(ViewHandle { ptr: surface, kind })
            }
            SurfaceKind::Video => Err(Error::Invalid(
                "video wallpapers need the GStreamer stack, which this build does \
                 not include. Rebuild with `--features linux-video`."
                    .into(),
            )),
            SurfaceKind::Web => Err(Error::Invalid(
                "web wallpapers need WebKitGTK, which this build does not include. \
                 Rebuild with `--features linux-web`."
                    .into(),
            )),
        }
    }

    fn swap_view_source(view: &ViewHandle, target: &Path) -> bool {
        match view.kind {
            SurfaceKind::Image => crate::platform::linux_sys::swap_image_source(view.ptr, target),
            _ => false,
        }
    }

    fn destroy_view(_view: ViewHandle) {
        // Views are torn down with their surface on Linux: the Cairo surface is
        // owned by the window record, so dropping the surface drops them too.
    }

    fn set_playing(view: &ViewHandle, playing: bool) {
        match view.kind {
            SurfaceKind::Video => crate::platform::linux_sys::set_video_playing(view.ptr, playing),
            SurfaceKind::Shader => {
                crate::platform::linux_sys::set_shader_paused(view.ptr, !playing)
            }
            _ => {}
        }
    }

    fn set_volume(view: &ViewHandle, volume: f32) {
        if view.kind == SurfaceKind::Video {
            crate::platform::linux_sys::set_video_volume(view.ptr, volume);
        }
    }

    fn seek(view: &ViewHandle, percent: f64) {
        if view.kind == SurfaceKind::Video {
            crate::platform::linux_sys::seek_video(view.ptr, percent);
        }
    }

    fn playback(view: &ViewHandle) -> Playback {
        if view.kind != SurfaceKind::Video {
            return Playback {
                state: PlaybackState::Unknown,
                position: -1.0,
                duration: -1.0,
            };
        }
        crate::platform::linux_sys::video_playback(view.ptr)
    }

    fn send_property(_view: &ViewHandle, _name: &str, _value: &str) -> Result<()> {
        Err(Error::Invalid(
            "wallpaper properties need the WebKitGTK bridge, which this build does \
             not include."
                .into(),
        ))
    }

    fn reload_web(_view: &ViewHandle) {}

    fn capture(surface: *mut c_void, out_path: &Path) -> Result<()> {
        crate::platform::linux_sys::capture(surface, out_path)
    }

    fn present_ui() -> Result<()> {
        Err(Error::Invalid(
            "the graphical settings window is macOS-only in this build. Use the CLI: \
             `lively set <path>`, `lively list-library`, `lively screenshot`."
                .into(),
        ))
    }
}

/// Library path helper shared by the CLI, used when reporting errors.
pub fn library_root() -> PathBuf {
    crate::config::LivelyConfig::support_dir().join("Library")
}
