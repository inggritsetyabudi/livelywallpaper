//! Lively Wallpaper daemon entry point.
//!
//! Two roles in one binary:
//!   * **client** — a CLI invocation that forwards to a running daemon over IPC
//!   * **daemon** — started when no daemon is listening; owns the UI
//!
//! Keeping both in one binary means `lively set ...` "just works" regardless of
//! whether the daemon happens to be running, with no separate launcher.

use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;

use clap::{CommandFactory, Parser, Subcommand};
use log::{debug, error, info};

mod automation;
mod config;
mod engine;
mod error;
// The Objective-C bridge. macOS-only by nature -- on Linux the equivalent
// boundary is `platform::linux_sys`, which talks to XCB and Wayland instead.
// This was previously compiled everywhere, which put AppKit-only symbols into
// shared code and made the Linux target fail to link.
#[cfg(target_os = "macos")]
mod ffi;
mod ipc;
mod library;
mod package;
mod platform;
mod state;
mod ui;

use crate::automation::AutomationWatcher;
use crate::automation::PauseCause;
use crate::config::LivelyConfig;
use crate::engine::EngineManager;
use crate::error::{Error, Result};
use crate::ipc::IpcResponse;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "lively",
    version,
    about = "Lively Wallpaper — animated desktop wallpapers for macOS",
    long_about = None
)]
struct Cli {
    /// Print verbose diagnostics.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Set a wallpaper on a screen (file, folder, package, or URL)
    Set {
        /// Path to a media file, a Lively project folder, or an http(s) URL
        path: String,
        /// Screen index; use `lively screens` to list them
        #[arg(short, long, default_value_t = 0)]
        monitor: u32,
    },
    /// Remove the wallpaper from a screen (leaves the screen usable)
    Close {
        /// Screen index, or -1 for every screen
        #[arg(short, long, default_value_t = -1)]
        monitor: i32,
    },
    /// Pause or resume playback
    #[command(visible_alias = "playpause")]
    Play {
        /// Resume playback
        #[arg(long)]
        resume: bool,
    },
    /// Pause playback
    Pause,
    /// Set the wallpaper volume
    Volume {
        /// Target volume 0-100, or a relative change like `+10` / `-5`.
        /// `allow_hyphen_values` is required so `-5` is read as a negative
        /// number rather than mistaken for a flag.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Jump to a position in the video
    Seek {
        /// Target position 0-100, or a relative change like `+10` / `-5`
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Screen index
        #[arg(short, long, default_value_t = 0)]
        monitor: u32,
    },
    /// Send a user-defined property to a web wallpaper
    SetProp {
        /// Property name
        name: String,
        /// Property value
        value: String,
        /// Screen index
        #[arg(short, long, default_value_t = 0)]
        monitor: u32,
    },
    /// Save a screenshot of a wallpaper to a file
    Screenshot {
        /// Destination path; `.png` selects PNG, anything else JPEG
        #[arg(short, long)]
        file: PathBuf,
        /// Screen index
        #[arg(short, long, default_value_t = 0)]
        monitor: u32,
    },
    /// List attached screens
    Screens,
    /// List wallpapers in the library
    #[command(visible_alias = "ls")]
    ListLibrary,
    /// Import a file or Lively project into the library
    AddLibrary {
        /// Path to the media file or project folder to import
        path: PathBuf,
    },
    /// Remove a wallpaper from the library
    RemoveLibrary {
        /// Item id, as shown by `lively list-library`
        id: String,
    },
    /// Run the daemon in the foreground. Normally started by the login item;
    /// run it directly to debug or to run without a menu-bar app.
    Daemon {
        /// Also open the screensaver fullscreen once the daemon is ready.
        #[arg(long)]
        screensaver: bool,
    },
    /// Play the screensaver wallpaper fullscreen until dismissed
    Screensaver {
        #[command(subcommand)]
        action: ScreensaverAction,
    },
    /// Show the menu-bar app (currently unimplemented)
    ShowApp,
    /// Stop the daemon
    Shutdown,
    /// Send a raw request to the settings UI bridge and print the reply.
    ///
    /// Hidden: this is the debugging surface for the SwiftUI bridge, not
    /// something a user should type. Useful for confirming the two halves agree
    /// on the wire format without clicking through the window.
    #[command(name = "__ui", hide = true)]
    UiProbe {
        /// Request JSON, e.g. `{"action":"state"}`
        request: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ScreensaverAction {
    Start {
        /// Screen index
        #[arg(short, long, default_value_t = 0)]
        monitor: u32,
    },
    Stop,
}

/// How long the main loop blocks per iteration. 50ms keeps the tray responsive
/// while costing ~20 wakeups/second, which is negligible.
const EVENT_PUMP_TIMEOUT: f64 = 0.05;

/// Exit signal raised by native callbacks (tray Quit, screensaver dismissal).
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Why the daemon is shutting down, for logging.
static SHUTDOWN_REASON: AtomicI32 = AtomicI32::new(0);

/// The shared engine, reachable from native C callbacks.
///
/// `OnceLock` rather than `static mut`: the value is published exactly once and
/// read-only afterwards, so no torn reads are possible.
static APP_STATE: OnceLock<AppState> = OnceLock::new();

/// The live application state, once the daemon has built it.
///
/// Returns `None` before startup completes. The UI uses this to refuse actions
/// rather than mutating a half-initialised engine.
pub fn app_state() -> Option<&'static AppState> {
    APP_STATE.get()
}

/// Reason codes for [`SHUTDOWN_REASON`].
mod reason {
    pub const TRAY_QUIT: i32 = 1;
    pub const SCREENSAVER_DISMISSED: i32 = 2;
    pub const IPC: i32 = 3;
}

fn main() {
    init_logging();
    let cli = Cli::parse();

    let code = match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn init_logging() {
    // Level is decided from the flag before the logger exists, so parse the
    // env directly rather than depending on clap having run.
    let level = if std::env::var("LIVELY_LOG").is_ok() {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let _ = env_logger::Builder::new().filter_level(level).try_init();
}

/// True when this process was started from inside an `.app` bundle.
///
/// macOS requires that layout for a Dock icon, a menu bar item, or Finder
/// activation, so "am I bundled?" is the reliable way to tell "double-clicked
/// the app" apart from "typed a command in a terminal".
fn launched_from_bundle() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    exe.to_string_lossy().contains(".app/Contents/MacOS/")
}

fn run(cli: Cli) -> Result<()> {
    debug!("lively starting (verbose={})", cli.verbose);

    let command = match cli.command {
        Some(c) => c,
        None => {
            // Launched from Finder with no arguments: behave like the menu-bar app
            // it is — start the daemon and open the settings window. From a
            // terminal with no arguments, print help as usual.
            if launched_from_bundle() {
                info!("launched from the app bundle: starting the daemon");
                // Show the settings window, not the screensaver: double-clicking
                // the app should open its UI.
                return run_daemon(false, true);
            }
            return Cli::command()
                .print_help()
                .map_err(|e| Error::Invalid(e.to_string()));
        }
    };

    // Commands that never need a daemon: handle locally and exit.
    match &command {
        Commands::Screens => return print_screens(),
        Commands::ListLibrary => return print_library(),
        Commands::AddLibrary { path } => return add_to_library(path),
        Commands::RemoveLibrary { id } => return remove_from_library(id),
        Commands::UiProbe { request } => return ui_probe(request),
        _ => {}
    }

    // Fail fast on bad input, before reporting that no daemon is running: the
    // user needs to see the real problem (bad path, bad number), not a
    // side-quest about process management.
    validate_arguments(&command)?;

    // Everything else: hand off to a running daemon if there is one.
    if ipc::is_daemon_running() {
        return dispatch_to_daemon(command);
    }

    // No daemon. For commands whose whole purpose is to change what is running,
    // starting one is what the user meant -- refusing just makes them learn an
    // incantation first. `shutdown` is excluded on principle: starting a daemon
    // in order to immediately stop it is absurd.
    if wants_running_daemon(&command) && auto_start_daemon()? {
        return dispatch_to_daemon(command);
    }

    // Only these commands may start a daemon. Everything else must fail loudly:
    // silently backgrounding a process the user did not ask for is surprising,
    // and for `shutdown` it would block forever waiting for work that never
    // arrives.
    match &command {
        Commands::Daemon { screensaver } => {
            return run_daemon(*screensaver, false);
        }
        Commands::Screensaver {
            action: ScreensaverAction::Start { .. },
        } => {
            return run_daemon(true, true);
        }
        _ => {}
    }

    Err(Error::Ipc(
        "no daemon is running. Start one with `lively daemon`.".into(),
    ))
}

/// Whether this command only makes sense while the daemon is running.
fn wants_running_daemon(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Set { .. }
            | Commands::Close { .. }
            | Commands::Play { .. }
            | Commands::Pause
            | Commands::Volume { .. }
            | Commands::Seek { .. }
            | Commands::SetProp { .. }
            | Commands::Screenshot { .. }
    )
}

/// Start a detached daemon and wait for it to answer.
///
/// Spawning the same executable with `daemon` keeps one binary authoritative
/// about how the daemon starts. `LIVELY_NO_AUTOSTART=1` opts out, which is what
/// scripts that manage the process themselves need.
fn auto_start_daemon() -> Result<bool> {
    if std::env::var_os("LIVELY_NO_AUTOSTART").is_some() {
        return Ok(false);
    }

    let Ok(exe) = std::env::current_exe() else {
        return Ok(false);
    };

    info!("no daemon running; starting one");

    let spawned = std::process::Command::new(exe)
        .arg("daemon")
        // Detach: the daemon outlives this CLI invocation.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    if spawned.is_err() {
        return Ok(false);
    }

    // Poll for readiness rather than sleeping a fixed amount: a slow disk
    // makes the daemon start slower, and a fixed sleep would be either too
    // long on a fast machine or too short on a slow one.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if ipc::is_daemon_running() {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(false)
}

/// Check everything that can be checked without touching the daemon.
///
/// The daemon re-validates everything — this is purely so the user gets a
/// useful message immediately instead of a transport error.
fn validate_arguments(command: &Commands) -> Result<()> {
    match command {
        Commands::Set { path, .. } => {
            let resolved = resolve_set_target(path)?;
            EngineManager::resolve_target(&resolved)?;
        }
        Commands::Volume { value } => {
            // Syntax only: whether a *relative* volume lands in range depends
            // on the current level, which only the daemon knows. Checking here
            // against a made-up baseline would reject valid commands.
            parse_adjustment(value)?;
        }
        Commands::Seek { value, .. } => {
            parse_percentage(value)?;
        }
        Commands::Screenshot { file, .. } => {
            if file.as_os_str().is_empty() {
                return Err(Error::Invalid("screenshot path is empty".into()));
            }
        }
        Commands::SetProp { name, .. } if name.trim().is_empty() => {
            return Err(Error::Invalid("property name is empty".into()));
        }
        _ => {}
    }
    Ok(())
}

/// Resolve what the user typed for `set` into something the engine can load.
///
/// A bare name is almost always a library id -- that is what the UI shows and
/// what `lively list-library` prints -- so treat it as one before reporting
/// "no such path". Paths, URLs and anything containing a separator are left
/// alone.
fn resolve_set_target(input: &str) -> Result<String> {
    let trimmed = input.trim();

    let looks_like_path = trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('~')
        || crate::engine::is_url(trimmed);

    if !looks_like_path && !Path::new(trimmed).exists() {
        if let Ok(library) = library::LibraryManager::new() {
            if let Some(item) = library.find(trimmed) {
                return Ok(item.path.to_string_lossy().into_owned());
            }
        }

        // Not an id either: offer the nearest titles so the user can see what
        // they could have typed.
        let Ok(library) = library::LibraryManager::new() else {
            return Ok(input.to_string());
        };
        let suggestions: Vec<String> = library
            .items
            .iter()
            .filter(|item| item.title.to_lowercase().contains(&trimmed.to_lowercase()))
            .take(3)
            .map(|item| format!("{} (id: {})", item.title, item.id))
            .collect();

        if suggestions.is_empty() {
            return Err(Error::Invalid(format!(
                "`{trimmed}` is neither an existing path nor a library item"
            )));
        }
        return Err(Error::Invalid(format!(
            "`{trimmed}` is a library item name, not an id. Did you mean: {}",
            suggestions.join(", ")
        )));
    }

    Ok(input.to_string())
}

/// Forward a command to the running daemon and relay its reply.
fn dispatch_to_daemon(command: Commands) -> Result<()> {
    let cmd = to_ipc_command(&command);
    let resp = ipc::send_command(&cmd)?;

    if resp.ok {
        println!("{}", resp.message);
        Ok(())
    } else {
        Err(Error::Ipc(resp.message))
    }
}

/// Forward a request straight to the UI command handler and print its reply.
///
/// Bridges a gap that is otherwise invisible: if the Swift and Rust halves
/// disagree about the JSON shape, this shows the exact bytes on both sides.
fn ui_probe(request: &str) -> Result<()> {
    // The bridge lives in the daemon; this process has no engine. Forward it.
    let response = ipc::send_command(&ipc::IpcCommand::UiProbe {
        request: request.to_string(),
    })?;

    if !response.ok {
        return Err(Error::Ipc(response.message));
    }
    println!("{}", response.message);
    Ok(())
}

/// Run a UI-bridge request inside this process. Daemon side of [`ui_probe`].
fn run_ui_probe(request: &str) -> String {
    let Ok(c_request) = CString::new(request) else {
        return r#"{"ok":false,"error":"request contained a NUL byte"}"#.to_string();
    };

    // SAFETY: `c_request` is a valid NUL-terminated string that outlives the
    // call. The handler returns a CString we own and free here.
    let raw = ui::ui_command_entry(c_request.as_ptr());
    if raw.is_null() {
        return r#"{"ok":false,"error":"the UI handler returned nothing"}"#.to_string();
    }
    let text = unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { ui::lively_ui_string_free(raw) };
    text
}

/// Translate CLI arguments into the wire format.
fn to_ipc_command(command: &Commands) -> ipc::IpcCommand {
    match command {
        // Handled locally before we ever get here. Mapped to a harmless command
        // so the match stays total without pretending it does something.
        Commands::Screens
        | Commands::ListLibrary
        | Commands::AddLibrary { .. }
        | Commands::RemoveLibrary { .. } => ipc::IpcCommand::ShowApp,

        Commands::Set { path, monitor } => ipc::IpcCommand::SetWallpaper {
            // Resolved here as well: the daemon receives a concrete path, and a
            // client that skipped validation still gets sane behaviour.
            path: resolve_set_target(path).unwrap_or_else(|_| path.clone()),
            monitor: *monitor,
        },
        Commands::Close { monitor } => ipc::IpcCommand::CloseWallpaper { monitor: *monitor },
        Commands::Play { resume } => ipc::IpcCommand::PlayPause {
            play: if *resume { Some(true) } else { Some(false) },
        },
        Commands::Pause => ipc::IpcCommand::PlayPause { play: Some(false) },
        Commands::Volume { value } => ipc::IpcCommand::Volume {
            value: value.clone(),
        },
        Commands::Seek { value, monitor } => ipc::IpcCommand::Seek {
            monitor: *monitor,
            value: value.clone(),
        },
        Commands::SetProp {
            name,
            value,
            monitor,
        } => ipc::IpcCommand::SetProp {
            monitor: *monitor,
            name: name.clone(),
            value: value.clone(),
        },
        Commands::Screenshot { file, monitor } => ipc::IpcCommand::Screenshot {
            monitor: *monitor,
            file: file.to_string_lossy().into_owned(),
        },
        Commands::Screensaver { action } => match action {
            ScreensaverAction::Start { monitor } => {
                ipc::IpcCommand::StartScreensaver { monitor: *monitor }
            }
            ScreensaverAction::Stop => ipc::IpcCommand::StopScreensaver,
        },
        Commands::Daemon { .. } | Commands::UiProbe { .. } | Commands::ShowApp => {
            ipc::IpcCommand::ShowApp
        }
        Commands::Shutdown => ipc::IpcCommand::Shutdown,
    }
}

// ---------------------------------------------------------------------------
// Local (daemon-less) commands
// ---------------------------------------------------------------------------

fn print_screens() -> Result<()> {
    if !platform::ensure_app_initialised() {
        return Err(Error::Invalid(
            "could not initialise the platform display connection".into(),
        ));
    }
    let count = platform::screen_count();
    if count == 0 {
        println!("no screens attached");
        return Ok(());
    }

    println!(
        "{:<4} {:<24} {:>10} x {:<10}",
        "IDX", "NAME", "WIDTH", "HEIGHT"
    );
    for i in 0..count {
        let (name, w, h) = platform::screen_info(i);
        println!("{:<4} {:<24} {:>10} x {:<10}", i, name, w, h);
    }
    Ok(())
}

fn print_library() -> Result<()> {
    let lib = library::LibraryManager::new()?;
    if lib.items.is_empty() {
        println!("library is empty. import something with `lively add-library <path>`");
        return Ok(());
    }
    println!("{:<28} {:<4} {:<10} TITLE", "ID", "TYPE", "SIZE");
    for item in &lib.items {
        let size = item
            .path
            .join(read_file_name(&item.path))
            .metadata()
            .map(|m| human_bytes(m.len()))
            .unwrap_or_else(|_| "-".to_string());
        println!(
            "{:<28} {:<4} {:<10} {}",
            item.id, item.wallpaper_type, size, item.title
        );
    }
    Ok(())
}

fn read_file_name(dir: &Path) -> String {
    std::fs::read_dir(dir)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .find(|e| {
                    e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| x != "json")
                })
                .and_then(|e| e.file_name().into_string().ok())
        })
        .unwrap_or_default()
}

fn add_to_library(path: &Path) -> Result<()> {
    let mut lib = library::LibraryManager::new()?;
    let item = lib.add_from_media(path)?;
    println!("imported {} as {}", path.display(), item.id);
    Ok(())
}

fn remove_from_library(id: &str) -> Result<()> {
    let mut lib = library::LibraryManager::new()?;
    lib.remove(id)?;
    println!("removed {id}");
    Ok(())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}{}", UNITS[0])
    } else {
        format!("{size:.1}{}", UNITS[unit])
    }
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

fn run_daemon(start_screensaver: bool, show_ui: bool) -> Result<()> {
    info!("starting daemon");

    let config = LivelyConfig::load();
    let mut engine = EngineManager::new()?;

    // Restore transport state before anything can observe it, so the first
    // snapshot the UI renders already matches the saved preferences.
    engine.volume = config.volume;
    engine.is_muted = config.is_muted;
    engine.is_paused = config.is_paused;

    let app_state = AppState::new(engine);
    // Publish before installing callbacks so a callback can never observe an
    // uninitialised engine.
    let state = APP_STATE.get_or_init(|| app_state);

    restore_wallpapers(state, &config);

    // Give the UI a way to talk back before anything can present it.
    //
    // Routed through the platform: macOS installs the Swift handler, and a
    // platform with no bundled UI does nothing. Calling the shim directly here
    // is what left the Linux build unable to link.
    platform::register_ui_handler();

    platform::install_tray_menu(tray_callback);
    update_tray_title("Lively");

    let ipc_server = ipc::IpcServer::bind()?;

    install_watcher(&config)?;

    let screen_count = state.engine()?.screen_count();
    info!("daemon ready ({screen_count} screens)");

    if show_ui {
        // Opening the window switches the app from menu-bar-only to regular, so
        // it is scheduled after the run loop is live rather than before it.
        ui::present();
    }

    if start_screensaver {
        // Opened after the loop is live so the dismissal callback has a running
        // run loop to tear down.
        state.with_engine(|e| e.start_screensaver(0, screensaver_dismiss_callback))?;
        info!("screensaver showing");
    }

    // Main loop: service IPC, then let AppKit breathe.
    while !SHUTDOWN.load(Ordering::Relaxed) {
        ipc_server.poll_with(|cmd| match handle_command_with_message(state, cmd) {
            Ok(message) => IpcResponse::ok(message),
            Err(e) => {
                error!("command failed: {e}");
                IpcResponse::err(e.to_string())
            }
        });

        // Apply any pause transition the automation watcher observed. Done here
        // rather than in the watcher thread because it touches the engine and
        // the status bar, both of which are main-thread-only.
        drain_pending_pause(state);

        // SAFETY: runs on the main thread, inside the run loop we own.
        platform::pump_events(EVENT_PUMP_TIMEOUT);
    }

    shutdown_reason(reason_of(SHUTDOWN_REASON.load(Ordering::Relaxed)));

    // Stop the watcher before teardown so no thread touches a dying engine.
    drop(WATCHER.lock().map_err(|_| Error::poisoned())?.take());
    state.engine()?.stop_screensaver();

    persist_shutdown_state(state);
    Ok(())
}

fn reason_of(code: i32) -> &'static str {
    match code {
        reason::TRAY_QUIT => "quit from the menu bar",
        reason::SCREENSAVER_DISMISSED => "screensaver dismissed",
        reason::IPC => "shutdown requested over IPC",
        _ => "unknown",
    }
}

fn shutdown_reason(text: &str) {
    info!("shutting down ({text})");
}

/// Re-apply the wallpapers recorded in the previous session.
fn restore_wallpapers(state: &AppState, config: &LivelyConfig) {
    if config.active_wallpapers.is_empty() {
        return;
    }

    let snapshots: Vec<(u32, String)> = config
        .active_wallpapers
        .iter()
        .map(|(k, v)| (*k, v.clone()))
        .collect();

    for (screen, path) in snapshots {
        let result = state
            .engine()
            .and_then(|mut e| e.set_wallpaper(screen, &path));
        match result {
            Ok(()) => info!("restored wallpaper on screen {screen}"),
            // The file may have been deleted since last run; that must not stop
            // the daemon from booting.
            Err(e) => {
                error!("could not restore screen {screen} ({e})");
                ipc::clear_active_wallpaper(screen);
            }
        }
    }
}

/// Re-create the automation watcher after the user changes a toggle.
///
/// The watcher captures the two flags when it is created, so a settings change
/// has no effect until it is rebuilt. Done inline rather than through a channel
/// so the new state is guaranteed to be in force before the UI redraws.
pub fn restart_automation_watcher(_state: &AppState, config: &LivelyConfig) -> Result<()> {
    let mut slot = WATCHER.lock().map_err(|_| Error::poisoned())?;
    // Dropping the old watcher joins its thread, so this happens before the new
    // one exists and the two can never sample concurrently.
    drop(slot.take());
    *slot = Some(start_automation_watcher(config));
    Ok(())
}

/// The running watcher, kept behind a mutex so it can be swapped at runtime.
///
/// `Option<AutomationWatcher>` because `Drop` joins the worker thread, which
/// must not happen from the thread being dropped.
static WATCHER: std::sync::Mutex<Option<AutomationWatcher>> = std::sync::Mutex::new(None);

fn install_watcher(config: &LivelyConfig) -> Result<()> {
    *WATCHER.lock().map_err(|_| Error::poisoned())? = Some(start_automation_watcher(config));
    Ok(())
}

fn start_automation_watcher(config: &LivelyConfig) -> AutomationWatcher {
    let pause_on_fullscreen = config.pause_on_fullscreen;
    let pause_on_battery = config.pause_on_battery;

    // The callback fires on a worker thread. It must touch nothing AppKit, so it
    // only records state; the main loop applies it on the next iteration.
    AutomationWatcher::start(pause_on_fullscreen, pause_on_battery, move |cause| {
        PENDING_PAUSE_CAUSE.store(
            cause.map(|c| c as u8 as i8).unwrap_or(-1),
            Ordering::Relaxed,
        );
        notify_main_loop();
    })
}

/// Pause cause observed by the watcher, waiting to be applied by the main loop.
/// `-1` means "not paused"; otherwise the `PauseCause` discriminant.
static PENDING_PAUSE_CAUSE: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

/// Wake the main loop so a pending change is applied without waiting a full tick.
fn notify_main_loop() {
    // Nudge AppKit from another thread; this is explicitly allowed and simply
    // makes the run loop return so we can service state sooner.
    platform::dispatch_on_main(nop_on_main, std::ptr::null_mut());
}

extern "C" fn nop_on_main(_ctx: *mut c_void) {}

/// Apply any pause transition the watcher has recorded. Main thread only.
fn drain_pending_pause(state: &AppState) {
    let raw = PENDING_PAUSE_CAUSE.swap(-1, Ordering::Relaxed);
    let cause = match raw {
        -1 => return,
        0 => Some(PauseCause::Fullscreen),
        1 => Some(PauseCause::Battery),
        2 => Some(PauseCause::FullscreenAndBattery),
        _ => return,
    };

    let should_play = cause.is_none();
    let label = cause.map(|c| c.label()).unwrap_or("Playing");

    let changed = state
        .engine()
        .map(|mut e| {
            if e.is_paused == should_play {
                false
            } else {
                e.set_playback_state(should_play);
                true
            }
        })
        .unwrap_or(false);

    if changed {
        update_tray_title(&tray_title_for(should_play, cause));
        info!("{label}");
        persist_pause_state(should_play);
    }
}

fn tray_title_for(playing: bool, cause: Option<PauseCause>) -> String {
    match (playing, cause) {
        (true, _) => "Lively".to_string(),
        (false, Some(c)) => c.label().to_string(),
        (false, None) => "Lively (paused)".to_string(),
    }
}

fn persist_pause_state(is_paused: bool) {
    let mut cfg = LivelyConfig::load();
    cfg.is_paused = is_paused;
    if let Err(e) = cfg.save() {
        error!("could not persist pause state: {e}");
    }
}

/// Write live engine state into the saved config at shutdown.
///
/// The config is re-read here rather than reusing the copy loaded at startup:
/// that copy is stale for anything the user changed mid-session (automation
/// toggles, layout mode), and saving it back would silently undo those changes.
fn persist_shutdown_state(state: &AppState) {
    let Ok(engine) = state.with_engine(|e| Ok((e.volume, e.is_muted, e.is_paused))) else {
        return;
    };

    let mut cfg = LivelyConfig::load();
    cfg.volume = engine.0;
    cfg.is_muted = engine.1;
    cfg.is_paused = engine.2;

    if let Err(e) = cfg.save() {
        error!("could not persist config on exit: {e}");
    }
}

fn update_tray_title(title: &str) {
    platform::set_tray_title(title);
}

// ---------------------------------------------------------------------------
// Command handling
// ---------------------------------------------------------------------------

/// Execute a command, discarding any reply text.
fn handle_command(state: &AppState, cmd: ipc::IpcCommand) -> Result<()> {
    handle_command_with_message(state, cmd).map(|_| ())
}

/// Execute a command and return the text to send back to the client.
///
/// Most commands acknowledge with `"ok"`, but the UI probe has to hand raw JSON
/// across, so the reply is part of the contract rather than a side effect.
fn handle_command_with_message(
    state: &AppState,
    cmd: ipc::IpcCommand,
) -> std::result::Result<String, Error> {
    match run_command(state, cmd) {
        Ok(Some(message)) => Ok(message),
        Ok(None) => Ok("ok".to_string()),
        Err(e) => Err(e),
    }
}

fn run_command(state: &AppState, cmd: ipc::IpcCommand) -> Result<Option<String>> {
    match cmd {
        ipc::IpcCommand::Ping => Ok(None),
        ipc::IpcCommand::SetWallpaper { path, monitor } => {
            state.with_engine(|e| e.set_wallpaper(monitor, &path))?;
            ipc::record_active_wallpaper(monitor, path.trim());
            Ok(None)
        }
        ipc::IpcCommand::CloseWallpaper { monitor } => {
            state.with_engine(|e| e.close_wallpaper(monitor))?;
            if monitor == ipc::ALL_SCREENS {
                let mut cfg = LivelyConfig::load();
                cfg.active_wallpapers.clear();
                let _ = cfg.save();
            } else {
                ipc::clear_active_wallpaper(monitor as u32);
            }
            Ok(None)
        }
        ipc::IpcCommand::PlayPause { play } => {
            // Exactly one guard for the whole command. Acquiring the lock twice
            // would deadlock: std::sync::Mutex is not reentrant, so a second
            // `state.engine()` while the first guard is alive blocks forever.
            let playing = state.with_engine(|engine| {
                match play {
                    Some(p) => engine.set_playback_state(p),
                    None => {
                        engine.toggle_play_pause();
                    }
                }
                Ok(!engine.is_paused)
            })?;
            update_tray_title(&tray_title_for(playing, None));
            Ok(None)
        }
        ipc::IpcCommand::Volume { value } => {
            // Read and write in separate scopes: the read guard must be dropped
            // before the write guard is taken.
            state.with_engine(|e| {
                let target = parse_volume(&value, e.volume * 100.0)?;
                e.set_volume(target);
                Ok(())
            })?;
            Ok(None)
        }
        ipc::IpcCommand::Seek { monitor, value } => {
            let percent = parse_percentage(&value)?;
            state.with_engine(|e| e.seek(monitor, percent))?;
            Ok(None)
        }
        ipc::IpcCommand::SetProp {
            monitor,
            name,
            value,
        } => {
            state.with_engine(|e| e.set_property(monitor, &name, &value))?;
            Ok(None)
        }
        ipc::IpcCommand::SetLayout { mode } => {
            if !(0..=2).contains(&mode) {
                return Err(Error::Invalid(format!(
                    "layout mode must be 0, 1 or 2 (got {mode})"
                )));
            }
            let mut cfg = LivelyConfig::load();
            cfg.layout_mode = mode;
            cfg.save()?;
            Ok(None)
        }
        ipc::IpcCommand::Screenshot { monitor, file } => {
            let path = PathBuf::from(file);
            state.with_engine(|e| e.take_screenshot(monitor, path.as_path()))?;
            Ok(None)
        }
        ipc::IpcCommand::StartScreensaver { monitor } => {
            state.with_engine(|e| e.start_screensaver(monitor, screensaver_dismiss_callback))?;
            Ok(None)
        }
        ipc::IpcCommand::StopScreensaver => state.with_engine(|e| {
            e.stop_screensaver();
            Ok(None)
        }),
        ipc::IpcCommand::ShowApp => {
            ui::present();
            Ok(None)
        }
        ipc::IpcCommand::UiProbe { request } => Ok(Some(run_ui_probe(&request))),
        ipc::IpcCommand::Shutdown => {
            SHUTDOWN_REASON.store(reason::IPC, Ordering::Relaxed);
            SHUTDOWN.store(true, Ordering::Relaxed);
            Ok(None)
        }
    }
}

/// Parse a volume argument into a 0..1 level.
///
/// Accepts an absolute percentage (`50`) or a relative change (`+10`, `-5`)
/// applied to `current_pct`. Out-of-range results are rejected rather than
/// silently clamped, so a typo surfaces instead of doing something unexpected.
fn parse_volume(raw: &str, current_pct: f32) -> Result<f32> {
    let (delta, absolute) = parse_adjustment(raw)?;

    let result_pct = if absolute { delta } else { current_pct + delta };

    if !(0.0..=100.0).contains(&result_pct) {
        return Err(Error::Invalid(format!(
            "volume {result_pct:.0}% is outside the range 0-100"
        )));
    }

    Ok(result_pct / 100.0)
}

/// Parse a position argument into 0..100.
///
/// Same absolute/relative syntax as [`parse_volume`]; there is no meaningful
/// "current position" to be relative to, so `+10` means "10 percent".
fn parse_percentage(raw: &str) -> Result<f64> {
    let (delta, _absolute) = parse_adjustment(raw)?;
    // Both forms carry the same meaning here: `+10` and `10` both mean 10%.
    let delta = delta as f64;

    if !(0.0..=100.0).contains(&delta) {
        return Err(Error::Invalid(format!(
            "position {delta:.0}% is outside the range 0-100"
        )));
    }
    Ok(delta)
}

/// Shared numeric-argument parsing.
///
/// Returns `(value, is_absolute)` where `value` keeps its sign:
///
/// | input  | value | absolute |
/// |--------|-------|----------|
/// | `50`   | 50.0  | yes      |
/// | `-50`  | -50.0 | yes      |
/// | `+10`  | 10.0  | no       |
/// | `-10`  | -10.0 | no       |
///
/// The sign must survive relative parsing: `-10` means "ten down", not "ten up".
/// Range checking happens in the caller, which is why a negative value is
/// returned rather than rejected here.
fn parse_adjustment(raw: &str) -> Result<(f32, bool)> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(Error::Invalid("expected a number".into()));
    }

    let first = s.as_bytes()[0];
    let is_relative = matches!(first, b'+' | b'-');
    let digits = if is_relative { &s[1..] } else { s };

    let magnitude: f32 = digits
        .parse()
        .map_err(|_| Error::Invalid(format!("`{raw}` is not a number")))?;

    if !magnitude.is_finite() {
        return Err(Error::Invalid(format!("`{raw}` is not a finite number")));
    }

    // A leading '-' negates; '+' and bare values pass through unchanged.
    let signed = if first == b'-' { -magnitude } else { magnitude };

    Ok((signed, !is_relative))
}

// ---------------------------------------------------------------------------
// Native callbacks
// ---------------------------------------------------------------------------

/// Menu-bar commands. Always invoked on the main thread by AppKit.
///
/// Wrapped in `catch_unwind`: this is called *from* Objective-C, and a Rust
/// panic unwinding into a C frame has nowhere to go — it aborts the process at
/// best and is undefined behaviour at worst. A menu bar click must never be able
/// to take the daemon down.
extern "C" fn tray_callback(command_id: i32) {
    let outcome = match std::panic::catch_unwind(|| handle_tray_command(command_id)) {
        Ok(outcome) => outcome,
        Err(_) => Err(Error::Invalid(
            "the menu bar action failed unexpectedly".into(),
        )),
    };

    if let Err(e) = outcome {
        error!("menu bar action failed: {e}");
    }
}

fn handle_tray_command(command_id: i32) -> Result<()> {
    let Some(state) = APP_STATE.get() else {
        return Ok(());
    };

    match command_id {
        1 => {
            info!("'Open Lively' selected");
            ui::present();
            Ok(())
        }
        2 => handle_command(state, ipc::IpcCommand::PlayPause { play: None }),
        3 => {
            let engine = state.engine();
            match engine {
                Ok(mut e) => {
                    let muted = e.toggle_mute();
                    update_tray_title(if muted { "Lively (muted)" } else { "Lively" });
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        4 => {
            // Random wallpaper is not implemented; pick from the library instead
            // of pretending.
            Err(Error::Invalid(
                "random wallpaper selection is not implemented yet".into(),
            ))
        }
        99 => {
            info!("quit selected from the menu bar");
            SHUTDOWN_REASON.store(reason::TRAY_QUIT, Ordering::Relaxed);
            SHUTDOWN.store(true, Ordering::Relaxed);
            Ok(())
        }
        other => {
            log::warn!("unknown tray command id {other}");
            Ok(())
        }
    }
}

/// Any user input while the screensaver is showing dismisses it.
///
/// Called from an `NSEvent` handler in Objective-C, so the same unwinding rule
/// as `tray_callback` applies.
extern "C" fn screensaver_dismiss_callback() {
    let _ = std::panic::catch_unwind(|| {
        info!("screensaver dismissed by user input");
        SHUTDOWN_REASON.store(reason::SCREENSAVER_DISMISSED, Ordering::Relaxed);
        SHUTDOWN.store(true, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_accepts_absolute_values() {
        assert!((parse_volume("50", 80.0).unwrap() - 0.5).abs() < 1e-6);
        assert!(parse_volume("0", 80.0).unwrap().abs() < 1e-6);
        assert!((parse_volume("100", 0.0).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn volume_applies_relative_changes_to_current_level() {
        // 50% current + 10 => 60%
        let v = parse_volume("+10", 50.0).unwrap();
        assert!((v - 0.6).abs() < 1e-6, "got {v}");

        // 50% current - 20 => 30%
        let v = parse_volume("-20", 50.0).unwrap();
        assert!((v - 0.3).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn volume_ignores_current_when_absolute() {
        // An absolute value overrides whatever was playing.
        let v = parse_volume("25", 90.0).unwrap();
        assert!((v - 0.25).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn volume_rejects_out_of_range() {
        assert!(parse_volume("150", 50.0).is_err(), "absolute above 100");
        assert!(parse_volume("+500", 50.0).is_err(), "relative overflow");
        assert!(parse_volume("-500", 50.0).is_err(), "relative underflow");
    }

    #[test]
    fn negative_absolute_volume_is_rejected() {
        // "-10" is a relative decrease, not "minus ten percent"; starting from
        // 50% it lands on 40%, which is valid.
        assert!((parse_volume("-10", 50.0).unwrap() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn volume_rejects_garbage() {
        for bad in ["abc", "", "  ", "nan", "inf", "1.2.3", "+", "-"] {
            assert!(
                parse_volume(bad, 50.0).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn percentage_parses_absolute_and_relative() {
        assert_eq!(parse_percentage("50").unwrap(), 50.0);
        assert_eq!(parse_percentage("+25").unwrap(), 25.0);
        assert_eq!(parse_percentage("0").unwrap(), 0.0);
        assert_eq!(parse_percentage("100").unwrap(), 100.0);
    }

    #[test]
    fn percentage_rejects_out_of_range_and_garbage() {
        for bad in ["120", "-5", "abc", "", "nan"] {
            assert!(
                parse_percentage(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn human_bytes_is_readable() {
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(2048), "2.0KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0MB");
    }

    #[test]
    fn cli_parses_set_with_monitor() {
        let cli = Cli::try_parse_from(["lively", "set", "/tmp/a.mp4", "-m", "2"]).unwrap();
        match cli.command {
            Some(Commands::Set { path, monitor }) => {
                assert_eq!(path, "/tmp/a.mp4");
                assert_eq!(monitor, 2);
            }
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn cli_defaults_monitor_to_zero() {
        let cli = Cli::try_parse_from(["lively", "set", "/tmp/a.mp4"]).unwrap();
        match cli.command {
            Some(Commands::Set { monitor, .. }) => assert_eq!(monitor, 0),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        assert!(Cli::try_parse_from(["lively", "frobnicate"]).is_err());
    }

    #[test]
    fn cli_parses_close_all() {
        let cli = Cli::try_parse_from(["lively", "close"]).unwrap();
        match cli.command {
            Some(Commands::Close { monitor }) => assert_eq!(monitor, -1),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn commands_map_to_ipc() {
        let set = Commands::Set {
            path: "/a".into(),
            monitor: 1,
        };
        match to_ipc_command(&set) {
            ipc::IpcCommand::SetWallpaper { monitor, .. } => assert_eq!(monitor, 1),
            other => panic!("wrong: {other:?}"),
        }

        let pause = Commands::Pause;
        match to_ipc_command(&pause) {
            ipc::IpcCommand::PlayPause { play } => assert_eq!(play, Some(false)),
            other => panic!("wrong: {other:?}"),
        }
    }

    #[test]
    fn screensaver_subcommands_map_to_ipc() {
        let start = Commands::Screensaver {
            action: ScreensaverAction::Start { monitor: 3 },
        };
        match to_ipc_command(&start) {
            ipc::IpcCommand::StartScreensaver { monitor } => assert_eq!(monitor, 3),
            other => panic!("wrong: {other:?}"),
        }
    }
}
