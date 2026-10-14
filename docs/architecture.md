# Architecture

Lively is a single native binary that acts as both a wallpaper daemon and a
settings UI. This document describes how it is put together, and why.

## 1. Process model

There is exactly one long-lived process per user session. It is started on
demand by any CLI invocation, or by the app bundle at login.

```
$ lively set plasma-drift
        │  no daemon running
        ▼
   spawn `lively daemon` ──────────────┐
        │                               │
        │ connect to unix socket        │
        ▼                               │
   ┌───────────────────────────────────┴──────────────┐
   │  lively (daemon + UI, one process)               │
   │                                                  │
   │   main thread ── IPC poll ── AppKit event pump   │
   │        │                                         │
   │        ├── engine ── platform::macos ── native  │
   │        │                                          │
   │        └── SwiftUI window (same process)         │
   └──────────────────────────────────────────────────┘
```

The daemon and the window deliberately share a process. Splitting them would
require a second IPC channel just to tell the UI that the engine changed
state, and the FFI calls between Rust and Swift would still be in-process
anyway. The cost is that the window and the renderer share a main thread;
this is handled by keeping the IPC loop non-blocking (see §4).

Auto-start can be disabled with `LIVELY_NO_AUTOSTART=1`, which is used by the
test suites so they can control daemon lifetime explicitly.

## 2. Module map

| Module | Responsibility | Lines |
|---|---|---|
| `src/main.rs` | CLI parsing, daemon lifecycle, command dispatch, AppKit run loop | ~1330 |
| `src/engine.rs` | Target resolution, renderer selection, per-screen state | — |
| `src/platform/mod.rs` | `Platform` trait: the seam between engine and OS | — |
| `src/platform/macos.rs` | macOS backend (AVFoundation / WebKit / Metal) | — |
| `src/platform/linux.rs` | Linux session detection and capability gating | — |
| `src/native/lively_native.m` | Objective-C: windows, views, WebKit, Metal, status item | ~1150 |
| `src/ui/*.swift` | SwiftUI settings window and view model | ~1900 |
| `src/ui.rs` | JSON snapshot builder — the daemon side of the UI contract | — |
| `src/ipc.rs` | Unix socket transport, framed request/response | — |
| `src/library.rs` | Wallpaper library, built-in seeding, thumbnails | — |
| `src/config.rs` | Persisted settings | — |
| `src/automation.rs` | Fullscreen / power-source watchers | — |

Codebase size: ~6,700 lines Rust, ~1,900 Swift, ~1,150 Objective-C.

## 3. The platform seam

`src/platform/mod.rs` defines a single trait that the engine programs
against:

```rust
pub trait Platform {
    fn capabilities(&self) -> Result<Capabilities>;
    fn screens(&self) -> Result<Vec<ScreenInfo>>;
    fn create_view(&self, kind: SurfaceKind, target: &str) -> Result<ViewHandle>;
    fn destroy_view(&self, view: ViewHandle);
    fn set_opacity(&self, view: ViewHandle, opacity: f32);
    fn set_volume(&self, view: ViewHandle, volume: f32);
    fn set_playing(&self, view: ViewHandle, playing: bool);
    fn set_property(&self, view: ViewHandle, name: &str, value: &str);
}
```

`SurfaceKind` is the important part:

```rust
pub enum SurfaceKind {
    Video,
    WebFile,   // local HTML — loaded with read access to its own directory
    WebUrl,    // remote http(s)
    Shader,
    Image,
}
```

`WebFile` and `WebUrl` are separate variants on purpose. An earlier revision
had a single `Web` variant and the macOS backend passed `is_local_file =
true` unconditionally, so every remote URL was handed to
`loadFileURL:allowingReadAccessToURL:`. WebKit raised
`NSInvalidArgumentException`, the Objective-C exception unwound into a Rust
frame, and the process aborted — `lively set https://example.com` killed the
daemon. Splitting the variant makes the loader choice explicit at the call
site.

## 4. The main loop

The daemon owns the main thread. It is a single loop:

```rust
while !SHUTDOWN.load(Ordering::Relaxed) {
    ipc_server.poll_with(|cmd| handle_command_with_message(state, cmd));
    drain_pending_pause(state);
    unsafe { ffi::lively_pump_events(EVENT_PUMP_TIMEOUT) };  // AppKit
}
```

Both the socket and AppKit are serviced every iteration. `lively_pump_events`
blocks for at most `EVENT_PUMP_TIMEOUT`, so an idle daemon wakes roughly that
often to service UI events, and an IPC command interrupts the wait
immediately because the socket is polled every pass.

Consequence: **main-thread work must be short.** Decoding a 4K image happens
on the native side and must not block the loop for more than a frame, or the
window stops responding. This is the single biggest constraint on the current
design and the reason the renderers are implemented natively rather than as
blocking Rust calls.

## 5. The UI contract

The UI is SwiftUI. Rust owns the data.

```
  Swift                        Rust
  ────                        ────
  @_cdecl("lively_ui_install_handler")
        ────────────────────▶  saves the Swift closure pointer
                               
  lively_ui_set_handler()  ───▶  forwards the *same* closure to Swift
                               
  User clicks "Apply"
        │                      
        └──▶ lively_ui_send(json) ──▶ handle_command(json)
                                            │
                                            ▼
                                    ┌──────────────┐
                                    │  ui.rs       │
                                    │  builds JSON │
                                    │  snapshot    │
                                    └──────────────┘
```

The handler is passed **twice**, once through each side's own registration
path. This is deliberate. Both Rust and Swift had a symbol named
`lively_ui_set_handler`; whichever side registered last won, and in practice
Swift never received the handler, so the window showed an empty library and a
"not connected" banner while every data-layer test passed. `UiSelfTest` now
exercises the real `Swift → Rust → Swift` path so that class of wiring bug
fails a test instead of shipping.

`src/ui.rs` builds one JSON snapshot containing the library, screens, config,
playback state, and the currently applied wallpaper. The Swift side is a pure
function of that snapshot.

## 6. Playback polling

The window polls `playbackStatus` every 700 ms to drive the pause button and
volume slider.

That call originally returned a full state snapshot, so each poll re-read the
entire library and parsed the config from disk — twice a second, forever, for
a UI that only needed `{state, volume, position}`. The reply is now
`{ok, playback}` and nothing else.

This is the shape to keep in mind for any future poll: a UI that needs N
fields should not be able to ask for the whole world.

## 7. Error handling across FFI

`panic = "abort"` is deliberately **not** set in `Cargo.toml`. It is
tempting for a release binary, but it would remove `catch_unwind` at the FFI
boundary, and the window would be killed by any panic in a command handler
instead of returning an error to the caller.

Objective-C exceptions are a separate hazard: Rust cannot catch them, and one
unwinding into a Rust frame aborts the process with `Rust cannot catch
foreign exceptions`. Everything WebKit-facing therefore goes through:

```c
static void *lively_guarded(void *(^block)(void), void *fallback) {
    @try    { return block(); }
    @catch (NSException *e) {
        NSLog(@"[Lively] caught %@: %@", e.name, e.reason);
        return fallback;
    }
}
```

A bad wallpaper now fails one command and returns an error. The daemon stays
up. This is what fixed the remote-URL abort described in §3.

## 8. Memory ownership

The native layer owns all Objective-C objects and hands out opaque pointers:

- Objects are created with `+alloc`/`new` and bridged with `__bridge_retained`
  before the pointer is returned to Rust.
- Handles are `malloc`'d `Lively*Context` structs holding exactly one
  retained object.
- `destroy` releases the object, then frees the struct. Every destroy path is
  NULL-guarded so a double-free or a destroy-after-failure cannot crash.

Images are the reason this matters. Decoding a 2560×1440 PNG allocates a
~15 MB bitmap. A naive implementation that allocates a fresh view per
wallpaper change grows RSS without bound; the current implementation sets the
image and releases the old bitmap, which flattens RSS across repeated swaps.

## 9. Data locations

| What | Where |
|---|---|
| Config | `~/Library/Application Support/LivelyWallpaper/config.json` |
| Library | `~/Library/Application Support/LivelyWallpaper/Library/<id>/` |
| IPC socket | `$TMPDIR/livelywallpaper-<uid>/daemon.sock` |
| Login item | `~/Library/LaunchAgents/com.lively.wallpaper.plist` |
| Logs | stdout; `lively daemon` writes to the terminal or a redirect |

A stale socket is removed at daemon start; two daemons cannot run at once,
which is also why `make-app.sh --run` does not launch a second one.