# Lively Wallpaper

A native live wallpaper engine for macOS, with a SwiftUI settings window and a
scriptable CLI. Image, video, web, and Metal shader wallpapers, with a library
of six built-ins that ship with the app.

> **Status: macOS is verified. Linux builds but has never been run.**
> The Linux backend compiles and links, but no wallpaper has been shown on a
> real X11 or Wayland session — see [docs/linux.md](docs/linux.md). Nothing in
> this README should be read as claiming otherwise.

---

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Build from source](#build-from-source)
- [Install](#install)
- [Verify the build](#verify-the-build)
- [Usage](#usage)
- [Make a wallpaper](#make-a-wallpaper)
- [How it works](#how-it-works)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [Documentation](#documentation)
- [Licence](#licence)

---

## Features

- **Five renderers** — static image, looping video, local HTML, remote URL, and
  GPU Metal shader.
- **Six built-in wallpapers**, seeded on first launch and upgradeable in place.
- **SwiftUI settings window** — library browser, search, per-wallpaper
  properties, multi-screen assignment.
- **Scriptable CLI** — every operation the UI performs is a command.
- **Daemon on demand** — it starts when you need it and restores your
  wallpaper at login.
- **~6,700 lines Rust, ~1,900 Swift, ~1,150 Objective-C.** No third-party
  crates beyond `clap`, `serde`, `thiserror`, and `log`.

---

## Requirements

### macOS

| | Minimum | Notes |
|---|---|---|
| macOS | **14.0 Sonoma** | Declared as `LSMinimumSystemVersion` in the bundle |
| Architecture | Apple silicon or Intel | Builds natively for your host |
| Disk | ~1.5 GB | Rust `target/` after a full release build |

The macOS floor is set by the bundle rather than derived from API usage — the
APIs in play (`AVPlayerLayer`, `MTKView`, `NSHostingController`) are all much
older, so this is a deliberate choice rather than a discovered limit. If you
lower it in `scripts/make-app.sh`, expect to verify on that older release
yourself.

### Toolchain

All three are required — the Rust binary calls into Objective-C and Swift, so
this is not a pure-Rust project.

| Tool | How to get it | Verified against |
|---|---|---|
| Rust (stable) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` | 1.96.0 |
| Xcode Command Line Tools | `xcode-select --install` | Swift 6.4, Apple clang 21 |
| `codesign` | ships with the CLT | present on any Mac |

Check what you have:

```console
$ rustc --version
$ swiftc --version
$ xcode-select -p
```

`xcodebuild` is **not** required — only the Command Line Tools. Full Xcode is
fine if you have it; nothing in the build needs it.

---

## Build from source

```console
$ git clone https://github.com/inggritsetyabudi/livelywallpaper.git
$ cd livelywallpaper
$ ./scripts/make-app.sh
```

That single command does everything:

1. `cargo build --release` — the Rust binary.
2. Compiles `src/ui/*.swift` and links it against the binary via `build.rs`.
3. Compiles `src/native/lively_native.m` through `cc`.
4. Generates the app icon.
5. Assembles `dist/Lively.app` with a valid `Info.plist`.
6. Signs the bundle ad-hoc.

Output:

```
==> Building release binary
==> Assembling dist/Lively.app
==> Signing (ad-hoc)
==> Built dist/Lively.app
```

To build **and** launch in one step, stopping any previous copy first:

```console
$ ./scripts/make-app.sh --run
```

### Building only the binary

If you want the CLI without the app bundle:

```console
$ cargo build --release
$ ./target/release/livelywallpaper --help
```

A bare executable works for the CLI. It cannot own a menu bar item or be
launched from Finder, because macOS requires the bundle layout for that —
which is exactly what `make-app.sh` creates.

### Build profiles

```console
$ cargo build              # debug, fast
$ cargo build --release    # optimised
$ cargo test               # build and run all 100 tests
```

### Building for Linux

Linux media support sits behind Cargo features so the project still builds on
a machine without the media stack:

```console
$ sudo apt-get install -y \
    libxcb1-dev libcairo2-dev libegl1-mesa-dev libgl1-mesa-dev \
    libwayland-dev wayland-protocols \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libwebkit2gtk-4.1-dev

$ cargo build --release --features linux-video,linux-web
```

| Feature | Default | Pulls in |
|---|---|---|
| `linux-wayland` | yes | XCB, Cairo, EGL, layer-shell |
| `linux-video` | no | GStreamer 1.x |
| `linux-web` | no | WebKitGTK 4.1 |

The resulting Linux build is experimental and unverified — read
[docs/linux.md](docs/linux.md) before using it.

---

## Install

### Run from the build directory

Nothing to install:

```console
$ ./dist/Lively.app/Contents/MacOS/Lively
```

### Install to /Applications

```console
$ cp -R dist/Lively.app /Applications/
$ open /Applications/Lively.app
```

### Install to your PATH (CLI only)

```console
$ sudo install -m 0755 target/release/livelywallpaper /usr/local/bin/lively
$ lively --help
```

### Start at login

The app does **not** register a login item by itself. To have your wallpaper
restored each session, install the LaunchAgent:

```console
$ mkdir -p ~/Library/LaunchAgents
$ cp scripts/com.lively.wallpaper.plist ~/Library/LaunchAgents/
$ launchctl load ~/Library/LaunchAgents/com.lively.wallpaper.plist
```

It starts `Lively daemon` at login, which restores the wallpapers recorded in
your config.

The plist points at `/Applications/Lively.app/Contents/MacOS/Lively`. If you
keep the app somewhere else, edit `ProgramArguments` to the real path first.

To remove it:

```console
$ launchctl unload ~/Library/LaunchAgents/com.lively.wallpaper.plist
$ rm ~/Library/LaunchAgents/com.lively.wallpaper.plist
```

### Uninstall

```console
$ lively shutdown                       # stop the daemon
$ rm -rf ~/Library/Application\ Support/LivelyWallpaper   # config + library
$ rm -rf /Applications/Lively.app
```

Your wallpapers live in `~/Library/Application Support/LivelyWallpaper/`.
Deleting that directory removes them permanently, so back it up first if you
have added your own.

### Where things are stored

| What | Where |
|---|---|
| Config | `~/Library/Application Support/LivelyWallpaper/config.json` |
| Library | `~/Library/Application Support/LivelyWallpaper/Library/<id>/` |
| IPC socket | `$TMPDIR/livelywallpaper-<uid>/daemon.sock` |
| Login item | `~/Library/LaunchAgents/com.lively.wallpaper.plist` |

---

## Verify the build

Do not skip this — it is quick, and it catches a wiring failure that looks
perfectly healthy from the command line.

```console
$ cargo fmt --all -- --check              # formatting
$ cargo clippy --all-targets -- -D warnings   # lints
$ cargo test                              # 100 tests
$ LIVELY_UI_SELFTEST=1 ./dist/Lively.app/Contents/MacOS/Lively
```

The last one should print:

```
ALL 28 CHECKS PASSED
```

This test runs through the **real** Swift → Rust → Swift path on purpose.
A test that calls the Rust side directly would pass even if the Swift wiring
were completely dead — which is exactly what shipped once, leaving the
window rendering an empty library while every other test was green.

Expected results on a healthy build:

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | 0 errors |
| `cargo test` | 100 passed |
| UI self-test | 28 / 28 |

---

## Usage

```console
$ lively screens                          # list displays
$ lively set plasma-drift                 # by library id
$ lively set ./photo.png --monitor 0      # a file
$ lively set https://example.com          # a remote web wallpaper
$ lively pause
$ lively volume +10
$ lively seek 30 --monitor 0              # video wallpapers only
$ lively set-prop Intensity 80            # <NAME> <VALUE>
$ lively close --monitor 0
$ lively shutdown
```

A daemon is started automatically if one is not already running, so `set`
just works. Set `LIVELY_NO_AUTOSTART=1` to disable that — useful when a test
needs to control daemon lifetime itself.

### Common commands

Every command takes `-v/--verbose` for diagnostics, and `-h/--help` for its
exact arguments. `lively --help` lists them all.

| Command | Arguments | What it does |
|---|---|---|
| `lively screens` | — | List displays |
| `lively set <PATH>` | `-m, --monitor` | Set a wallpaper on a screen |
| `lively close` | `-m, --monitor` | Remove the wallpaper; default is **every** screen |
| `lively play` | — | Resume playback |
| `lively pause` | — | Pause playback |
| `lively volume <VALUE>` | — | Absolute `50`, or relative `+10` / `-5` |
| `lively seek <VALUE>` | `-m, --monitor` | Jump to a percentage; video only |
| `lively set-prop <NAME> <VALUE>` | `-m, --monitor` | Set a wallpaper property |
| `lively list-library` | — | Show the library |
| `lively add-library <PATH>` | — | Import a file or project folder |
| `lively remove-library <ID>` | — | Remove one, by the id from `list-library` |
| `lively show-app` | — | Open the settings window |
| `lively shutdown` | — | Stop the daemon |

Notes worth knowing:

- `--monitor` is an index from `lively screens`. On a single-display Mac the
  only valid index is `0`.
- `lively close` defaults to `-1`, which clears **every** display. Pass
  `-m 0` to clear just one.
- `lively set-prop` takes its name and value **positionally**, not as
  `--name`/`--value`.
- `lively seek` only applies to video wallpapers; on any other type it
  reports that rather than doing nothing.
- `lively remove-library` needs the **id** shown by `lively list-library`,
  which is not always the folder name. The six bundled wallpapers cannot be
  removed — they are re-seeded from the app on every launch.
- `lively set-prop` only affects renderers with a property channel. Web
  wallpapers receive it through `window.livelySetProperty`, shaders through
  their uniform buffer. On a plain image or video wallpaper the daemon
  reports that properties are unsupported instead of pretending to succeed.

### Where things happen

The daemon lives in the **menu bar**. Close the settings window and it returns
to a menu bar item with no Dock icon. Your wallpaper keeps running.

---

## Make a wallpaper

A wallpaper is a directory with a `LivelyInfo.json` and a payload:

```console
$ mkdir -p ~/wallpapers/my-wallpaper
$ cp /path/to/image.png ~/wallpapers/my-wallpaper/wallpaper.png
```

`~/wallpapers/my-wallpaper/LivelyInfo.json`:

```json
{
  "Title": "My Wallpaper",
  "Type": 5,
  "FileName": "wallpaper.png",
  "Desc": "A still image.",
  "Author": "You",
  "License": "Apache-2.0"
}
```

Then add it to the library:

```console
$ lively add-library ~/wallpapers/my-wallpaper
```

Or just point at it directly:

```console
$ lively set ~/wallpapers/my-wallpaper
```

### Types

| `Type` | Kind |
|---|---|
| 0 | Video |
| 1 | Web (local HTML file) |
| 2 | Web (remote URL) |
| 5 | Image |
| 6 | Shader (Metal) |

### Adjustable properties

Add a `LivelyProperties.json` and the UI generates a controls panel for it —
no extra work on your side.

```json
{
  "Intensity": {
    "type": "slider",
    "text": "Intensity",
    "value": 50,
    "min": 0,
    "max": 100,
    "step": 1
  },
  "Accent":  { "type": "color", "text": "Accent", "value": "#ff7a29" },
  "Vignette": { "type": "checkbox", "text": "Vignette", "value": true }
}
```

Full reference — including how properties reach each renderer — in
[docs/spec.md](docs/spec.md).

---

## How it works

One process per user session, acting as both daemon and UI:

```
$ lively set plasma-drift
        │  no daemon running
        ▼
   spawn `lively daemon`
        │
        ▼
   ┌────────────────────────────────────┐
   │  main thread ── IPC poll ── AppKit │
   │       │                            │
   │       ├── engine ── platform ── native
   │       └── SwiftUI window (same process)
   └────────────────────────────────────┘
```

Details in [docs/architecture.md](docs/architecture.md).

---

## Development

```console
$ cargo test                              # 100 tests
$ cargo fmt --all -- --check
$ cargo clippy --all-targets -- -D warnings
$ ./scripts/leak-check.sh                 # memory soak
```

| Script | Purpose |
|---|---|
| `scripts/make-app.sh` | Build and assemble `dist/Lively.app` |
| `scripts/leak-check.sh` | Memory soak |
| `scripts/linux-check.sh` | Compile the Linux backend (add `--link` to link) |
| `scripts/linux-sysroot.sh` | Build a Linux sysroot for cross-compiling from macOS |
| `scripts/make-icon.swift` | Generate the app icon |
| `scripts/render-shader.swift` | Offline Metal shader renderer |

`dist/` is a build artifact and is git-ignored. A fresh clone needs one
`./scripts/make-app.sh` to produce it.

Conventions, layout, and the rules worth knowing before contributing are in
[docs/development.md](docs/development.md).

---

## Troubleshooting

**The settings window does not open.**

The daemon is the menu bar item. Try `lively show-app`. If nothing happens:

```console
$ lively shutdown && ./scripts/make-app.sh --run
```

An `LSUIElement` app that was already running will keep its old activation
policy. Restarting the process clears it.

**The window is empty and says it is not connected to a daemon.**

The UI's command handler failed to install. Check the daemon is alive:

```console
$ lively screens
```

If that works but the window is still empty, rebuild — the Swift half is
linked at build time and an old binary will not pick up Swift changes.

**`lively set` says the path does not exist, but the file is there.**

Use the library id rather than a title. If you typed a title, the error lists
the ids that exist:

```
error: `Plasma` is a library item name, not an id. Did you mean: Plasma Drift (id: plasma-drift)
```

**A remote wallpaper kills the daemon.**

This was a real bug and is fixed. If you built before the fix, rebuild from
current `main` — see the changelog entry for `NSInvalidArgumentException`.

**Nothing happens and there is no output.**

The daemon logs to stdout. Run it in the foreground:

```console
$ lively shutdown
$ ./dist/Lively.app/Contents/MacOS/Lively daemon
```

**Gatekeeper refuses to open the app.**

The bundle is ad-hoc signed, which is enough to run locally but not to
distribute. Build from source on the machine, or remove the quarantine flag:

```console
$ xattr -dr com.apple.quarantine /Applications/Lively.app
```

**Wallpapers are applied but invisible behind other windows.**

That is correct — the wallpaper sits below application windows, above the
desktop picture. Close or minimise windows to see it.

**Two daemons seem to be fighting.**

They cannot both run; the second removes the stale socket at startup. If you
see repeated socket errors, a daemon is being launched twice — note that
`make-app.sh --run` already handles this by stopping any previous copy.

---

## Project status

Verified on the development machine:

```
cargo test                        100 passed
cargo clippy --all-targets        0 errors, warnings denied
cargo fmt --check                 clean
UI self-test (Swift→Rust→Swift)   28 / 28
```

Memory holds flat across repeated swaps; a 560-cycle web-wallpaper soak left
zero crashes and ~20 KB total leaked bytes. Visual checks confirmed real
thumbnails, correct metadata, property controls that adapt per wallpaper type,
and a wallpaper genuinely visible on the desktop.

**Open items:** the Screens and Settings pages have not been visually
confirmed; the Linux backend compiles and links but has never been executed;
random-wallpaper rotation, the audio visualiser, and any Linux GUI are
unimplemented. All of this is enumerated in
[docs/quality.md §5](docs/quality.md#5-what-has-not-been-verified).

---

## Documentation

| Document | Contents |
|---|---|
| [Architecture](docs/architecture.md) | Process model, module map, the platform seam, the run loop, memory ownership |
| [Specification](docs/spec.md) | Wallpaper package format, property controls, IPC protocol, configuration |
| [Quality](docs/quality.md) | What is verified, how, and what is not |
| [macOS](docs/macos.md) | Window layering, renderers, the exception hazard |
| [Linux](docs/linux.md) | Design intent per compositor, and why GNOME is out |
| [Development](docs/development.md) | Build, test, conventions, layout |
| [Security](SECURITY.md) | Threat model and trust boundaries |
| [Contributing](CONTRIBUTING.md) | Before you open a pull request |

---

## Licence

Apache License 2.0 — see [LICENSE](LICENSE). Bundled wallpapers are part of
this project under the same licence; see [NOTICE](NOTICE).