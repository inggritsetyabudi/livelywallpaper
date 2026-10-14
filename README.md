# Lively Wallpaper for macOS

Animated desktop wallpapers: video, web pages, images, and Metal shaders —
per screen, with a menu-bar app and a CLI.

Native macOS wallpaper engines (AVFoundation, WKWebView, NSImage, CAMetalLayer)
behind a Rust control plane, with a SwiftUI settings app and an Objective-C
bridge — all compiled into **one binary**.

## Built-in wallpapers

Six wallpapers ship inside the binary, so the library is never empty on first
launch. They cover three renderers:

| Wallpaper | Type | Renderer |
|---|---|---|
| Midnight Mesh | 5 | Image — deep indigo/violet mesh gradient |
| Ember | 5 | Image — warm sunset counterpart |
| Tidal | 5 | Image — light filtering through water |
| Graphite | 5 | Image — near-monochrome |
| Drift | 1 | Web — three CSS colour fields on offset loops |
| Plasma Drift | 6 | Metal shader — domain-warped noise at 60fps |

Each carries full metadata (title, author, description, licence, thumbnail) and
a `LivelyProperties.json`, so selecting one in the window shows its properties as
real controls and offers **Apply Wallpaper** for any screen.

Built-ins cannot be deleted — they are reinstalled on the next launch, so
offering the option would only mislead.

Everything under `resources/wallpapers/` is generated, not hand-authored:

```sh
./scripts/make-wallpapers.py      # render the PNG wallpapers + thumbnails
./scripts/render-shader.swift \  # compile a Metal shader and render its thumbnail
  resources/wallpapers/plasma-drift/shader.metal \
  resources/wallpapers/plasma-drift/thumb.png 640 360
```

`build.rs` embeds the whole folder with `include_bytes!`, so the executable
stays self-contained and a fresh install has content immediately.

The shader renderer also *validates* as it generates: `render-shader.swift`
compiles the Metal source and fails loudly if it does not build, so a broken
shader is caught here rather than at runtime — where the native layer would
silently fall back to a default.

## Status

Working end to end: a SwiftUI settings window, multi-screen wallpaper mounting, a
menu-bar app, a CLI over a Unix socket, a wallpaper library with per-wallpaper
property controls, and a screensaver mode.

The settings window has:

- a **Featured** section with the wallpapers that ship with the app, and a
  separate **Your Wallpapers** section for imports
- a details pane showing a large preview and the wallpaper's metadata — title,
  author, description, licence, format — plus a screen picker and an
  **Apply Wallpaper** button
- generated controls for `LivelyProperties.json` — sliders, checkboxes,
  dropdowns and colour values are rendered from the file, with no per-wallpaper
  code
- transport controls (play/pause, mute, volume) that always reflect the running
  daemon
- a sidebar listing every screen with the wallpaper currently applied to it

Not built yet, and reported as errors rather than silently no-op'd:

- Random / "next" wallpaper selection (menu bar item 4)
- Audio-spectrum visualiser (`audio_visualizer_enabled` is persisted but unused)
- Live wallpaper preview inside the window (previews show thumbnails)
- Window level / click-through options (`lively_set_window_level`,
  `lively_set_window_interactive` are bound but unused)

## Platforms

| | Wallpapers | Video | Web | Image | GUI |
|---|---|---|---|---|---|
| **macOS** | yes | AVFoundation | WKWebView | NSImage | SwiftUI |
| **X11** | yes | needs GStreamer | needs WebKitGTK | Cairo | CLI only |
| **Wayland (layer-shell)** | yes | needs GStreamer | needs WebKitGTK | Cairo | CLI only |
| **GNOME / Mutter** | **no** — see [`docs/linux.md`](docs/linux.md) | — | — | — | — |

Wayland has no "window beneath other windows" in the base protocol;
`zwlr_layer_shell_v1` is the closest thing and covers Sway, Hyprland, river,
niri, wayfire and KDE Plasma 6. GNOME implements neither, so it is not
supported.

**The Linux build has never been executed** — it was written on a Mac with no
Docker or cross-toolchain available. CI
(`.github/workflows/build.yml`) is the real test; treat it as unverified until
that matrix is green.

## Build

```sh
./scripts/make-app.sh          # builds and assembles dist/Lively.app
./scripts/make-app.sh --run    # ...and launches it
```

Then double-click `dist/Lively.app`, or open it from a terminal:

```sh
cargo build --release    # target/release/livelywallpaper
cargo test               # 100 unit tests, no network or UI needed
cargo clippy --all-targets -- -D warnings
```

On Linux the media renderers are opt-in, so a bare `cargo build` succeeds
without them:

```sh
cargo build --release --features linux-video   # GStreamer video wallpapers
cargo build --release --features linux-web     # WebKitGTK web wallpapers
```

The app icon is generated too, so it stays crisp and tweakable rather than being
an opaque binary:

```sh
./scripts/make-icon.swift        # -> assets/AppIcon.icns
```

Requires Xcode command line tools (`xcode-select --install`). `build.rs`
compiles all three halves — Objective-C engines, SwiftUI app, Rust control plane
— and links them into one executable. The Swift half needs `libclang_rt.osx.a`
for back-deployment; `build.rs` locates it automatically.

Launched from Finder with no arguments, the app starts the daemon and opens the
window. From a terminal with no arguments it prints help.

## Usage

Start the daemon, then drive it:

```sh
lively daemon &                 # foreground; use a login item in production
lively set ~/Media/clip.mp4     # video wallpaper on screen 0
lively set ./MyLivelyProject    # a folder with LivelyInfo.json
lively set https://example.com  # web wallpaper
lively screens                  # list displays
```

Any command that needs the daemon will report a clear error if none is running,
rather than silently starting one.

### Commands

| Command | Purpose |
|---|---|
| `set <path> [-m N]` | Mount a media file, project folder, or URL on screen N |
| `close [-m N]` | Remove a wallpaper (`-m -1` for all screens) |
| `play` / `pause` | Resume or pause playback |
| `volume <v>` | `50`, or a relative `+10` / `-5` |
| `seek <v> [-m N]` | Jump to a position in the video |
| `set-prop <name> <value>` | Send a property to a web wallpaper |
| `screenshot -f <file>` | Capture a screen to `.png` / `.jpg` |
| `screensaver start\|stop` | Fullscreen until dismissed |
| `add-library` / `list-library` / `remove-library` | Manage the library |
| `daemon [--screensaver]` | Run the daemon in the foreground |
| (double-click the app) | Start the daemon and open the settings window |
| `shutdown` | Stop the daemon |

Set `LIVELY_LOG=1` for debug logging.

## Architecture

```
src/
  main.rs        CLI parsing, daemon lifecycle, command dispatch, run loop
  ui.rs          settings-UI bridge: wire types, command dispatch, snapshots
  ui/
    AppModel.swift    SwiftUI view model + C bridge
    ContentView.swift the window itself
  engine.rs      one window per screen; target resolution + renderer routing
  state.rs       Arc<Mutex<EngineManager>> shared with native callbacks
  ipc.rs         Unix-socket protocol, client and server
  automation.rs  fullscreen / battery watcher
  library.rs     on-disk wallpaper collection
  package.rs     LivelyInfo.json manifest parsing
  config.rs      persisted settings
  error.rs       one error enum, one Result alias
  ffi.rs         extern declarations
  native/        Objective-C wallpaper engines
build.rs         compiles ObjC + Swift, links everything
scripts/
  make-app.sh          assemble dist/Lively.app
  make-icon.swift      generate assets/AppIcon.icns
  make-wallpapers.py   render the built-in PNG wallpapers
  render-shader.swift  compile a Metal shader and render its thumbnail
```

**One binary, two roles.** With a daemon running, a CLI invocation is a client
that forwards over IPC and exits. Without one, it either runs as a daemon
(`daemon`, `screensaver start`, or double-clicking the app) or explains that it
needs one.

## The settings UI

Rust owns all state. The SwiftUI layer is a pure view:

```
Rust --present()-->   Swift  "show the window"
Rust --refresh()-->   Swift  "re-read state and redraw"
Swift --handler(json)->> Rust  "the user acted; execute and reply with state"
```

Swift never reads the config, the library, or the engine. Every user action is
a command to Rust, and what the UI renders is always the snapshot Rust returns
after executing it. That is what prevents the classic split-brain bug where a
toggle reads "on" while the wallpaper is paused — the value the window shows is
by construction the value the daemon is running.

Two rules fall out of this:

- The snapshot reports **live** transport state from the engine, not from the
  config file. Reading the file would show the value from the last save.
- Actions that change persistent settings rebuild the snapshot *after* the
  change. Returning the pre-action snapshot would make a successful apply look
  like it did nothing.

The property panel is generated from `LivelyProperties.json`: each `type`
(`slider`, `checkbox`, `dropdown`, `color`) becomes a matching SwiftUI control.
Adding support for a new wallpaper property needs no UI code.

**Logic is separated from FFI.** Which renderer handles which file, and how
`LivelyInfo` type codes map to renderers, are pure functions with no AppKit
involvement — so the parts most likely to regress are tested on any machine.
Only `EngineManager::create_view` crosses into native code.

## Memory and stability

Wallpaper switching is the operation that repeats for the life of the process,
so it is where resource handling was audited. `./scripts/leak-check.sh` runs the
measurement: it applies one wallpaper type repeatedly against a live daemon and
reports both RSS and the `leaks` byte count.

Both numbers matter, and they catch different faults. A lost pointer shows up in
`leaks`; an **over-retain** does not, because the object is still reachable — it
shows up only as RSS climbing.

Two rules fall out of that:

- **Reuse the expensive view; do not rebuild it.** Rebuilding an `AVPlayer` /
  `AVPlayerLooper` / `AVPlayerLayer` per wallpaper change leaked ~1.3 MB every
  time with no plateau; reloading a 2560x1440 PNG cost ~15 MB of decoded bitmap
  per switch. Both are now swapped in place via `lively_video_set_source` and
  `lively_image_set_source`. Measured over 200 switches:

  | Path | Before | After |
  |---|---|---|
  | Video, 200 switches | 128 MB → 645 MB | 60 MB → 56 MB |
  | Image, 200 switches | 411 MB → 844 MB | flat at 82 MB |

- **`CFRelease(NULL)` traps and a Rust `catch_unwind` cannot catch an
  Objective-C exception.** Both crash the process outright — the latter aborts
  with *"Rust cannot catch foreign exceptions"*. Every destroy path null-checks
  its handles, and the one API documented to throw on a precondition
  (`-[AVQueuePlayer insertItem:afterItem:]`) is guarded by
  `canInsertItem:afterItem:`. Native callbacks that Rust cannot intercept are
  wrapped in `catch_unwind`.

Residual, measured rather than assumed: Metal shaders retain ~12 KB per
switch — the `MTKView` renderer is not reused. At a few hundred kilobytes per
day of wallpaper switching this is not urgent, but it is not zero either, and it
is the obvious next thing to fix. Web views plateau around 80 MB and are clean
under `leaks`.

## Threading

AppKit is main-thread-only, and this daemon has three sources of work:
the main loop, native callbacks, and a background automation watcher.

- The main loop polls IPC, applies pending watcher state, then pumps AppKit for
  50 ms. It never holds the engine lock across `lively_pump_events`.
- Native callbacks (tray menu, screensaver dismissal) receive no captured state,
  so they read `APP_STATE: OnceLock<AppState>` — an `Arc<Mutex<EngineManager>>`,
  never a raw pointer plus `unsafe impl Sync`.
- The watcher thread only calls the two thread-safe native probes
  (`lively_is_any_app_fullscreen`, `lively_is_running_on_battery`), both of which
  deliberately avoid AppKit. It records the result; the main loop applies it.

`AppState::with_engine` takes a closure rather than handing out a guard. A raw
guard is easy to hold across a second `state.engine()` call, and `std::sync::Mutex`
is not reentrant — that self-deadlocks the daemon silently. The closure form
makes nested acquisition impossible to express.

`ffi::ensure_main_thread` converts "you called AppKit off-thread" from undefined
behaviour into a typed error.

## Durability and safety

- **Transport state is restored on launch** and persisted on every change, so a crash does not lose the user's volume or mute choice.
- **Config writes are atomic**: temp file → `fsync` → `rename`. A crash mid-write
  cannot truncate the config. A corrupt config is moved aside as
  `config.json.corrupt` and defaults are used, because refusing to boot is worse
  than losing preferences.
- **The socket is not in `/tmp`.** `/tmp` is shared and world-writable, so a
  predictable socket name there is a hijacking vector. The socket lives in
  `$XDG_RUNTIME_DIR/livelywallpaper-<uid>/` and is `chmod 0600`.
- **Liveness is a round-trip, not a `connect()`.** A socket file outlives a crash.
  `is_daemon_running` sends a `Ping` and requires a reply under a 750 ms ceiling,
  on a helper thread, and clears the socket if it times out.
- **Replies reflect what happened.** The server writes its response *after* the
  command runs, so the CLI reports real failures rather than always printing `ok`.
- **Library imports are sandboxed.** Filenames are reduced to a single safe path
  component (`[A-Za-z0-9._-]`, no separators, no leading dots, length-bounded),
  symlinks are skipped while copying, and `delete` refuses paths outside the
  library root.
- **IPC is bounded**: 64 KiB max message, 5 s read/write timeouts.

## Configuration

`~/Library/Application Support/LivelyWallpaper/config.json`

```json
{
  "version": 1,
  "volume": 0.8,
  "is_muted": false,
  "is_paused": false,
  "pause_on_fullscreen": true,
  "pause_on_battery": true,
  "layout_mode": 0,
  "active_wallpapers": {},
  "audio_visualizer_enabled": true,
  "interactive_mouse": false
}
```

Missing keys fall back to defaults, so configs written by older builds keep
loading. Wallpapers in `active_wallpapers` are re-mounted at startup; any that
fail are dropped rather than retried forever.

Wallpapers live in `.../LivelyWallpaper/Library/`, one Lively project folder each.

## Wallpaper formats

| Input | Renderer |
|---|---|
| `mp4 mkv mov webm avi m4v mpg mpeg` | AVFoundation video |
| `html htm` | WKWebView, local file |
| `http(s)://…` | WKWebView, remote |
| `gif jpg jpeg png webp heic heif bmp tiff` | NSImageView |
| `metal shader fragment` | CAMetalLayer |
| folder with `LivelyInfo.json` | decided by its `Type` field |

Lively `Type` codes: `0` video, `1`/`2` web, `4`/`5` image, `6` shader.
`3` (App) is rejected with a clear message — it needs an executable host that is
not implemented.