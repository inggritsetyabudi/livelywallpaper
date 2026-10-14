# Linux support

macOS makes wallpaper easy: an `NSWindow` placed below other windows. Linux has
no equivalent in the base protocols, so support is split by window system, and
one desktop environment is genuinely not supported.

## Summary

| | Wallpapers | Video | Web | Image | Shader | GUI |
|---|---|---|---|---|---|---|
| **macOS** | yes | AVFoundation | WKWebView | NSImage | Metal | SwiftUI |
| **X11** | yes | GStreamer | WebKitGTK | Cairo | not yet | CLI only |
| **Wayland (layer-shell)** | yes | GStreamer | **X11 only** | Cairo | not yet | CLI only |
| **GNOME / Mutter** | **no** | — | — | — | — | — |

Image and video render straight into the wallpaper surface. Web wallpapers are
a GTK toplevel instead — WebKitGTK only renders into a `GtkWindow`, and GTK
offers no way to adopt our layer-surface — so on Wayland they are refused rather
than shown as a blank window.

The shader renderer is not implemented on Linux; it reports that rather than
showing black.

## Why GNOME is not supported

Wayland's core protocol has no concept of "a window beneath other windows". A
compositor draws its own wallpaper and nothing else may sit underneath.

The closest thing is `zwlr_layer_shell_v1`, with `layer = "bottom"`. It is
implemented by:

- Sway, Hyprland, river, niri, wayfire, dwl, Qtile (Wayland), any wlroots compositor
- KDE Plasma 6

**GNOME/Mutter implements neither layer-shell nor any public wallpaper API.**
The commonly circulated workarounds reach into Mutter's private behaviour —
GNOME Shell extensions that reparent windows, or abusing `gdk_set_input_shape`
— and they break between releases. On GNOME, Lively will report that it cannot
create a wallpaper surface rather than silently showing nothing.

KDE Plasma 5 (X11) is a special case: Plasma 5 replaced its wallpaper QML
pluggable with a static colour, so wallpaper plugins do not work there either.

## Building

```sh
# X11 + Wayland layer-shell surfaces, images via Cairo
cargo build --release

# With video wallpapers (GStreamer)
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
cargo build --release --features linux-video

# With web wallpapers (WebKitGTK)
sudo apt install libwebkit2gtk-4.1-dev
cargo build --release --features linux-web
```

The media features are opt-in so a plain `cargo build` still succeeds on a
machine with neither stack installed. When a renderer is not compiled in, the
daemon says so explicitly instead of failing at the point of use.

## Verifying

**The Linux build has never been run, and has never even been syntax-checked.**
It was written on macOS, where neither Docker, a cross-toolchain, nor the Cairo
and XCB development headers are available — `gcc -fsyntax-only` fails at the
first `#include <cairo/cairo.h>`.

Three bugs in the C shim were caught by reading it rather than compiling it:

- `gst_bus_add_watch` dispatches from a GLib main loop this daemon does not
  run, so EOS and errors would never have been observed. Replaced with
  `gst_bus_set_sync_handler`, which runs inside the state change.
- `video-changed` was connected to `playbin`, which is not a signal on it, and
  before the element was added to the bin. Removed.
- Nothing looped the video, so a wallpaper would stop at the end of the file.
  The bus handler now seeks back on EOS, matching `AVPlayerLooper` on macOS.

Treat those as the floor, not the ceiling: expect more.

`.github/workflows/build.yml` is therefore the real test: it builds the Linux
matrix on Ubuntu, runs `fmt`, `clippy -D warnings` and the test suite. Until
that workflow is green, treat Linux as unverified.

If you are porting or extending this, run it before trusting it:

```sh
# what CI runs on Ubuntu
sudo apt install libxcb1-dev libcairo2-dev libegl1-mesa-dev libgl1-mesa-dev \
  libwayland-dev wayland-protocols
cargo build --release --features linux-video
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Under a real session:

```sh
WAYLAND_DISPLAY=wayland-1 ./target/release/livelywallpaper daemon &
./target/release/livelywallpaper set plasma-drift
```

An unsupported compositor produces a message naming the problem, not a silent
blank desktop:

```
error: could not create a layer-shell surface. This compositor does not
       implement wlr-layer-shell (GNOME/Mutter does not).
```

## Architecture

All OS-specific code sits behind the `Platform` trait in `src/platform/mod.rs`:

```
src/platform/
  mod.rs        trait + SurfaceKind + PlaybackState, and the GNOME caveat
  macos.rs      wraps the Objective-C layer in src/native
  linux.rs      session detection, capability reporting, error wording
  linux_sys.rs  thin Rust FFI over the C shim
  native/
    lively_linux.c   XCB + Cairo + Wayland layer-shell
```

`SurfaceKind` and `PlaybackState` are the shared vocabulary. The `kind` travels
with a `ViewHandle` so teardown and transport calls dispatch correctly without
re-inferring the type.

The CLI, IPC, library, config, and wallpaper routing are plain Rust and compile
everywhere — that is why the trait exists rather than `#[cfg]` scattered through
the engine.