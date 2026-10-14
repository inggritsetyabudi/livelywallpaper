# Linux support

> ## Status: experimental — not built, not tested
>
> The Linux backend in this repository **has never been compiled or run.**
> It was written ahead of the macOS work and has received no build fixes, no
> test passes, and no runtime verification.
>
> It lives here because the platform seam was designed for it and because the
> CI job that would compile it is declared. Treat everything below as a design
> intent, not as a description of working software.
>
> Known unproven assumptions are listed in
> [quality.md §5](quality.md#5-what-has-not-been-verified).

## Why this is not simply "the same thing on Linux"

macOS makes wallpaper easy: an `NSWindow` placed below every other window.
Linux has no equivalent in the base protocols, so support necessarily splits
by window system, and one common desktop is genuinely not supported.

| Target | Status | Video | Web | Image | Shader | GUI |
|---|---|---|---|---|---|---|
| **macOS** | **verified** | AVFoundation | WKWebView | NSImage | Metal | SwiftUI |
| **X11** | *unbuilt* | GStreamer | WebKitGTK | Cairo | not implemented | CLI only |
| **Wayland** (layer-shell) | *unbuilt* | GStreamer | X11 only | Cairo | not implemented | CLI only |
| **GNOME / Mutter** | **not supported** | — | — | — | — | — |

Image and video are intended to render straight into the wallpaper surface.
Web wallpapers are intended to be a GTK toplevel instead — WebKitGTK only
renders into a `GtkWindow`, and GTK offers no way to adopt our layer-surface —
so on Wayland they are expected to be refused rather than shown as a blank
window.

## Why GNOME is not supported

Wayland's core protocol has no concept of "a window beneath other windows". A
compositor draws its own background and nothing else may sit underneath it.

The closest available mechanism is `zwlr_layer_shell_v1` with
`layer = "bottom"`, implemented by wlroots-based compositors and KDE Plasma 6:

- Sway
- Hyprland
- river
- niri
- KDE Plasma 6

GNOME's Mutter implements neither layer-shell nor any equivalent, and does not
expose a way to replace the desktop background. No workaround exists short of
patching the shell. Lively reports the limitation rather than drawing a
window the user did not ask for.

## Build

Media support is behind Cargo features so that `cargo build` succeeds on a
machine without the media stack installed.

| Feature | Default | Pulls in |
|---|---|---|
| `linux-wayland` | **yes** | XCB, Cairo, EGL, `zwlr_layer_shell_v1` |
| `linux-video` | no | GStreamer 1.x |
| `linux-web` | no | WebKitGTK 4.1 |

```console
$ sudo apt-get install -y \
    libxcb1-dev libcairo2-dev libegl1-mesa-dev libgl1-mesa-dev \
    libwayland-dev wayland-protocols \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libwebkit2gtk-4.1-dev

$ cargo build --release --features linux-video,linux-web
```

## Runtime expectations

`lively screens` reports what the current session actually supports, and each
renderer reports its own capability rather than failing opaquely:

```console
$ lively screens
  screen 0:  Built-in Display (WAYLAND)
    video: unavailable — built without the linux-video feature
    web:   unsupported on Wayland — use an X11 session
    image: ok
    shader: not implemented on Linux
```

Failing loudly is the intended behaviour everywhere. A renderer that cannot
work reports why; it does not render a black rectangle and let the user
wonder.

## Known gaps

Beyond the unverified-status caveat above, these are gaps by construction:

- The shader renderer is not implemented on Linux.
- There is no Linux GUI — CLI only.
- The Wayland path detects `zwlr_layer_shell_v1` through the registry but does
  not yet create a `wl_surface`.
- `SurfaceRecord` (Rust) and `LivelySurface` (C) are assumed to share a layout
  and ownership convention. Nothing enforces or checks this assumption, and it
  has never been exercised.
- The Linux event loop is unimplemented.

## How to move this to "verified"

1. Get the CI Linux job green — it is the only place this code compiles today.
2. Replace the assumed struct layout with an explicit C-ABI type, or move the
   record entirely into C and hand Rust an opaque handle.
3. Implement `wl_surface` creation for the layer-shell path.
4. Render one image wallpaper on X11 and one on Wayland, and screenshot both.
5. Run the memory and stability soaks from [quality.md](quality.md) on Linux.
6. Add the UI self-test equivalent to the X11/Wayland GUI, or document that
   there is no GUI.