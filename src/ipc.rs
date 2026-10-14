use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::LivelyConfig;
use crate::error::{Error, Result};

/// Upper bound on a single IPC message. Prevents a hostile/buggy client from
/// making the daemon allocate unbounded memory.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Socket read/write timeout. A client that connects and then goes silent must
/// not wedge the daemon's main loop.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Ceiling on the "is a daemon listening?" probe. Kept short: this runs on the
/// CLI's startup path, where a lag is immediately visible to the user.
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

/// Sentinel accepted by commands that apply to every screen at once.
pub const ALL_SCREENS: i32 = -1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "PascalCase")]
pub enum IpcCommand {
    /// Liveness check. The daemon acknowledges it and changes nothing.
    Ping,
    SetWallpaper {
        path: String,
        monitor: u32,
    },
    CloseWallpaper {
        monitor: i32,
    },
    PlayPause {
        play: Option<bool>,
    },
    /// `value` is a percentage string ("50") or a relative delta ("+10", "-5").
    Volume {
        value: String,
    },
    Seek {
        monitor: u32,
        value: String,
    },
    SetProp {
        monitor: u32,
        name: String,
        value: String,
    },
    SetLayout {
        mode: i32,
    },
    Screenshot {
        monitor: u32,
        file: String,
    },
    StartScreensaver {
        monitor: u32,
    },
    StopScreensaver,
    ShowApp,
    Shutdown,
    /// Forward a raw request to the settings-UI bridge.
    UiProbe {
        request: String,
    },
}

impl IpcCommand {
    /// Human-readable label, used in logs and error messages.
    pub fn name(&self) -> &'static str {
        match self {
            IpcCommand::Ping => "Ping",
            IpcCommand::SetWallpaper { .. } => "SetWallpaper",
            IpcCommand::CloseWallpaper { .. } => "CloseWallpaper",
            IpcCommand::PlayPause { .. } => "PlayPause",
            IpcCommand::Volume { .. } => "Volume",
            IpcCommand::Seek { .. } => "Seek",
            IpcCommand::SetProp { .. } => "SetProp",
            IpcCommand::SetLayout { .. } => "SetLayout",
            IpcCommand::Screenshot { .. } => "Screenshot",
            IpcCommand::StartScreensaver { .. } => "StartScreensaver",
            IpcCommand::StopScreensaver => "StopScreensaver",
            IpcCommand::ShowApp => "ShowApp",
            IpcCommand::Shutdown => "Shutdown",
            IpcCommand::UiProbe { .. } => "UiProbe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub message: String,
}

impl IpcResponse {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: msg.into(),
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: msg.into(),
        }
    }
}

/// Socket lives in the per-user runtime dir rather than `/tmp`.
///
/// `/tmp` is world-writable and shared by every user on the machine, so a
/// predictable socket name there is a hijack/spoofing vector: another local user
/// could pre-create or replace it and answer our CLI requests. A 0700 runtime
/// directory keeps us isolated.
pub fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .or_else(|_| std::env::var("TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join(format!("livelywallpaper-{}", current_uid()))
        .join("daemon.sock")
}

extern "C" {
    fn getuid() -> u32;
}

fn current_uid() -> u32 {
    // SAFETY: `getuid` is a pure syscall wrapper with no preconditions; it
    // cannot fail and touches no memory we own.
    unsafe { getuid() }
}

fn ensure_socket_parent() -> Result<PathBuf> {
    let path = socket_path();
    let parent = path
        .parent()
        .ok_or_else(|| Error::Ipc("socket path has no parent directory".into()))?;
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    Ok(path)
}

/// Send a command to a running daemon and wait for its reply.
pub fn send_command(cmd: &IpcCommand) -> Result<IpcResponse> {
    let path = ensure_socket_parent()?;
    let mut stream = UnixStream::connect(&path).map_err(|e| Error::io(&path, e))?;

    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|e| Error::io(&path, e))?;

    let mut payload = serde_json::to_string(cmd)?;
    payload.push('\n');
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(Error::Ipc(format!(
            "command too large ({} bytes)",
            payload.len()
        )));
    }
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| Error::io(&path, e))?;
    stream.flush().map_err(|e| Error::io(&path, e))?;

    let mut buf = String::new();
    stream
        .read_to_string(&mut buf)
        .map_err(|e| Error::io(&path, e))?;

    serde_json::from_str(buf.trim())
        .map_err(|e| Error::Ipc(format!("malformed response from daemon: {e}")))
}

/// Probe the daemon with a bounded round-trip.
///
/// A bare `connect()` is not enough to prove a daemon is alive: the socket file
/// outlives a crash, and on some paths a connect against a stale socket can
/// block instead of failing fast. So we send a real [`IpcCommand::Ping`] and
/// require a reply, with a hard ceiling on the whole exchange. The work runs on
/// a helper thread so even a stuck syscall cannot hang the CLI.
pub fn is_daemon_running() -> bool {
    let path = socket_path();
    if !path.exists() {
        return false;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let probe_path = path.clone();

    std::thread::Builder::new()
        .name("lively-ipc-probe".into())
        .spawn(move || {
            let alive = send_command_at(&probe_path, &IpcCommand::Ping)
                .map(|resp| resp.ok)
                .unwrap_or(false);
            let _ = tx.send(alive);
        })
        .ok();

    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(alive) => {
            if !alive {
                log::debug!("stale socket at {}; no daemon answering", path.display());
            }
            alive
        }
        Err(_) => {
            // Timed out or the helper died. Treat as "not running" and clear
            // the socket so the next launch can bind cleanly.
            log::debug!("daemon probe timed out; assuming no daemon");
            let _ = std::fs::remove_file(&path);
            false
        }
    }
}

/// Send a command to a specific socket path.
fn send_command_at(path: &std::path::Path, cmd: &IpcCommand) -> Result<IpcResponse> {
    let mut stream = UnixStream::connect(path).map_err(|e| Error::io(path, e))?;

    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|e| Error::io(path, e))?;

    let mut payload = serde_json::to_string(cmd)?;
    payload.push('\n');
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| Error::io(path, e))?;
    stream.flush().map_err(|e| Error::io(path, e))?;

    let mut buf = String::new();
    stream
        .read_to_string(&mut buf)
        .map_err(|e| Error::io(path, e))?;

    serde_json::from_str(buf.trim())
        .map_err(|e| Error::Ipc(format!("malformed response from daemon: {e}")))
}

pub struct IpcServer {
    listener: UnixListener,
}

impl IpcServer {
    /// Bind the listening socket, replacing any stale one left by a crash.
    pub fn bind() -> Result<Self> {
        let path = ensure_socket_parent()?;

        if path.exists() {
            // Only remove it if nothing is actually listening: if a live daemon
            // owns it, refuse rather than hijacking the address.
            if UnixStream::connect(&path).is_ok() {
                return Err(Error::Ipc("another daemon is already listening".into()));
            }
            log::warn!("removing stale socket at {}", path.display());
            std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        }

        let listener = UnixListener::bind(&path).map_err(|e| Error::io(&path, e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| Error::io(&path, e))?;

        restrict_socket_permissions(&path)?;

        Ok(Self { listener })
    }

    /// Accept and decode one pending command, if any. Never blocks.
    /// Accept one connection, run `handler`, and reply with its real outcome.
    ///
    /// Returns `true` when a client was served.
    ///
    /// The reply is written *after* the command runs. Acknowledging on receipt
    /// instead would report success for commands that then failed, leaving the
    /// CLI unable to tell the user what went wrong — the failure would only ever
    /// appear in the daemon's log.
    pub fn poll_with<F>(&self, handler: F) -> bool
    where
        F: FnOnce(IpcCommand) -> IpcResponse,
    {
        let Ok((mut stream, _)) = self.listener.accept() else {
            return false;
        };

        let response = match read_command(&mut stream) {
            Ok(cmd) => {
                log::debug!("ipc: received {}", cmd.name());
                handler(cmd)
            }
            Err(e) => IpcResponse::err(e.to_string()),
        };

        if let Err(e) = write_response(&mut stream, &response) {
            log::warn!("ipc: could not send reply: {e}");
        }
        true
    }
}

/// Read one command from a freshly accepted client.
fn read_command(stream: &mut UnixStream) -> Result<IpcCommand> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| Error::Ipc(e.to_string()))?;

    let payload = read_limited(stream).map_err(Error::Ipc)?;
    serde_json::from_str::<IpcCommand>(payload.trim())
        .map_err(|e| Error::Ipc(format!("invalid command: {e}")))
}

/// Read up to one newline, refusing anything larger than `MAX_MESSAGE_BYTES`.
fn read_limited(stream: &mut UnixStream) -> std::result::Result<String, String> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|e| format!("read failed: {e}"))?;
    if read == 0 {
        return Err("empty request".to_string());
    }
    if read > MAX_MESSAGE_BYTES {
        return Err(format!("request too large (max {MAX_MESSAGE_BYTES} bytes)"));
    }
    Ok(line)
}

fn write_response(stream: &mut UnixStream, resp: &IpcResponse) -> std::result::Result<(), String> {
    let mut payload = serde_json::to_string(resp).map_err(|e| e.to_string())?;
    payload.push('\n');
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

/// `chmod 0600` the socket so only the owning user can talk to the daemon.
fn restrict_socket_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| Error::io(path, e))?
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(|e| Error::io(path, e))?;
    Ok(())
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let path = socket_path();
        if let Err(e) = std::fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("failed to clean up socket {}: {e}", path.display());
            }
        }
    }
}

/// Persist the set of active wallpapers so they can be restored on next launch.
pub fn record_active_wallpaper(screen_id: u32, target: &str) {
    let mut cfg = LivelyConfig::load();
    cfg.active_wallpapers.insert(screen_id, target.to_string());
    if let Err(e) = cfg.save() {
        log::error!("could not persist active wallpaper: {e}");
    }
}

/// Forget a screen's wallpaper (used when it is closed).
pub fn clear_active_wallpaper(screen_id: u32) {
    let mut cfg = LivelyConfig::load();
    if cfg.active_wallpapers.remove(&screen_id).is_some() {
        if let Err(e) = cfg.save() {
            log::error!("could not persist wallpaper removal: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_scoped_to_uid() {
        let path = socket_path();
        assert_eq!(path.file_name().unwrap(), "daemon.sock");

        // The per-user scoping lives in the parent directory name.
        let parent = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            parent.contains(&current_uid().to_string()),
            "socket dir must be per-user: {parent}"
        );
    }

    #[test]
    fn commands_roundtrip_with_tagged_representation() {
        let cmd = IpcCommand::SetWallpaper {
            path: "/a/b.mp4".into(),
            monitor: 2,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"cmd\":\"SetWallpaper\""), "got: {json}");

        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        match back {
            IpcCommand::SetWallpaper { path, monitor } => {
                assert_eq!(path, "/a/b.mp4");
                assert_eq!(monitor, 2);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn every_variant_deserializes() {
        let cases = vec![
            r#"{"cmd":"Ping"}"#,
            r#"{"cmd":"CloseWallpaper","monitor":-1}"#,
            r#"{"cmd":"PlayPause","play":true}"#,
            r#"{"cmd":"Volume","value":"+10"}"#,
            r#"{"cmd":"Seek","monitor":0,"value":"50"}"#,
            r#"{"cmd":"SetProp","monitor":0,"name":"n","value":"v"}"#,
            r#"{"cmd":"SetLayout","mode":1}"#,
            r#"{"cmd":"Screenshot","monitor":0,"file":"/tmp/s.png"}"#,
            r#"{"cmd":"StartScreensaver","monitor":0}"#,
            r#"{"cmd":"StopScreensaver"}"#,
            r#"{"cmd":"ShowApp"}"#,
            r#"{"cmd":"Shutdown"}"#,
            r#"{"cmd":"UiProbe","request":"{\"action\":\"state\"}"}"#,
        ];
        for case in cases {
            let parsed: IpcCommand = serde_json::from_str(case)
                .unwrap_or_else(|e| panic!("failed to parse {case}: {e}"));
            assert!(!parsed.name().is_empty());
        }
    }

    #[test]
    fn unknown_command_is_rejected() {
        let parsed = serde_json::from_str::<IpcCommand>(r#"{"cmd":"DropDatabase"}"#);
        assert!(parsed.is_err(), "unknown command must not deserialize");
    }

    #[test]
    fn malformed_json_is_rejected_not_panicking() {
        assert!(serde_json::from_str::<IpcCommand>("{ not json").is_err());
        assert!(serde_json::from_str::<IpcCommand>("").is_err());
    }

    #[test]
    fn response_helpers_set_ok_flag() {
        assert!(IpcResponse::ok("fine").ok);
        assert!(!IpcResponse::err("bad").ok);
    }
}
