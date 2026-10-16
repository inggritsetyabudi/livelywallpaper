# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-10-16

The first tagged release. macOS is verified end to end; the Linux backend
compiles and links but has never been run, which is why this is 0.2.0 and not
1.0.0.

### Added

- **Five renderers** — image, video, local web, remote URL, and Metal shader.
- **SwiftUI settings window** with library browser, search, metadata panel,
  per-wallpaper property controls, and multi-screen assignment.
- **Six built-in wallpapers** — Midnight Mesh, Ember, Tidal, Graphite, Drift,
  and Plasma Drift — seeded on first launch and re-seeded when the built-in
  version is bumped.
- **CLI** covering every UI operation: `set`, `close`, `play`, `pause`,
  `volume`, `seek`, `set-prop`, `screenshot`, `screens`, `list-library`,
  `add-library`, `remove-library`, `daemon`, `screensaver`, `show-app`,
  `shutdown`.
- **Daemon on demand.** Any command starts the daemon if it is not running;
  `LIVELY_NO_AUTOSTART=1` disables this.
- **Wallpaper restore at login**, backed by a LaunchAgent.
- **UI self-test** running through the real Swift → Rust → Swift path.
- **Memory soak script** (`scripts/leak-check.sh`).
- **Linux backend** — compiles and links; creates a real X11 desktop window
  and a real Wayland layer-shell surface. Never executed. See `docs/linux.md`.
- **`lively diagnostics`** — capability report for bug reports.
- **CI** for macOS 14 and an Ubuntu 24.04 feature matrix. The Linux job has
  never been observed green.

### Fixed

- **Remote URL wallpapers aborted the daemon.** The macOS backend passed
  `is_local_file = true` unconditionally, so every `https` URL was handed to
  `loadFileURL:allowingReadAccessToURL:`; WebKit raised
  `NSInvalidArgumentException`, and the Objective-C exception unwound into a
  Rust frame and killed the process. `SurfaceKind::Web` is now split into
  `WebFile` and `WebUrl` so the correct loader is chosen by construction, and
  the WebKit lifecycle is wrapped in a guard that converts any exception into
  a `NULL` return. A bad wallpaper now fails one command.
- **The settings window never received its command handler.** Both the Rust
  and Swift sides exported `lively_ui_set_handler`; the Swift registration
  lost, so the UI rendered an empty library and a "not connected" banner while
  every data-layer test passed. The handler is now installed explicitly on
  both sides, and the UI self-test covers the wiring.
- **`make-app.sh --run` started a second daemon**, which then fought over the
  socket. It now reuses the running one.
- **Playback polling re-read the entire library and config from disk every
  700 ms.** `playbackStatus` returns only `{ok, playback}`.
- **Image and video wallpapers leaked memory.** A fresh bitmap was decoded per
  wallpaper change without releasing the previous one — a 2560×1440 PNG is
  ~15 MB. Video went 128 MB → 645 MB over 400 swaps, image 411 MB → 844 MB
  over 200. Both are now flat.
- **`lively set <id>` failed with "path does not exist"** when given a library
  id, which is what the UI displays. Ids now resolve, and a title is answered
  with the matching ids.

### Security

- Objective-C exceptions crossing the FFI boundary are caught and converted to
  error returns, so a hostile or malformed wallpaper cannot terminate the
  daemon.

### Known issues

- The Screens and Settings pages have not been visually confirmed.
- The Linux backend has never been compiled. `SurfaceRecord` (Rust) and
  `LivelySurface` (C) are assumed to share a layout; this is unverified.
- Random-wallpaper rotation, the audio visualiser, and any Linux GUI are
  unimplemented.

[Unreleased]: https://github.com/your-org/livelywallpaper/compare/v0.1.0...HEAD