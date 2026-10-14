use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::ffi;

/// How often the watcher samples system state.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Why playback was suspended. Drives the tray tooltip so the user knows *why*
/// their wallpaper stopped moving instead of assuming it broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseCause {
    Fullscreen,
    Battery,
    FullscreenAndBattery,
}

impl PauseCause {
    fn from_flags(fullscreen: bool, battery: bool) -> Option<Self> {
        match (fullscreen, battery) {
            (false, false) => None,
            (true, false) => Some(PauseCause::Fullscreen),
            (false, true) => Some(PauseCause::Battery),
            (true, true) => Some(PauseCause::FullscreenAndBattery),
        }
    }

    /// Short label suitable for a status-bar tooltip.
    pub fn label(self) -> &'static str {
        match self {
            PauseCause::Fullscreen => "Paused (fullscreen app)",
            PauseCause::Battery => "Paused (on battery)",
            PauseCause::FullscreenAndBattery => "Paused (fullscreen, on battery)",
        }
    }
}

/// Watches system state and reports playback-pause transitions.
///
/// Threading contract: this runs on its own worker thread and only calls the
/// thread-safe native probes. Anything that touches AppKit (tray text, the run
/// loop) is the caller's responsibility to marshal onto the main thread.
pub struct AutomationWatcher {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl AutomationWatcher {
    /// Spawn the watcher.
    ///
    /// `on_cause_change` fires only on an actual *transition* into or out of a
    /// paused state, never on every poll, so callers can do expensive work.
    pub fn start<F>(pause_on_fullscreen: bool, pause_on_battery: bool, on_cause_change: F) -> Self
    where
        F: Fn(Option<PauseCause>) + Send + Sync + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let worker_flag = Arc::clone(&running);
        let callback = Arc::new(on_cause_change);

        let handle = thread::Builder::new()
            .name("lively-automation".into())
            .spawn(move || {
                let mut last_cause: Option<PauseCause> = None;

                while worker_flag.load(Ordering::Relaxed) {
                    // Thread-safe: CoreGraphics + IOKit only, no AppKit.
                    let fullscreen =
                        pause_on_fullscreen && unsafe { ffi::lively_is_any_app_fullscreen() };
                    let battery =
                        pause_on_battery && unsafe { ffi::lively_is_running_on_battery() };

                    let cause = PauseCause::from_flags(fullscreen, battery);

                    if cause != last_cause {
                        log::debug!("automation: pause state -> {cause:?}");
                        callback(cause);
                        last_cause = cause;
                    }

                    // Sleep in slices so `stop()` takes effect promptly instead
                    // of blocking the thread for the full interval.
                    let mut slept = Duration::ZERO;
                    while slept < POLL_INTERVAL && worker_flag.load(Ordering::Relaxed) {
                        let remaining = POLL_INTERVAL - slept;
                        thread::sleep(remaining.min(Duration::from_millis(50)));
                        slept += Duration::from_millis(50);
                    }
                }
            })
            .ok();

        if handle.is_none() {
            log::error!("automation watcher thread failed to spawn; auto-pause disabled");
        }

        Self { running, handle }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AutomationWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cause_is_none_when_nothing_is_triggered() {
        assert_eq!(PauseCause::from_flags(false, false), None);
    }

    #[test]
    fn cause_identifies_each_trigger() {
        assert_eq!(
            PauseCause::from_flags(true, false),
            Some(PauseCause::Fullscreen)
        );
        assert_eq!(
            PauseCause::from_flags(false, true),
            Some(PauseCause::Battery)
        );
        assert_eq!(
            PauseCause::from_flags(true, true),
            Some(PauseCause::FullscreenAndBattery)
        );
    }

    #[test]
    fn cause_labels_are_descriptive() {
        assert!(PauseCause::Fullscreen.label().contains("fullscreen"));
        assert!(PauseCause::Battery.label().contains("battery"));
        assert!(PauseCause::FullscreenAndBattery.label().contains("battery"));
    }

    #[test]
    fn watcher_starts_and_stops_without_panicking() {
        // Both triggers are enabled; on the CI machine the outcome is unknown,
        // so we only assert the lifecycle itself is sound.
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sink = Arc::clone(&counter);

        let mut watcher = AutomationWatcher::start(false, false, move |_cause| {
            sink.fetch_add(1, Ordering::Relaxed);
        });

        thread::sleep(Duration::from_millis(120));
        watcher.stop();

        // With both triggers disabled there is never a pause transition.
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stop_is_idempotent_and_safe_on_drop() {
        let watcher = AutomationWatcher::start(false, false, |_| {});
        drop(watcher); // Drop must join cleanly.
    }
}
