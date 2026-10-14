# Linux support

macOS makes wallpaper easy: an `NSWindow` placed below other windows. Linux has
no equivalent in the base protocols, so support is split by window system, and
one desktop environment is genuinely not supported.

## Summary

| | Wallpapers | Video | Web | Image | GUI |
|---|---|---|---|---|---|
| **macOS** | yes | AVFoundation | WKWebView | NSImage | SwiftUI |
| **X11** | yes | needs GStreamer | needs WebKitGTK | Cairo | CLI only |
| **Wayland (layer-shell)** | yes | needs GStreamer | needs WebKitGTK | Cairo | CLI only |
| **GNOME / Mutter** | **no** | — | — | — | — |

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

**The Linux build has never been run.** It was written on macOS, where neither
Docker nor a cross-toolchain was available, so not one line has been executed.

`.github/workflows/build.yml` is therefore the real test: it builds the Linux
matrix on Ubuntu, runs `fmt`, `clippy -D warnings` and the test suite. Until
that workflow is green, treat Linux as unverified.

If you are porting or extending this, run it before trusting it:

```sh
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
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