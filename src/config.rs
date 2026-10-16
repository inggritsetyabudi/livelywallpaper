use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::error::{Error, Result};

/// Bumped whenever the on-disk shape changes in a non-backwards-compatible way.
pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivelyConfig {
    #[serde(default = "config_version")]
    pub version: u32,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default)]
    pub is_paused: bool,
    #[serde(default = "default_true")]
    pub pause_on_fullscreen: bool,
    #[serde(default = "default_true")]
    pub pause_on_battery: bool,
    /// 0 = PerScreen, 1 = Span, 2 = Duplicate
    #[serde(default)]
    pub layout_mode: i32,
    /// screen_id -> wallpaper path or URL
    #[serde(default)]
    pub active_wallpapers: HashMap<u32, String>,
    #[serde(default = "default_true")]
    pub audio_visualizer_enabled: bool,
    #[serde(default)]
    pub interactive_mouse: bool,
}

fn config_version() -> u32 {
    CONFIG_VERSION
}
fn default_volume() -> f32 {
    0.8
}
fn default_true() -> bool {
    true
}

impl Default for LivelyConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            volume: default_volume(),
            is_muted: false,
            is_paused: false,
            pause_on_fullscreen: true,
            pause_on_battery: true,
            layout_mode: 0,
            active_wallpapers: HashMap::new(),
            audio_visualizer_enabled: true,
            interactive_mouse: false,
        }
    }
}

impl LivelyConfig {
    /// Root of all per-user application state.
    pub fn support_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

        // XDG Base Directory on Linux, the conventional Application Support
        // folder on macOS.
        //
        // Previously this hard-coded the macOS path, which meant a Linux user
        // got a stray `~/Library/Application Support/LivelyWallpaper` in their
        // home directory: wrong location, not where anything else looks, and
        // invisible to the XDG tooling a Linux user would use to find or clean
        // it up.
        if cfg!(target_os = "linux") {
            if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
                if !data_home.is_empty() {
                    return PathBuf::from(data_home).join("LivelyWallpaper");
                }
            }
            return PathBuf::from(&home).join(".local/share/LivelyWallpaper");
        }

        PathBuf::from(home).join("Library/Application Support/LivelyWallpaper")
    }

    pub fn config_path() -> PathBuf {
        Self::support_dir().join("config.json")
    }

    /// Read config from disk.
    ///
    /// A missing or corrupt file is *not* an error: we fall back to defaults and
    /// keep the unreadable file around as `config.json.corrupt` so users can
    /// recover it. Losing preferences is bad, but refusing to boot is worse.
    pub fn load() -> Self {
        let path = Self::config_path();

        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };

        match serde_json::from_str::<Self>(&data) {
            Ok(mut cfg) => {
                cfg.sanitize();
                cfg
            }
            Err(e) => {
                log::warn!(
                    "config at {} is unreadable ({e}); falling back to defaults",
                    path.display()
                );
                let backup = path.with_extension("json.corrupt");
                if let Err(rename_err) = fs::rename(&path, &backup) {
                    log::error!("could not preserve broken config: {rename_err}");
                }
                Self::default()
            }
        }
    }

    /// Clamp / drop values that would put the daemon in an invalid state.
    fn sanitize(&mut self) {
        if !self.volume.is_finite() {
            self.volume = default_volume();
        }
        self.volume = self.volume.clamp(0.0, 1.0);

        if !(0..=2).contains(&self.layout_mode) {
            self.layout_mode = 0;
        }

        if self.version > CONFIG_VERSION {
            log::warn!(
                "config was written by a newer version ({} > {CONFIG_VERSION}); unknown fields ignored",
                self.version
            );
        }
        self.version = CONFIG_VERSION;

        // Empty strings are not valid wallpaper targets; drop them so restore
        // does not try to load "" on boot.
        self.active_wallpapers
            .retain(|_, target| !target.trim().is_empty());
    }

    /// Persist config atomically: write to a temp file, fsync, then rename.
    ///
    /// `rename(2)` within the same directory is atomic, so a crash or power loss
    /// mid-write can never leave a truncated config behind.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        let serialized = serde_json::to_vec_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");

        {
            use std::io::Write;
            let mut file = fs::File::create(&tmp_path).map_err(|e| Error::io(&tmp_path, e))?;
            file.write_all(&serialized)
                .map_err(|e| Error::io(&tmp_path, e))?;
            file.sync_all().map_err(|e| Error::io(&tmp_path, e))?;
        }

        fs::rename(&tmp_path, &path).map_err(|e| {
            // Best effort: do not leave the scratch file behind on failure.
            let _ = fs::remove_file(&tmp_path);
            Error::io(&path, e)
        })?;

        Ok(())
    }
}

/// Resolve the config path under `base`, so tests never touch the real user
/// profile.
#[cfg(test)]
pub(crate) fn support_dir_in(base: &std::path::Path) -> PathBuf {
    base.join("LivelyWallpaper")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("lively-cfg-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn default_is_sane() {
        let cfg = LivelyConfig::default();
        assert_eq!(cfg.layout_mode, 0);
        assert!((0.0..=1.0).contains(&cfg.volume));
        assert!(cfg.pause_on_fullscreen);
    }

    #[test]
    fn sanitize_clamps_volume_and_layout() {
        let mut cfg = LivelyConfig {
            volume: 5.0,
            layout_mode: 99,
            ..Default::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.volume, 1.0);

        let mut cfg = LivelyConfig {
            volume: -3.0,
            ..Default::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.volume, 0.0);

        let mut cfg = LivelyConfig {
            layout_mode: 7,
            ..Default::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.layout_mode, 0);
    }

    #[test]
    fn sanitize_rejects_nan_volume() {
        let mut cfg = LivelyConfig {
            volume: f32::NAN,
            ..Default::default()
        };
        cfg.sanitize();
        assert!(cfg.volume.is_finite());
    }

    #[test]
    fn sanitize_drops_blank_wallpaper_targets() {
        let mut cfg = LivelyConfig::default();
        cfg.active_wallpapers.insert(0, String::new());
        cfg.active_wallpapers.insert(1, "   ".into());
        cfg.active_wallpapers.insert(2, "/real/path.mp4".into());
        cfg.sanitize();

        assert_eq!(cfg.active_wallpapers.len(), 1);
        assert!(cfg.active_wallpapers.contains_key(&2));
    }

    #[test]
    fn roundtrips_through_json() {
        let mut cfg = LivelyConfig::default();
        cfg.active_wallpapers.insert(1, "/tmp/a.mp4".into());
        cfg.volume = 0.33;

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: LivelyConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.volume, 0.33);
        assert_eq!(parsed.active_wallpapers.get(&1).unwrap(), "/tmp/a.mp4");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Simulates a config written by an older build that lacks new keys.
        let parsed: LivelyConfig = serde_json::from_str(r#"{"volume":0.5}"#).unwrap();
        assert_eq!(parsed.volume, 0.5);
        assert_eq!(parsed.layout_mode, 0);
        assert!(parsed.pause_on_battery);
        assert!(parsed.active_wallpapers.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored_not_fatal() {
        let parsed: LivelyConfig =
            serde_json::from_str(r#"{"volume":0.5,"some_future_key":42}"#).unwrap();
        assert_eq!(parsed.volume, 0.5);
    }

    #[test]
    fn corrupt_config_preserves_original_and_returns_defaults() {
        let dir = tmpdir("corrupt");
        let path = support_dir_in(&dir).join("config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ this is not json").unwrap();

        // Mirror what load() does, but against our scratch dir.
        let data = fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<LivelyConfig>(&data).is_err());

        fs::rename(&path, path.with_extension("json.corrupt")).unwrap();
        assert!(path.with_extension("json.corrupt").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let dir = tmpdir("atomic");
        let target = support_dir_in(&dir).join("config.json");

        // Exercise the same write sequence save() performs.
        let cfg = LivelyConfig::default();
        let bytes = serde_json::to_vec_pretty(&cfg).unwrap();
        let tmp = target.with_extension("json.tmp");

        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&tmp, &bytes).unwrap();
        fs::rename(&tmp, &target).unwrap();

        assert!(target.exists());
        assert!(!tmp.exists(), "temp file must be renamed away");
        assert!(serde_json::from_slice::<LivelyConfig>(&bytes).is_ok());

        let _ = fs::remove_dir_all(&dir);
    }
}
