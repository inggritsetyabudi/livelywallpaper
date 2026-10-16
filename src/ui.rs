//! Settings UI bridge.
//!
//! Rust owns all application state. The SwiftUI layer is a pure view: it asks
//! for a snapshot, renders it, and sends user intents back as commands. It never
//! reads the config, the library, or the engine directly.
//!
//! That direction matters. A UI that touches the filesystem on its own will
//! eventually disagree with the daemon about what is running — the classic
//! "the toggle shows on but the wallpaper is paused" bug. Keeping one owner
//! makes that class of bug unrepresentable.

use std::ffi::{c_char, CStr, CString};

use serde::{Deserialize, Serialize};

use crate::config::LivelyConfig;
use crate::error::Result;
// The Objective-C bridge exists only on macOS. The rest of this module -- the
// JSON snapshot the CLI reads and the IPC action dispatcher -- is portable, so
// only the bridge is gated.
#[cfg(target_os = "macos")]
use crate::ffi;
use crate::library::LibraryManager;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// A request from the UI. One shape for every action keeps the Swift side to a
/// single decoding path instead of one per command.
#[derive(Debug, Deserialize)]
struct UiRequest {
    action: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    screen: Option<u32>,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    monitor: Option<i32>,
}

#[derive(Debug, Serialize)]
struct UiResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The fresh snapshot after the action ran.
    ///
    /// Omitted for actions that do not touch app state -- notably
    /// `playbackStatus`, which the window polls several times a second. Building
    /// a snapshot re-reads the whole wallpaper library and the config from
    /// disk, so returning one here would re-scan everything for a value the UI
    /// throws away.
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<StateSnapshot>,
    /// Present only when the caller asked for playback details.
    #[serde(skip_serializing_if = "Option::is_none")]
    playback: Option<Option<PlaybackView>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    library: Vec<LibraryEntry>,
    screens: Vec<ScreenEntry>,
    config: ConfigView,
    properties: Vec<PropertyEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    id: String,
    title: String,
    author: String,
    /// Ships with the app rather than imported by the user.
    is_builtin: bool,
    /// Licence text from LivelyInfo.json, shown in the metadata panel.
    license: String,
    /// Lively type code, for the "Video / Web / Image" badge.
    wallpaper_type: i32,
    type_label: &'static str,
    has_thumbnail: bool,
    thumbnail_path: Option<String>,
    /// Wallpaper type code as declared in LivelyInfo.json.
    description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenEntry {
    index: u32,
    name: String,
    width: u32,
    height: u32,
    /// Title of the wallpaper currently on this screen, if any.
    current_title: Option<String>,
    current_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    volume: f32,
    volume_percent: u32,
    is_muted: bool,
    is_paused: bool,
    pause_on_fullscreen: bool,
    pause_on_battery: bool,
    layout_mode: i32,
    audio_visualizer_enabled: bool,
}

/// Where a video wallpaper has got to, for the status line in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackView {
    /// 0 unknown, 1 loading, 2 ready, 3 playing, 4 paused, 5 failed, 6 ended.
    state: i32,
    /// Human-readable state, or null when it is simply playing.
    label: Option<&'static str>,
    position: f64,
    duration: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyEntry {
    key: String,
    /// `slider` | `checkbox` | `dropdown` | `color` | `button` | `text`
    prop_type: String,
    label: String,
    value: Option<serde_json::Value>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    options: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public Rust API
// ---------------------------------------------------------------------------

/// Show the settings window. Falls back to a log line if the UI is missing.
pub fn present() {
    // The platform decides whether there is a window to show. macOS presents
    // the SwiftUI settings window; the Linux build has none yet and says so
    // rather than pretending the command worked.
    if let Err(e) = crate::platform::present_ui() {
        log::warn!("could not present the settings window: {e}");
    }
}

/// Ask the UI to redraw from a fresh snapshot.
///
/// Called when the daemon changes state the UI is showing (an automation pause,
/// for instance) so the window never shows stale transport state.
#[allow(dead_code)]
pub fn refresh() {
    // SAFETY: `lively_ui_refresh` dispatches to the main queue internally.
    #[cfg(target_os = "macos")]
    unsafe {
        ffi::lively_ui_refresh();
    }
}

/// Whether the Swift half is linked in.
///
/// Lets the CLI report a clear message if the app was ever built without the UI.
#[allow(dead_code)]
pub fn available() -> bool {
    // The UI is a macOS-only artefact; there is nothing to link elsewhere.
    #[cfg(target_os = "macos")]
    // SAFETY: reads a Swift global; safe from any thread.
    unsafe {
        ffi::lively_ui_available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// FFI entry points
// ---------------------------------------------------------------------------

/// Installs the UI command handler.
///
/// Records it for this process *and* hands it to the Swift half. Doing only the
/// first leaves `commandHandler` nil on the Swift side, and the window then
/// reports "the settings UI is not connected to the daemon" for every action.
///
/// # Safety
///
/// Must be called on the main thread, before any UI action can arrive, with a
/// valid function pointer.
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn lively_ui_set_handler(handler: ffi::LivelyUiCommandHandler) {
    use std::sync::atomic::Ordering;

    // The handler is a plain `extern "C" fn` with no captures, so storing it in
    // a static is sound: it has no lifetime and no thread affinity beyond the
    // main thread, which is where every caller already is.
    HANDLER.store(handler as usize, Ordering::Release);

    // Swift stores the handler itself, so it must be told. The pointer crosses
    // as an integer because Swift cannot hold a Rust function pointer in a
    // typed property without a bit cast.
    //
    // This calls straight into the shim, *not* `platform::register_ui_handler`.
    // The platform entry point exists for the daemon to install the handler in
    // the first place; routing back through it here would make this function
    // call itself, and the recursion ends in a stack overflow during startup.
    ffi::lively_ui_install_handler(handler as usize);
}

/// # Safety
///
/// Called by Swift when it is finished with a response returned by the handler.
#[no_mangle]
pub unsafe extern "C" fn lively_ui_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Handle a UI request. Registered with Swift; must not panic.
#[no_mangle]
pub extern "C" fn ui_command_entry(request_json: *const c_char) -> *mut c_char {
    // A panic crossing the FFI boundary is UB (the Swift unwinder has nowhere
    // to go), so every path returns a value and panics are converted to an
    // error response.
    let result = std::panic::catch_unwind(|| handle_request(request_json));

    let response = match result {
        Ok(Ok(json)) => json,
        // `handle_request` uses its own error type, which the UI reports as text.
        Ok(Err(message)) => error_response(&message.to_string()),
        Err(_) => error_response("the daemon hit an internal error handling that action"),
    };

    match CString::new(response) {
        Ok(c) => c.into_raw(),
        // Only possible if the JSON contained an interior NUL, which serde_json
        // cannot emit.
        Err(_) => CString::new(r#"{"ok":false,"error":"response encoding failed"}"#)
            .expect("literal has no NUL")
            .into_raw(),
    }
}

static HANDLER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn error_response(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

fn handle_request(request_json: *const c_char) -> Result<String> {
    // SAFETY: Swift guarantees a NUL-terminated string. A null pointer is
    // tolerated rather than dereferenced.
    let raw = if request_json.is_null() {
        return Ok(error_response("empty request"));
    } else {
        // SAFETY: pointer is non-null and NUL-terminated per the contract.
        unsafe { CStr::from_ptr(request_json) }
            .to_str()
            .map_err(|_| crate::error::Error::Invalid("request was not valid UTF-8".into()))?
            .to_string()
    };

    let request: UiRequest = serde_json::from_str(&raw)
        .map_err(|e| crate::error::Error::Invalid(format!("malformed UI request: {e}")))?;

    let Some(state) = crate::app_state() else {
        return Ok(error_response("the engine is not ready yet"));
    };

    let selected = request.id.clone();
    let mut response = UiResponse {
        ok: true,
        error: None,
        state: None,
        playback: None,
    };

    // Run the action, then rebuild the snapshot so the UI renders the state
    // *after* the change. Returning a pre-action snapshot would make a
    // successful apply look like it did nothing.
    let wants_snapshot = action_needs_snapshot(&request.action);
    let outcome = dispatch(&request, state);
    if let Ok(Some(playback)) = &outcome {
        response.playback = Some(Some(playback.clone()));
    }

    let action_failed = match &outcome {
        Ok(_) => None,
        Err(e) => {
            log::warn!("ui action `{}` failed: {e}", request.action);
            Some(e.to_string())
        }
    };

    // A failed action still returns state so the controls resync rather than
    // showing stale values.
    if wants_snapshot || action_failed.is_some() {
        response.state = Some(build_snapshot(state, selected.as_deref())?);
    }

    if let Some(message) = action_failed {
        response.ok = false;
        response.error = Some(message);
    }

    Ok(serde_json::to_string(&response)?)
}

/// Whether the caller needs a full snapshot back.
///
/// Anything that changes state, or reads the library or screens, does. A pure
/// playback poll does not, and rebuilding the snapshot for it would re-read the
/// whole library from disk several times a second.
fn action_needs_snapshot(action: &str) -> bool {
    !matches!(action, "playbackStatus")
}

fn dispatch(request: &UiRequest, state: &AppState) -> Result<Option<PlaybackView>> {
    match request.action.as_str() {
        // A pure read: the snapshot was already built.
        "state" => {}

        "select" => {
            // Rebuilding the snapshot with this id attached is enough; the
            // property list is part of the response.
        }

        "apply" => {
            let id = require(request.id.as_deref(), "wallpaper id")?;
            let screen = request.screen.unwrap_or(0);
            let path = library_path(id)?;
            state.with_engine(|e| e.set_wallpaper(screen, &path))?;
            crate::ipc::record_active_wallpaper(screen, path.trim());
        }

        "closeWallpaper" => {
            let monitor = request.monitor.unwrap_or(0);
            state.with_engine(|e| e.close_wallpaper(monitor))?;
            if monitor < 0 {
                let mut cfg = LivelyConfig::load();
                cfg.active_wallpapers.clear();
                cfg.save()?;
            } else {
                crate::ipc::clear_active_wallpaper(monitor as u32);
            }
        }

        "addLibrary" => {
            let path = require(request.path.as_deref(), "path")?;
            let mut library = LibraryManager::new()?;
            let item = library.add_from_media(std::path::Path::new(&path))?;
            log::info!("imported {} as {}", path, item.id);
        }

        "removeLibrary" => {
            let id = require(request.id.as_deref(), "wallpaper id")?;
            let mut library = LibraryManager::new()?;
            library.remove(id)?;
        }

        "volume" => {
            let value = request
                .value
                .as_ref()
                .and_then(|v| v.as_f64())
                .ok_or_else(|| crate::error::Error::Invalid("volume needs a number".into()))?;
            state.with_engine(|e| {
                e.set_volume((value as f32 / 100.0).clamp(0.0, 1.0));
                Ok(())
            })?;
            persist_transport(state)?;
        }

        "toggleMute" => {
            state.with_engine(|e| {
                e.toggle_mute();
                Ok(())
            })?;
            persist_transport(state)?;
        }

        "setPlaying" => {
            let play = request
                .value
                .as_ref()
                .and_then(|v| v.as_bool())
                .ok_or_else(|| {
                    crate::error::Error::Invalid("setPlaying needs true or false".into())
                })?;
            state.with_engine(|e| {
                e.set_playback_state(play);
                Ok(())
            })?;
            persist_transport(state)?;
        }

        "setConfig" => {
            let key = require(request.key.as_deref(), "config key")?;
            let value = request.value.clone().unwrap_or(serde_json::Value::Null);
            let mut cfg = LivelyConfig::load();
            apply_config_change(&mut cfg, key, &value)?;
            cfg.save()?;
            // Automation flags are captured when the watcher starts, so restart it.
            if matches!(key, "pauseOnFullscreen" | "pauseOnBattery") {
                crate::restart_automation_watcher(state, &cfg)?;
            }
        }

        "playbackStatus" => {
            let screen = request.screen.unwrap_or(0);
            return Ok(state
                .with_engine(|e| e.video_playback(screen))
                .ok()
                .map(|p| PlaybackView {
                    state: p.state as i32,
                    label: p.state.label(),
                    position: p.position,
                    duration: p.duration,
                }));
        }

        "setProperty" => {
            let name = require(request.key.as_deref(), "property name")?;
            let value = request
                .value
                .as_ref()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let screen = request.screen.unwrap_or(0);
            state.with_engine(|e| e.set_property(screen, name, &value))?;
        }

        other => {
            return Err(crate::error::Error::Invalid(format!(
                "unknown action `{other}`"
            )));
        }
    }

    Ok(None)
}

/// Write live transport state back to the config file.
///
/// The engine owns this state at runtime; the config is how it survives a
/// restart. Persisting on change (rather than only at exit) means a crash or a
/// `kill -9` does not lose the user's volume and mute choice.
fn persist_transport(state: &AppState) -> Result<()> {
    let (volume, is_muted, is_paused) =
        state.with_engine(|e| Ok((e.volume, e.is_muted, e.is_paused)))?;

    let mut cfg = LivelyConfig::load();
    if cfg.volume == volume && cfg.is_muted == is_muted && cfg.is_paused == is_paused {
        return Ok(());
    }

    cfg.volume = volume;
    cfg.is_muted = is_muted;
    cfg.is_paused = is_paused;
    cfg.save()
}

fn apply_config_change(cfg: &mut LivelyConfig, key: &str, value: &serde_json::Value) -> Result<()> {
    match key {
        "pauseOnFullscreen" => cfg.pause_on_fullscreen = value.as_bool().unwrap_or(false),
        "pauseOnBattery" => cfg.pause_on_battery = value.as_bool().unwrap_or(false),
        "audioVisualizerEnabled" => cfg.audio_visualizer_enabled = value.as_bool().unwrap_or(false),
        "interactiveMouse" => cfg.interactive_mouse = value.as_bool().unwrap_or(false),
        "layoutMode" => {
            let mode = value.as_i64().unwrap_or(0);
            if !(0..=2).contains(&mode) {
                return Err(crate::error::Error::Invalid(format!(
                    "layout mode must be 0, 1 or 2 (got {mode})"
                )));
            }
            cfg.layout_mode = mode as i32;
        }
        other => {
            return Err(crate::error::Error::Invalid(format!(
                "unknown setting `{other}`"
            )));
        }
    }
    Ok(())
}

/// Unwrap an optional field into a required one, naming what was absent.
fn require<'a>(value: Option<&'a str>, what: &str) -> Result<&'a str> {
    value
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| crate::error::Error::Invalid(format!("missing {what}")))
}

/// Resolve a library id back to the path of its media file.
fn library_path(id: &str) -> Result<String> {
    let library = LibraryManager::new()?;
    let item = library
        .find(id)
        .ok_or_else(|| crate::error::Error::Invalid(format!("no wallpaper with id `{id}`")))?;

    // Apply the project folder, not the media file: the engine re-reads
    // LivelyInfo.json from it and picks the right renderer.
    Ok(item.path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// Read the daemon's state into the shape the UI renders.
///
/// `selected` is the library id whose properties should be attached, if any.
pub fn build_snapshot(state: &AppState, selected: Option<&str>) -> Result<StateSnapshot> {
    let library = LibraryManager::new()?;
    let cfg = LivelyConfig::load();

    // Live transport state comes from the engine, not the config file. Reading
    // it from disk would make the UI show the value from the last time we
    // happened to save — the classic "slider says 80, I set 35" bug. The config
    // is only authoritative for settings the engine does not own.
    let (volume, is_muted, is_paused) =
        state.with_engine(|e| Ok((e.volume, e.is_muted, e.is_paused)))?;

    let library_entries: Vec<LibraryEntry> = library
        .items
        .iter()
        .map(|item| LibraryEntry {
            id: item.id.clone(),
            title: item.title.clone(),
            author: item.author.clone(),
            is_builtin: LibraryManager::is_builtin(&item.id),
            license: item.license.clone(),
            wallpaper_type: item.wallpaper_type,
            type_label: type_label(item.wallpaper_type),
            has_thumbnail: item.thumbnail_path.is_some(),
            thumbnail_path: item
                .thumbnail_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            description: item.desc.clone(),
        })
        .collect();

    let properties = selected
        .map(|id| properties_for(&library, id))
        .unwrap_or_default();

    let screens = screen_entries(state)?;

    Ok(StateSnapshot {
        library: library_entries,
        screens,
        config: ConfigView {
            volume,
            volume_percent: (volume * 100.0).round() as u32,
            is_muted,
            is_paused,
            pause_on_fullscreen: cfg.pause_on_fullscreen,
            pause_on_battery: cfg.pause_on_battery,
            layout_mode: cfg.layout_mode,
            audio_visualizer_enabled: cfg.audio_visualizer_enabled,
        },
        properties,
    })
}

fn screen_entries(state: &AppState) -> Result<Vec<ScreenEntry>> {
    let cfg = LivelyConfig::load();
    let library = LibraryManager::new()?;

    let count = state.with_engine(|e| Ok(e.screen_count()))?;
    let mut screens = Vec::with_capacity(count);

    for index in 0..count {
        let (name, width, height) = crate::platform::screen_info(index as u32);
        let path = cfg.active_wallpapers.get(&(index as u32)).cloned();

        // Map the stored path back to a library id so the UI can show what is
        // already applied and keep the selection in sync.
        let (current_id, current_title) = match &path {
            Some(p) => library
                .items
                .iter()
                .find(|item| item.path.to_string_lossy() == *p)
                .map(|item| (Some(item.id.clone()), Some(item.title.clone())))
                .unwrap_or_else(|| (None, Some(shorten(p)))),
            None => (None, None),
        };

        screens.push(ScreenEntry {
            index: index as u32,
            name,
            width,
            height,
            current_title,
            current_id,
        });
    }

    Ok(screens)
}

fn shorten(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    if name.chars().count() > 28 {
        format!("{}…", name.chars().take(27).collect::<String>())
    } else {
        name
    }
}

fn properties_for(library: &LibraryManager, id: &str) -> Vec<PropertyEntry> {
    let Some(item) = library.find(id) else {
        return Vec::new();
    };

    let Ok(info) = crate::package::LivelyInfo::load_from_dir(&item.path) else {
        return Vec::new();
    };
    let Some(props) = info.load_properties() else {
        return Vec::new();
    };

    let mut entries: Vec<PropertyEntry> = props
        .into_iter()
        .map(|(key, item)| PropertyEntry {
            key,
            prop_type: item.prop_type.clone(),
            label: if item.text.is_empty() {
                item.prop_type.clone()
            } else {
                item.text.clone()
            },
            value: item.value.as_ref().map(to_json),
            min: item.min,
            max: item.max,
            step: item.step,
            options: item.options.clone().unwrap_or_default(),
        })
        .collect();

    // Stable order: the file order is a HashMap's, which would make the panel
    // reshuffle on every refresh.
    entries.sort_by_key(|e| e.label.to_lowercase());
    entries
}

fn to_json(value: &crate::package::PropertyValue) -> serde_json::Value {
    use crate::package::PropertyValue;
    match value {
        PropertyValue::Number(n) => {
            // Whole numbers round-trip as integers; SwiftUI sliders and
            // steppers both bind more predictably to `Int`.
            if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                serde_json::json!(*n as i64)
            } else {
                serde_json::json!(n)
            }
        }
        PropertyValue::String(s) => serde_json::json!(s),
        PropertyValue::Bool(b) => serde_json::json!(b),
    }
}

pub fn type_label(wallpaper_type: i32) -> &'static str {
    match wallpaper_type {
        0 => "Video",
        1 | 2 => "Web",
        3 => "App",
        4 => "GIF",
        5 => "Image",
        6 => "Shader",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_labels_cover_the_lively_codes() {
        assert_eq!(type_label(0), "Video");
        assert_eq!(type_label(1), "Web");
        assert_eq!(type_label(2), "Web");
        assert_eq!(type_label(5), "Image");
        assert_eq!(type_label(6), "Shader");
        assert_eq!(type_label(99), "Unknown");
    }

    #[test]
    fn property_values_map_to_json() {
        use crate::package::PropertyValue;
        assert_eq!(to_json(&PropertyValue::Number(1.5)), serde_json::json!(1.5));
        assert_eq!(to_json(&PropertyValue::Bool(true)), serde_json::json!(true));
        assert_eq!(
            to_json(&PropertyValue::String("x".into())),
            serde_json::json!("x")
        );
    }

    #[test]
    fn request_deserializes_with_only_an_action() {
        let req: UiRequest = serde_json::from_str(r#"{"action":"state"}"#).unwrap();
        assert_eq!(req.action, "state");
        assert!(req.id.is_none());
        assert!(req.screen.is_none());
    }

    #[test]
    fn request_deserializes_full_fields() {
        let req: UiRequest =
            serde_json::from_str(r#"{"action":"apply","id":"abc","screen":2}"#).unwrap();
        assert_eq!(req.action, "apply");
        assert_eq!(req.id.as_deref(), Some("abc"));
        assert_eq!(req.screen, Some(2));
    }

    #[test]
    fn malformed_request_is_rejected() {
        assert!(serde_json::from_str::<UiRequest>("{ nope").is_err());
        assert!(serde_json::from_str::<UiRequest>("{}").is_err());
    }

    #[test]
    fn config_changes_are_validated() {
        let mut cfg = LivelyConfig::default();

        apply_config_change(&mut cfg, "pauseOnBattery", &serde_json::json!(false)).unwrap();
        assert!(!cfg.pause_on_battery);

        apply_config_change(&mut cfg, "layoutMode", &serde_json::json!(2)).unwrap();
        assert_eq!(cfg.layout_mode, 2);

        // Out of range and unknown keys must be refused rather than written.
        assert!(apply_config_change(&mut cfg, "layoutMode", &serde_json::json!(9)).is_err());
        assert!(apply_config_change(&mut cfg, "nonsense", &serde_json::json!(1)).is_err());
        assert_eq!(cfg.layout_mode, 2, "a rejected change must not be applied");
    }

    #[test]
    fn error_response_is_valid_json() {
        let json = error_response("boom");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["ok"], serde_json::json!(false));
        assert_eq!(parsed["error"], serde_json::json!("boom"));
    }

    #[test]
    fn shorten_truncates_long_names() {
        assert_eq!(shorten("/tmp/a.mp4"), "a.mp4");
        let long = "/tmp/".to_string() + &"x".repeat(100);
        let out = shorten(&long);
        assert!(
            out.chars().count() <= 28,
            "got {} chars",
            out.chars().count()
        );
        assert!(out.ends_with('…'));
    }
}
