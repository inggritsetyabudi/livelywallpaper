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

/// Narrows a view's surface pointer to the shim's opaque handle type.
///
/// A `ViewHandle` is a raw pointer that may have been zeroed by the caller;
/// the shim takes a `NonNull`, so the conversion happens once here rather than
/// at every call site.
fn view_handle(view: &ViewHandle) -> Option<crate::platform::linux_sys::SurfaceHandle> {
    std::ptr::NonNull::new(view.ptr)
}

// ---------------------------------------------------------------------------
// poll(2)
//
// Declared here rather than pulling in the `libc` crate: this is one struct and
// one function, and the project has committed to keeping its dependency list
// short enough to audit by eye.
// ---------------------------------------------------------------------------

type libc_c_int = i32;
type libc_nfds_t = u64;

const POLLIN: i16 = 0x001;

#[repr(C)]
struct PollFd {
    fd: libc_c_int,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: libc_nfds_t, timeout: libc_c_int) -> libc_c_int;
}

/// Whether the machine is running on battery.
///
/// Reads sysfs rather than shelling out to `upower` or parsing `acpi`: it is
/// the kernel's own interface, it is present on every Linux system with a
/// battery, and it costs one small file read.
///
/// A desktop with no battery is on AC power by definition, so an absent
/// `/sys/class/power_supply` BAT* entry answers false.
fn battery_is_discharging() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("BAT") {
            continue;
        }
        let status = entry.path().join("status");
        if let Ok(text) = std::fs::read_to_string(&status) {
            if text.trim() == "Discharging" {
                return true;
            }
        }
    }
    false
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
        // Block on the display server's fd rather than spinning.
        //
        // The previous implementation slept for a fixed slice on every pass,
        // which woke the daemon tens of times a second to do nothing. On a
        // laptop that is measurable battery drain for a process whose only
        // job is to sit behind the desktop. poll(2) sleeps in the kernel until
        // the server actually has something to say, so an idle wallpaper
        // costs no CPU at all.
        let timeout_ms = (timeout_secs.clamp(0.0, 1.0) * 1000.0) as libc_c_int;

        let Some(fd) = crate::platform::linux_sys::event_fd() else {
            // Headless: nothing can ever arrive, so sleep the whole slice
            // rather than calling poll on an invalid fd.
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms as u64));
            return;
        };

        let mut fds = [PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        }];
        // SAFETY: `fds` is a live, correctly sized array and `poll` only writes
        // into its `revents` field. A negative return is EFAULT/EINTR and is
        // deliberately ignored: the loop's timeout still bounds the wait, and
        // a signal should simply let the next iteration run.
        unsafe {
            poll(fds.as_mut_ptr(), 1, timeout_ms);
        }
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
        let handle =
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
        Ok(handle.as_ptr())
    }

    fn destroy_surface(surface: *mut c_void) {
        // SAFETY: every pointer handed out by `create_surface` came from the C
        // shim and is returned exactly once. It is freed by C, which is also
        // where it was allocated -- see the ownership note in `linux_sys`.
        let Some(handle) = std::ptr::NonNull::new(surface) else {
            return;
        };
        crate::platform::linux_sys::destroy_surface(handle);
    }

    fn is_main_thread() -> bool {
        // The Linux backend does not run a GUI loop yet, so the process's main
        // thread is identified the same way Rust does elsewhere.
        true
    }

    fn create_view(surface: *mut c_void, target: &Path, kind: SurfaceKind) -> Result<ViewHandle> {
        let Some(handle) = std::ptr::NonNull::new(surface) else {
            return Err(Error::Invalid("surface has already been destroyed".into()));
        };
        if !target.exists() {
            return Err(Error::PathNotFound(target.to_path_buf()));
        }

        let support = VideoSupport::detect();
        let created = match kind {
            SurfaceKind::Image => crate::platform::linux_sys::create_image_view(handle, target),
            SurfaceKind::Video if support.video => {
                crate::platform::linux_sys::create_video_view(handle, target)
            }
            SurfaceKind::WebFile | SurfaceKind::WebUrl if support.web => {
                crate::platform::linux_sys::create_web_view(handle, target)
            }
            SurfaceKind::Shader => crate::platform::linux_sys::create_shader_view(handle, target),
            SurfaceKind::Video => {
                return Err(Error::Invalid(
                    "video wallpapers need the GStreamer stack, which this build does \
                     not include. Rebuild with `--features linux-video`."
                        .into(),
                ))
            }
            SurfaceKind::WebFile | SurfaceKind::WebUrl => {
                return Err(Error::Invalid(
                    "web wallpapers need WebKitGTK, which this build does not include. \
                     Rebuild with `--features linux-web`."
                        .into(),
                ))
            }
        };

        if !created {
            // The shim reports failure for a missing or undecodable file, an
            // unsupported format, and -- for video -- a pipeline that never
            // reached PLAYING. A generic message here is deliberately vague,
            // because the shim does not yet carry the reason across.
            return Err(Error::Invalid(format!(
                "could not render {} -- the file may be missing, in an unsupported \
                 format, or refused by the decoder",
                target.display()
            )));
        }

        // On Linux the wallpaper is drawn straight into the surface's own
        // window, so there is no separate view object: the surface *is* the
        // view. `ptr` therefore aliases the surface rather than holding a
        // distinct handle, and `destroy_view` is a no-op for exactly that
        // reason. The macOS backend does hold a separate view here.
        //
        // Consequence to preserve: `destroy_surface` is the only thing that
        // may free this pointer. If a separate view object is ever added on
        // Linux, this aliasing has to be revisited in the same change.
        Ok(ViewHandle { ptr: surface, kind })
    }

    fn swap_view_source(view: &ViewHandle, target: &Path) -> bool {
        match view.kind {
            SurfaceKind::Image => view_handle(view)
                .is_some_and(|h| crate::platform::linux_sys::swap_image_source(h, target)),
            _ => false,
        }
    }

    fn destroy_view(_view: ViewHandle) {
        // Deliberately empty: the view is the surface. See `create_view` for
        // why tearing it down here would free a window the engine still uses.
    }

    fn set_playing(view: &ViewHandle, playing: bool) {
        let Some(h) = view_handle(view) else { return };
        match view.kind {
            SurfaceKind::Video => crate::platform::linux_sys::set_video_playing(h, playing),
            SurfaceKind::Shader => crate::platform::linux_sys::set_shader_paused(h, !playing),
            _ => {}
        }
    }

    fn set_volume(view: &ViewHandle, volume: f32) {
        if view.kind == SurfaceKind::Video {
            if let Some(h) = view_handle(view) {
                crate::platform::linux_sys::set_video_volume(h, volume);
            }
        }
    }

    fn seek(view: &ViewHandle, percent: f64) {
        if view.kind == SurfaceKind::Video {
            if let Some(h) = view_handle(view) {
                crate::platform::linux_sys::seek_video(h, percent);
            }
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
        match view_handle(view) {
            Some(h) => crate::platform::linux_sys::video_playback(h),
            None => Playback {
                state: PlaybackState::Unknown,
                position: -1.0,
                duration: -1.0,
            },
        }
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
        let Some(handle) = std::ptr::NonNull::new(surface) else {
            return Err(Error::Invalid("surface has already been destroyed".into()));
        };
        crate::platform::linux_sys::capture(handle, out_path)
    }

    fn any_app_fullscreen() -> bool {
        match Self::session() {
            Session::X11 => crate::platform::linux_sys::x11_any_fullscreen(),
            // Wayland deliberately keeps _NET_ACTIVE_WINDOW out of the
            // protocol -- the whole point of Wayland is that clients cannot
            // observe other windows. Reporting false is the only honest
            // answer; guessing true would pause wallpapers at random.
            Session::Wayland | Session::None => false,
        }
    }

    fn on_battery_power() -> bool {
        battery_is_discharging()
    }

    fn diagnostics() -> String {
        crate::platform::linux_sys::diagnostics()
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
