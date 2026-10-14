use std::path::PathBuf;

/// Result type used across the daemon.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure mode the daemon can surface to the CLI / IPC layer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no such screen: monitor {0} (attached screens: {1})")]
    UnknownScreen(u32, usize),

    #[error("path does not exist: {0}")]
    PathNotFound(PathBuf),

    #[error("unsupported wallpaper format: .{0}")]
    UnsupportedFormat(String),

    #[error("unknown wallpaper type code: {0}")]
    UnknownWallpaperType(i32),

    #[error("native layer returned NULL for {0}")]
    NativeAllocationFailed(&'static str),

    #[error("`{0}` touches AppKit and must run on the main thread")]
    NotOnMainThread(&'static str),

    #[error("engine state lock poisoned")]
    StatePoisoned,

    #[error("value contains an interior NUL byte and cannot cross the FFI boundary")]
    InteriorNul,

    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("{0}")]
    Invalid(String),
}

impl Error {
    /// Attach a path to an io error so the message tells the user *what* failed.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Convenience for bounds-checked lookups that keeps call sites readable.
    pub fn screen(index: u32, attached: usize) -> Self {
        Error::UnknownScreen(index, attached)
    }

    /// Engine state is unreachable because another thread panicked while holding it.
    pub fn poisoned() -> Self {
        Error::StatePoisoned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_screen_message_lists_attached_count() {
        let err = Error::screen(3, 2);
        let msg = err.to_string();
        assert!(msg.contains("monitor 3"), "got: {msg}");
        assert!(msg.contains("attached screens: 2"), "got: {msg}");
    }

    #[test]
    fn io_error_carries_path() {
        let err = Error::io("/tmp/x", std::io::Error::other("boom"));
        assert_eq!(err.to_string(), "i/o error at /tmp/x: boom");
    }

    #[test]
    fn unsupported_format_names_extension() {
        let err = Error::UnsupportedFormat("txt".into());
        assert!(err.to_string().contains(".txt"), "got: {err}");
    }
}
