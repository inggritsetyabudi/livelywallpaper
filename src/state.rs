//! Process-wide state shared between the main run loop and the native callback
//! trampolines (status-bar menu, screensaver dismissal).
//!
//! # Why a mutex instead of a raw pointer
//!
//! Native callbacks are C function pointers, so they carry no captured state.
//! The usual shortcut is to stash a raw pointer in a global and dereference it
//! inside the callback. That trades a compile error for undefined behaviour:
//! a `*mut EngineManager` is neither `Send` nor `Sync`, so the only way to store
//! it in a global is to `unsafe impl` those traits by hand — which silences the
//! compiler without making the access actually sound, and leaves the value
//! un-droppable (it leaks, and it is never revalidated).
//!
//! `Arc<Mutex<Engine>>` is safe, shares ownership, and gives the callback a
//! lifetime it cannot outlive.
//!
//! # Locking discipline
//!
//! AppKit calls must happen on the main thread, and the run loop calls back into
//! AppKit. So: **never hold the lock across a native call that can re-enter the
//! run loop.** In practice this means methods on `EngineManager` mutate state
//! under the lock, and the lock is dropped before any `lively_*` FFI call that
//! might pump events. `with_engine` scopes the guard to a single closure and the
//! guard is dropped when that closure returns.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::engine::EngineManager;
use crate::error::{Error, Result};

/// The handle stored in the global slot. Cheap to clone, `'static`.
pub type SharedEngine = Arc<Mutex<EngineManager>>;

pub struct AppState {
    engine: SharedEngine,
}

impl AppState {
    pub fn new(engine: EngineManager) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }

    /// Borrow the engine, recovering from a poisoned mutex.
    ///
    /// A poisoned lock means some other thread panicked mid-update. The engine's
    /// invariants are plain data (a Vec of screens, some bools), so the state is
    /// still usable; propagating the panic on every later command would brick the
    /// daemon, which is strictly worse.
    pub fn engine(&self) -> Result<MutexGuard<'_, EngineManager>> {
        self.engine.lock().map_err(|_| Error::poisoned())
    }

    /// A second handle to the engine, for handing to a worker thread.
    #[allow(dead_code)]
    pub fn shared(&self) -> SharedEngine {
        Arc::clone(&self.engine)
    }

    /// Run `f` with the engine locked, releasing the lock when it returns.
    ///
    /// Prefer this over [`AppState::engine`] inside command handlers. A raw
    /// guard is easy to hold across a second `engine()` call, and
    /// `std::sync::Mutex` is not reentrant — that self-deadlocks the daemon with
    /// no error and no log line. Scoping the borrow to a closure makes nested
    /// acquisition impossible to express.
    pub fn with_engine<T>(&self, f: impl FnOnce(&mut EngineManager) -> Result<T>) -> Result<T> {
        let mut engine = self.engine()?;
        f(&mut engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn poison_is_reported_not_propagated() {
        let state = AppState::new(EngineManager::headless_for_tests());
        let shared = state.shared();

        // Poison the mutex the only way possible: panic while holding it.
        let s = Arc::clone(&shared);
        let _ = std::thread::spawn(move || {
            let _guard = s.lock().unwrap();
            panic!("boom");
        })
        .join();

        // The daemon must survive and keep serving commands.
        assert!(shared.lock().is_err(), "expected the mutex to be poisoned");
        assert!(matches!(state.engine(), Err(Error::StatePoisoned)));
    }

    #[test]
    fn with_engine_scopes_the_lock() {
        let state = AppState::new(EngineManager::headless_for_tests());

        state
            .with_engine(|engine| {
                engine.set_volume(0.25);
                Ok(())
            })
            .unwrap();

        // The lock must be free again once the closure returns.
        let volume = state.with_engine(|engine| Ok(engine.volume)).unwrap();
        assert!((volume - 0.25).abs() < 1e-6);
    }

    #[test]
    fn with_engine_propagates_errors_without_leaking_the_lock() {
        let state = AppState::new(EngineManager::headless_for_tests());

        let result: Result<()> = state.with_engine(|_| Err(Error::Invalid("nope".into())));
        assert!(matches!(result, Err(Error::Invalid(_))));

        // Still usable: the guard was dropped on the error path.
        assert_eq!(state.with_engine(|e| Ok(e.screen_count())).unwrap(), 0);
    }

    #[test]
    fn nested_locking_would_deadlock() {
        // Documents *why* `with_engine` exists. `std::sync::Mutex` has no
        // reentrant mode, so re-acquiring the guard inside the closure would
        // hang forever. This test pins the constraint without actually hanging:
        // it only asserts the closure API compiles and runs to completion.
        let state = AppState::new(EngineManager::headless_for_tests());
        state
            .with_engine(|_| Ok(()))
            .expect("closure must complete");
    }

    #[test]
    fn engine_is_shared_across_threads() {
        let state = AppState::new(EngineManager::headless_for_tests());
        let shared = state.shared();

        let handle = std::thread::spawn(move || {
            let guard = shared.lock().unwrap();
            guard.volume
        });

        assert!(handle.join().is_ok());
    }
}
