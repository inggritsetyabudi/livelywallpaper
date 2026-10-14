//! Wallpaper engine: owns one desktop window per screen and the view currently
//! mounted inside it.
//!
//! # Design
//!
//! All *decision* logic (which engine handles which file, what type code maps to
//! what) lives in pure functions over `WallpaperTarget` and is unit tested
//! without touching AppKit. Only [`EngineManager::mount`] crosses into native
//! code, so the parts most likely to regress can be tested on any machine.

use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::package::LivelyInfo;
use crate::platform::{ActivePlatform, Platform, Playback, SurfaceKind, ViewHandle};

/// Which renderer is currently mounted in a screen's window.
pub type ActiveEngineView = ViewHandle;

/// The renderer chosen for a wallpaper, decided without touching the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Video,
    /// Local HTML file loaded from disk.
    WebFile,
    /// Remote URL.
    WebUrl,
    Shader,
    Image,
}

/// A wallpaper that has been resolved to "renderer + concrete file".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub renderer: Renderer,
    pub location: PathBuf,
}

pub struct ScreenWallpaper {
    pub screen_index: u32,
    pub window: *mut c_void,
    pub active_view: Option<ActiveEngineView>,
    pub current_target: Option<PathBuf>,
}

/// SAFETY: `EngineManager` owns raw native handles (`*mut c_void`) that are not
/// `Send` by default. Every access is serialised through the `Mutex` in
/// `state::AppState`, and every one of those accesses happens on the main thread,
/// because AppKit objects (NSWindow, AVPlayer, WKWebView) may not be touched
/// from a background thread. Moving the handle between threads would be unsound;
/// moving the *guard* to the mutex that serialises them is not.
unsafe impl Send for EngineManager {}

pub struct EngineManager {
    pub screens: Vec<ScreenWallpaper>,
    pub volume: f32,
    pub is_paused: bool,
    pub is_muted: bool,
    screensaver_windows: Vec<*mut c_void>,
}

impl EngineManager {
    /// Build an engine with a real window per attached screen.
    pub fn new() -> Result<Self> {
        ActivePlatform::init()?;

        let count = ActivePlatform::screen_count() as usize;
        let mut screens = Vec::with_capacity(count);

        for i in 0..count {
            let win = ActivePlatform::create_surface(i as u32).ok();
            let Some(win) = win else {
                log::error!("could not create wallpaper window for screen {i}; skipping");
                continue;
            };
            screens.push(ScreenWallpaper {
                screen_index: i as u32,
                window: win,
                active_view: None,
                current_target: None,
            });
        }

        Ok(Self {
            screens,
            volume: 0.8,
            is_paused: false,
            is_muted: false,
            screensaver_windows: Vec::new(),
        })
    }

    /// An engine with no native windows. Used by tests that exercise routing
    /// and state logic without touching AppKit.
    #[cfg(test)]
    pub fn headless_for_tests() -> Self {
        Self {
            screens: Vec::new(),
            volume: 0.8,
            is_paused: false,
            is_muted: false,
            screensaver_windows: Vec::new(),
        }
    }

    pub fn screen_count(&self) -> usize {
        self.screens.len()
    }

    /// Resolve a user-supplied string into a concrete wallpaper target.
    ///
    /// Pure: no FFI, no mutation. Everything that can go wrong with user input
    /// (missing path, unknown extension, malformed package) is caught here so
    /// callers get a precise error before any window is touched.
    pub fn resolve_target(input: &str) -> Result<ResolvedTarget> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(Error::Invalid("wallpaper path is empty".into()));
        }

        if is_url(trimmed) {
            return Ok(ResolvedTarget {
                renderer: Renderer::WebUrl,
                location: PathBuf::from(trimmed),
            });
        }

        let path = Path::new(trimmed);

        // A Lively project directory: metadata decides the renderer.
        if path.is_dir() {
            let info_path = path.join("LivelyInfo.json");
            if !info_path.exists() {
                return Err(Error::PathNotFound(info_path));
            }
            let info =
                LivelyInfo::load_from_dir(path).map_err(|e| Error::Invalid(e.to_string()))?;
            let renderer = renderer_for_type(info.wallpaper_type)?;
            let media = info.target_path();

            if !media.exists() {
                return Err(Error::PathNotFound(media));
            }
            return Ok(ResolvedTarget {
                renderer,
                location: media,
            });
        }

        if !path.exists() {
            return Err(Error::PathNotFound(path.to_path_buf()));
        }

        let ext = extension_of(path)
            .ok_or_else(|| Error::Invalid(format!("{} has no file extension", path.display())))?;

        Ok(ResolvedTarget {
            renderer: renderer_for_extension(&ext)?,
            location: path.to_path_buf(),
        })
    }

    /// Position of `screen_index` within `screens`, or a descriptive error.
    ///
    /// Split out from the callers so bounds handling is testable without the
    /// main-thread requirement the mutating operations carry.
    fn find_screen_index(&self, screen_index: u32) -> Result<usize> {
        self.screens
            .iter()
            .position(|s| s.screen_index == screen_index)
            .ok_or_else(|| Error::screen(screen_index, self.screens.len()))
    }

    /// Mount a wallpaper on `screen_index`, replacing whatever was there.
    pub fn set_wallpaper(&mut self, screen_index: u32, input: &str) -> Result<()> {
        // Mounting creates native views inside an NSWindow, which is main-thread-only.
        if !ActivePlatform::is_main_thread() {
            return Err(Error::NotOnMainThread("set_wallpaper"));
        }
        let target = Self::resolve_target(input)?;

        // Transport state is read before the screen is borrowed, and the window
        // handle is copied out, so the engine is never immutably borrowed while
        // a screen holds a mutable one.
        let should_play = !self.is_paused;
        let volume = if self.is_muted { 0.0 } else { self.volume };

        let index = self.find_screen_index(screen_index)?;

        // Reuse an existing video view rather than rebuilding it. Constructing a
        // fresh AVPlayer / AVPlayerLooper / AVPlayerLayer on every wallpaper
        // change leaked about 1.3 MB each time and never plateaued; swapping the
        // item keeps those objects alive for the life of the screen, and makes
        // switching feel instant.
        // Same reuse argument as video: a decoded full-resolution bitmap is
        // around 15 MB, so rebuilding the view per change is expensive in both
        // time and memory.
        if target.renderer == Renderer::Image {
            let existing = self.screens[index]
                .active_view
                .as_ref()
                .filter(|v| v.kind == SurfaceKind::Image);
            if let Some(existing) = existing {
                if ActivePlatform::swap_view_source(existing, &target.location) {
                    let screen = &mut self.screens[index];
                    screen.current_target = Some(target.location.clone());
                    log::info!(
                        "screen {screen_index}: swapped to {}",
                        target.location.display()
                    );
                    return Ok(());
                }
                log::warn!("image swap failed; rebuilding the view");
            }
        }

        if target.renderer == Renderer::Video {
            let existing = self.screens[index]
                .active_view
                .as_ref()
                .filter(|v| v.kind == SurfaceKind::Video);
            if let Some(existing) = existing {
                if ActivePlatform::swap_view_source(existing, &target.location) {
                    let screen = &mut self.screens[index];
                    screen.current_target = Some(target.location.clone());
                    apply_transport(screen.active_view.as_ref().unwrap(), should_play, volume);
                    log::info!(
                        "screen {screen_index}: swapped to {}",
                        target.location.display()
                    );
                    return Ok(());
                }
                log::warn!("video swap rejected; rebuilding the view");
            }
        }

        let window = self.screens[index].window;
        let view = Self::create_view(window, &target)?;

        let screen = &mut self.screens[index];
        if let Some(old) = screen.active_view.take() {
            ActivePlatform::destroy_view(old);
        }
        screen.active_view = Some(view);
        screen.current_target = Some(target.location.clone());

        if let Some(view) = &screen.active_view {
            apply_transport(view, should_play, volume);
        }

        log::info!(
            "screen {screen_index}: mounted {:?} from {}",
            target.renderer,
            target.location.display()
        );
        Ok(())
    }

    /// Native allocation for the resolved target. The only platform-touching
    /// step in the whole engine.
    fn create_view(window: *mut c_void, target: &ResolvedTarget) -> Result<ActiveEngineView> {
        let kind = surface_kind(target.renderer)?;
        ActivePlatform::create_view(window, &target.location, kind)
    }

    /// Tear down the wallpaper on one screen, or all of them when `monitor < 0`.
    ///
    /// The surface itself survives; only what is drawn into it is removed.
    pub fn close_wallpaper(&mut self, monitor: i32) -> Result<()> {
        if monitor < 0 {
            for screen in &mut self.screens {
                unmount(screen);
            }
            return Ok(());
        }

        let index = self.find_screen_index(monitor as u32)?;
        unmount(&mut self.screens[index]);
        Ok(())
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        self.broadcast_volume();
    }

    /// Push the current volume to every video, honouring mute.
    fn broadcast_volume(&self) {
        let effective = if self.is_muted { 0.0 } else { self.volume };
        for screen in &self.screens {
            if let Some(view) = screen
                .active_view
                .as_ref()
                .filter(|v| v.kind == SurfaceKind::Video)
            {
                ActivePlatform::set_volume(view, effective);
            }
        }
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.is_muted = !self.is_muted;
        self.broadcast_volume();
        self.is_muted
    }

    pub fn set_playback_state(&mut self, play: bool) {
        self.is_paused = !play;
        let volume = if self.is_muted { 0.0 } else { self.volume };
        for screen in &self.screens {
            if let Some(view) = &screen.active_view {
                apply_transport(view, play, volume);
            }
        }
    }

    /// Returns the new transport state (`true` = playing).
    pub fn toggle_play_pause(&mut self) -> bool {
        let now_playing = self.is_paused;
        self.set_playback_state(now_playing);
        now_playing
    }

    /// Seek a video to `percent` (0-100).
    pub fn seek(&self, screen_index: u32, percent: f64) -> Result<()> {
        let index = self.find_screen_index(screen_index)?;
        let Some(view) = self.screens[index].active_view.as_ref() else {
            return Err(Error::Invalid(
                "no seekable wallpaper on this screen".into(),
            ));
        };

        match view.kind {
            SurfaceKind::Video => {
                ActivePlatform::seek(view, percent.clamp(0.0, 100.0));
                Ok(())
            }
            SurfaceKind::Web => {
                // For web wallpapers `0` doubles as "start over".
                if percent <= 0.0 {
                    ActivePlatform::reload_web(view);
                    Ok(())
                } else {
                    Err(Error::Invalid(
                        "seek is only supported for video wallpapers".into(),
                    ))
                }
            }
            _ => Err(Error::Invalid(
                "no seekable wallpaper on this screen".into(),
            )),
        }
    }

    /// Push normalised (0..1) frequency bands to renderers that consume audio.
    ///
    /// No-op on platforms without an audio bridge; the caller cannot tell, and
    /// does not need to.
    #[allow(dead_code)]
    pub fn send_audio_data(&self, bands: &[f32]) {
        let _ = bands;
    }

    /// Deliver a user-defined property to a web wallpaper.
    pub fn set_property(&self, screen_index: u32, name: &str, value: &str) -> Result<()> {
        let index = self.find_screen_index(screen_index)?;
        match self.screens[index].active_view.as_ref() {
            Some(view) if view.kind == SurfaceKind::Web => {
                ActivePlatform::send_property(view, name, value)
            }
            _ => Err(Error::Invalid(
                "properties are only supported for web wallpapers".into(),
            )),
        }
    }

    /// Live playback state of the video on a screen.
    ///
    /// Distinguishes "still loading" and "failed" from "paused": without this a
    /// broken video looks exactly like a working one showing black.
    pub fn video_playback(&self, screen_index: u32) -> Result<Playback> {
        let index = self.find_screen_index(screen_index)?;
        let view = self.screens[index]
            .active_view
            .as_ref()
            .filter(|v| v.kind == SurfaceKind::Video)
            .ok_or_else(|| Error::Invalid("no video wallpaper on this screen".into()))?;

        Ok(ActivePlatform::playback(view))
    }

    /// Capture a screen's surface to `out_path`.
    pub fn take_screenshot(&self, monitor: u32, out_path: &Path) -> Result<()> {
        let index = self.find_screen_index(monitor)?;
        let screen = &self.screens[index];

        if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        ActivePlatform::capture(screen.window, out_path)
    }

    /// Open a fullscreen screensaver surface on `monitor`.
    ///
    /// `dismiss_cb` fires on the first key press or mouse movement. It must be a
    /// plain `extern "C"` function: the native side stores it and calls it from
    /// an event handler, so it cannot capture anything.
    pub fn start_screensaver(&mut self, monitor: u32, dismiss_cb: extern "C" fn()) -> Result<()> {
        self.find_screen_index(monitor)?;

        if !ActivePlatform::is_main_thread() {
            return Err(Error::NotOnMainThread("start_screensaver"));
        }

        // SAFETY: main thread, inside the run loop we own.
        let win = unsafe { crate::ffi::lively_create_screensaver_window(monitor) };
        if win.is_null() {
            return Err(Error::NativeAllocationFailed("screensaver window"));
        }
        self.screensaver_windows.push(win);

        // Only one monitor may be active at a time: the native monitor is a
        // single global, so starting twice would silently drop the first.
        if self.screensaver_windows.len() == 1 {
            // SAFETY: a plain function pointer, called from the main thread.
            unsafe { crate::ffi::lively_start_screensaver_monitor(dismiss_cb) };
        }

        let target = self.screens[monitor as usize].current_target.clone();
        match target {
            Some(path) => log::info!("screensaver showing {path:?} on monitor {monitor}"),
            None => log::info!("screensaver started on monitor {monitor} (no wallpaper set)"),
        }
        Ok(())
    }

    pub fn stop_screensaver(&mut self) {
        // SAFETY: windows were produced by `lively_create_screensaver_window`
        // and are consumed exactly once here.
        unsafe {
            crate::ffi::lively_stop_screensaver_monitor();
            for win in self.screensaver_windows.drain(..) {
                crate::ffi::lively_close_screensaver_window(win);
            }
        }
    }

    /// Whether a screensaver surface is currently open.
    #[allow(dead_code)]
    pub fn screensaver_active(&self) -> bool {
        !self.screensaver_windows.is_empty()
    }
}

/// Map a renderer onto the surface the platform should create.
pub fn surface_kind(renderer: Renderer) -> Result<SurfaceKind> {
    Ok(match renderer {
        Renderer::Video => SurfaceKind::Video,
        Renderer::WebFile | Renderer::WebUrl => SurfaceKind::Web,
        Renderer::Shader => SurfaceKind::Shader,
        Renderer::Image => SurfaceKind::Image,
    })
}

impl Drop for EngineManager {
    fn drop(&mut self) {
        self.stop_screensaver();
        for screen in &mut self.screens {
            if let Some(view) = screen.active_view.take() {
                ActivePlatform::destroy_view(view);
            }
            ActivePlatform::destroy_surface(screen.window);
        }
    }
}

/// Release the wallpaper currently mounted in `screen`, if any.
fn unmount(screen: &mut ScreenWallpaper) {
    if let Some(view) = screen.active_view.take() {
        ActivePlatform::destroy_view(view);
    }
    screen.current_target = None;
}

/// Push play/pause + volume state onto a mounted view.
fn apply_transport(view: &ActiveEngineView, play: bool, volume: f32) {
    if view.kind == SurfaceKind::Video {
        ActivePlatform::set_volume(view, volume);
    }
    ActivePlatform::set_playing(view, play);
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

pub fn renderer_for_extension(ext: &str) -> Result<Renderer> {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" | "mpg" | "mpeg" => Ok(Renderer::Video),
        "html" | "htm" => Ok(Renderer::WebFile),
        "gif" | "jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "bmp" | "tiff" => {
            Ok(Renderer::Image)
        }
        "metal" | "shader" | "fragment" => Ok(Renderer::Shader),
        other => Err(Error::UnsupportedFormat(other.to_string())),
    }
}

pub fn renderer_for_type(wallpaper_type: i32) -> Result<Renderer> {
    match wallpaper_type {
        0 => Ok(Renderer::Video),
        1 | 2 => Ok(Renderer::WebFile),
        3 => Err(Error::UnknownWallpaperType(3)), // App wallpapers need an executable host
        4 | 5 => Ok(Renderer::Image),
        6 => Ok(Renderer::Shader),
        other => Err(Error::UnknownWallpaperType(other)),
    }
}

/// Build a NUL-terminated C string, rejecting embedded NULs explicitly.
pub fn cstring(value: &Path) -> Result<CString> {
    let s = value.to_string_lossy();
    CString::new(s.as_bytes()).map_err(|_| Error::InteriorNul)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lively-engine-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn urls_map_to_web_url_renderer() {
        let t = EngineManager::resolve_target("https://example.com/a.html").unwrap();
        assert_eq!(t.renderer, Renderer::WebUrl);
        assert_eq!(t.location, PathBuf::from("https://example.com/a.html"));
    }

    #[test]
    fn extension_mapping_is_case_insensitive() {
        assert_eq!(renderer_for_extension("MP4").unwrap(), Renderer::Video);
        assert_eq!(renderer_for_extension("WebM").unwrap(), Renderer::Video);
        assert_eq!(renderer_for_extension("HTML").unwrap(), Renderer::WebFile);
        assert_eq!(renderer_for_extension("GIF").unwrap(), Renderer::Image);
        assert_eq!(renderer_for_extension("METAL").unwrap(), Renderer::Shader);
    }

    #[test]
    fn unsupported_extension_is_rejected_by_name() {
        let err = renderer_for_extension("txt").unwrap_err();
        assert!(matches!(err, Error::UnsupportedFormat(ref e) if e == "txt"));
    }

    #[test]
    fn package_type_codes_map_to_renderers() {
        assert_eq!(renderer_for_type(0).unwrap(), Renderer::Video);
        assert_eq!(renderer_for_type(1).unwrap(), Renderer::WebFile);
        assert_eq!(renderer_for_type(2).unwrap(), Renderer::WebFile);
        assert_eq!(renderer_for_type(4).unwrap(), Renderer::Image);
        assert_eq!(renderer_for_type(5).unwrap(), Renderer::Image);
        assert_eq!(renderer_for_type(6).unwrap(), Renderer::Shader);
    }

    #[test]
    fn app_wallpaper_type_is_explicitly_unsupported() {
        assert!(matches!(
            renderer_for_type(3),
            Err(Error::UnknownWallpaperType(3))
        ));
        assert!(matches!(
            renderer_for_type(99),
            Err(Error::UnknownWallpaperType(99))
        ));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(EngineManager::resolve_target("").is_err());
        assert!(EngineManager::resolve_target("   ").is_err());
    }

    #[test]
    fn missing_path_is_reported_with_the_path() {
        let err = EngineManager::resolve_target("/definitely/not/here.mp4").unwrap_err();
        match err {
            Error::PathNotFound(p) => assert!(p.to_string_lossy().contains("here.mp4")),
            other => panic!("expected PathNotFound, got {other:?}"),
        }
    }

    #[test]
    fn file_without_extension_is_rejected() {
        let dir = tmpdir("noext");
        let f = dir.join("wallpaper");
        fs::write(&f, b"x").unwrap();

        let err = EngineManager::resolve_target(f.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_video_file_resolves() {
        let dir = tmpdir("video");
        let f = dir.join("clip.mp4");
        fs::write(&f, b"x").unwrap();

        let t = EngineManager::resolve_target(f.to_str().unwrap()).unwrap();
        assert_eq!(t.renderer, Renderer::Video);
        assert_eq!(t.location, f);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_directory_resolves_via_lively_info() {
        let dir = tmpdir("pkg");
        let proj = dir.join("MyWallpaper");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("clip.mp4"), b"x").unwrap();
        fs::write(
            proj.join("LivelyInfo.json"),
            br#"{"Title":"Test","Type":0,"FileName":"clip.mp4"}"#,
        )
        .unwrap();

        let t = EngineManager::resolve_target(proj.to_str().unwrap()).unwrap();
        assert_eq!(t.renderer, Renderer::Video);
        assert_eq!(t.location, proj.join("clip.mp4"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_with_missing_media_is_rejected() {
        let dir = tmpdir("pkgmissing");
        let proj = dir.join("Broken");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join("LivelyInfo.json"),
            br#"{"Title":"Test","Type":0,"FileName":"gone.mp4"}"#,
        )
        .unwrap();

        let err = EngineManager::resolve_target(proj.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, Error::PathNotFound(_)), "got {err:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_without_lively_info_is_rejected() {
        let dir = tmpdir("plain");
        let err = EngineManager::resolve_target(dir.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, Error::PathNotFound(_)), "got {err:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_package_json_is_reported_not_panicking() {
        let dir = tmpdir("badjson");
        let proj = dir.join("Bad");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("LivelyInfo.json"), b"{ nope").unwrap();

        let err = EngineManager::resolve_target(proj.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn volume_is_clamped() {
        let mut e = EngineManager::headless_for_tests();
        e.set_volume(5.0);
        assert_eq!(e.volume, 1.0);
        e.set_volume(-2.0);
        assert_eq!(e.volume, 0.0);
    }

    #[test]
    fn playback_state_toggles_correctly() {
        let mut e = EngineManager::headless_for_tests();
        assert!(!e.is_paused, "starts playing");

        // Playing -> paused, and toggle reports the new state.
        assert!(!e.toggle_play_pause(), "toggle must report paused");
        assert!(e.is_paused);

        // Paused -> playing.
        assert!(e.toggle_play_pause(), "toggle must report playing");
        assert!(!e.is_paused);
    }

    #[test]
    fn set_playback_state_sets_flag_consistently() {
        let mut e = EngineManager::headless_for_tests();

        e.set_playback_state(true);
        assert!(!e.is_paused, "play=true means not paused");

        e.set_playback_state(false);
        assert!(e.is_paused, "play=false means paused");
    }

    #[test]
    fn mute_toggles_and_keeps_volume_level() {
        let mut e = EngineManager::headless_for_tests();
        e.set_volume(0.42);
        assert!(e.toggle_mute());
        assert_eq!(e.volume, 0.42, "muting must not change the stored level");
        assert!(!e.toggle_mute());
    }

    #[test]
    fn operations_on_unknown_screen_are_rejected() {
        let mut e = EngineManager::headless_for_tests();
        assert!(e.seek(1, 50.0).is_err());
        assert!(e.set_property(1, "a", "b").is_err());
        assert!(e.take_screenshot(1, Path::new("/tmp/x.png")).is_err());
        assert!(e.close_wallpaper(1).is_err());
    }

    #[test]
    fn unknown_screen_error_names_the_attached_count() {
        let e = EngineManager::headless_for_tests();
        match e.find_screen_index(7) {
            Err(Error::UnknownScreen(idx, attached)) => {
                assert_eq!(idx, 7);
                assert_eq!(attached, 0);
            }
            other => panic!("expected UnknownScreen, got {other:?}"),
        }
    }

    #[test]
    fn find_screen_index_locates_an_existing_screen() {
        let mut e = EngineManager::headless_for_tests();
        e.screens.push(ScreenWallpaper {
            screen_index: 0,
            window: std::ptr::null_mut(),
            active_view: None,
            current_target: None,
        });
        e.screens.push(ScreenWallpaper {
            screen_index: 1,
            window: std::ptr::null_mut(),
            active_view: None,
            current_target: None,
        });

        assert_eq!(e.find_screen_index(0).unwrap(), 0);
        assert_eq!(e.find_screen_index(1).unwrap(), 1);
        assert!(e.find_screen_index(2).is_err());
    }

    #[test]
    fn set_wallpaper_refuses_to_run_off_the_main_thread() {
        // The unit-test harness runs each test on a worker thread, which is
        // exactly the situation the guard exists to catch: AppKit objects must
        // not be created there.
        let mut e = EngineManager::headless_for_tests();
        let result = e.set_wallpaper(0, "https://example.com");

        match result {
            Err(Error::NotOnMainThread(what)) => assert_eq!(what, "set_wallpaper"),
            // If the harness ever runs tests on the real main thread the guard
            // cannot fire, and we fall through to the no-screens-attached error.
            Err(Error::UnknownScreen(idx, 0)) => assert_eq!(idx, 0),
            other => panic!("expected a guard or bounds error, got {other:?}"),
        }
    }

    #[test]
    fn playback_labels_cover_every_state() {
        use crate::platform::PlaybackState;

        // Nothing to report while a video is simply doing its job.
        assert_eq!(PlaybackState::Playing.label(), None);

        assert_eq!(PlaybackState::Loading.label(), Some("Loading…"));
        assert_eq!(PlaybackState::Failed.label(), Some("Failed to load"));
        assert_eq!(PlaybackState::Paused.label(), Some("Paused"));
        assert_eq!(PlaybackState::Unknown.label(), Some("Idle"));
    }

    #[test]
    fn playback_state_codes_round_trip() {
        use crate::platform::PlaybackState;
        // The codes are shared with the macOS AVPlayer states; the mapping must
        // stay stable because the UI snapshot serialises them.
        for (code, expected) in [
            (0, PlaybackState::Unknown),
            (1, PlaybackState::Loading),
            (2, PlaybackState::Ready),
            (3, PlaybackState::Playing),
            (4, PlaybackState::Paused),
            (5, PlaybackState::Failed),
            (6, PlaybackState::Ended),
        ] {
            assert_eq!(PlaybackState::from_code(code), expected, "code {code}");
        }
    }

    #[test]
    fn video_playback_on_a_non_video_screen_is_an_error() {
        let e = EngineManager::headless_for_tests();
        assert!(e.video_playback(0).is_err());
    }

    #[test]
    fn renderer_reuse_covers_video_and_image_only() {
        // The reuse fast path exists for exactly these two renderers: they
        // allocate tens of megabytes (or a full AVFoundation object graph) per
        // instance, so rebuilding on every wallpaper change leaked badly.
        //
        // Web and shader deliberately do not reuse: their views are cheap, and a
        // web view is bound to the configuration it was created with.
        assert!(matches!(
            EngineManager::resolve_target("https://example.com/a.html"),
            Ok(ResolvedTarget {
                renderer: Renderer::WebUrl,
                ..
            })
        ));

        // A video target and an image target must report the renderers that
        // trigger reuse, so the engine branches above stay covered.
        assert_eq!(renderer_for_extension("mp4").unwrap(), Renderer::Video);
        assert_eq!(renderer_for_extension("png").unwrap(), Renderer::Image);
        assert_eq!(renderer_for_extension("html").unwrap(), Renderer::WebFile);
    }

    #[test]
    fn audio_bands_on_empty_slice_is_a_noop() {
        let e = EngineManager::headless_for_tests();
        e.send_audio_data(&[]); // must not panic
    }
}
