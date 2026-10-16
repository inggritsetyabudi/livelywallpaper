# Linux support

> ## Status: it builds. It has never been run.
>
> **What is verified.** The backend compiles for `x86_64-unknown-linux-gnu`
> and links into a working ELF binary — the C shim in all three of its feature
> configurations, the Rust side, and the full link. `./scripts/linux-check.sh
> --link` reproduces this from a Mac using a cross toolchain.
>
> **What is not verified.** Nothing has been *executed*. A Linux binary cannot
> run on macOS, and no container or virtual machine was available. So: no
> wallpaper has been shown on an X11 or Wayland session, no screenshot taken,
> no compositor exercised.
>
> Treat the X11 path as plausible-and-compiles, and the Wayland path as
> incomplete (see Known gaps). The unproven assumptions are listed in
> [quality.md §5](quality.md#5-what-has-not-been-verified).

## Why this is not simply "the same thing on Linux"

macOS makes wallpaper easy: an `NSWindow` placed below every other window.
Linux has no equivalent in the base protocols, so support necessarily splits
by window system, and one common desktop is genuinely not supported.

| Target | Status | Video | Web | Image | Shader | GUI |
|---|---|---|---|---|---|---|
| **macOS** | **verified** | AVFoundation | WKWebView | NSImage | Metal | SwiftUI |
| **X11** | builds, unrun | GStreamer | WebKitGTK | Cairo | not implemented | CLI only |
| **Wayland** (layer-shell) | builds, unrun | GStreamer | X11 only | Cairo | not implemented | CLI only |
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
  screen 0:  Display 1
    video: unavailable — built without the linux-video feature
    web:   unsupported on Wayland — use an X11 session
    image: ok
    shader: not implemented on Linux
```

Failing loudly is the intended behaviour everywhere. A renderer that cannot
work reports why; it does not render a black rectangle and let the user
wonder.

## Verifying the build

From a Linux machine, the normal commands are enough:

```console
$ cargo build --release --features linux-video,linux-web
$ cargo test
```

From a Mac, use the cross toolchain. This is how the backend was found to have
never compiled in the first place:

```console
$ rustup target add x86_64-unknown-linux-gnu
$ brew install --force-bottle messense/macos-cross-toolchains/x86_64-unknown-linux-gnu
$ ./scripts/linux-sysroot.sh          # headers + libraries, from Ubuntu packages
$ ./scripts/linux-check.sh            # C shim, cargo check, test build
$ ./scripts/linux-check.sh --link     # full link
```

Neither script installs anything system-wide, and neither runs the result —
a Linux binary cannot execute on macOS.

## Known gaps

Beyond the unverified-status caveat above, these are gaps by construction:

- The shader renderer is not implemented on Linux.
- There is no Linux GUI — CLI only. `lively show-app` reports this.
- Wayland creates one surface spanning all outputs. Per-output surfaces need
  `wl_output` enumeration, which is Phase C.
- The layer-shell protocol bindings are hand-written
  (`src/platform/native/wlr_layer_shell.h`) because `wayland-scanner` cannot run
  on a macOS build host. They cover only what the backend uses.
- The screensaver is macOS-only; `lively screensaver` says so.
- Wayland fullscreen detection always answers "no". Wayland deliberately keeps
  `_NET_ACTIVE_WINDOW` private, so there is no honest answer, and guessing
  "yes" would pause wallpapers for no visible reason.

## Testing on a real machine

```console
$ cargo build --release --features linux-video,linux-web
$ ./target/release/livelywallpaper diagnostics
```

`diagnostics` prints the session type, which globals the compositor offered,
the detected displays with their geometry, and which renderers are compiled in.
It opens no surface and changes no state, so it is safe to run on a machine
where nothing works. Paste its output into a bug report.

What to try, in order:

1. `lively diagnostics` — confirm `zwlr_layer_shell_v1: yes`. If it says no,
   the compositor cannot host a wallpaper surface; on GNOME that is expected
   and not a bug.
2. `lively screens` — expect one entry per monitor under X11.
3. `lively set <image>` — look behind other windows.
4. `lively daemon` in a terminal and watch for errors.

## What was fixed to get here

Recorded because each of these was invisible until something compiled it:

- **The surface handle was double-freed across the FFI boundary.** Rust took a
  `Box::from_raw` over a `calloc`'d record, copied out of it and dropped the
  `Box`, so Rust's allocator freed memory C still owned; destruction then
  wrapped the copy in a fresh `Box` and asked C's `free` to release it. The
  handle is now an opaque `NonNull` that C allocates and C frees.
- **`xcb_put_image` was called with 14 arguments against a 12-argument
  signature**, passing an integer where a pointer belongs, and no graphics
  context existed to draw with. The image went into a pixmap that was freed in
  the same breath.
- **X11 screens were treated as monitors.** A dual-display machine is one screen
  with two RANDR outputs, so per-monitor wallpapers were impossible. Monitors
  now come from RANDR, falling back to screens when RANDR is absent.
- **Override-redirect windows were mapped on top of everything,** because no
  window manager re-stacks an unmanaged window. They are now explicitly stacked
  below.
- **The daemon spun.** `pump_events` slept for a fixed slice every iteration.
  It now blocks in `poll(2)` on the display server's fd.
- **AppKit-only symbols were called from shared code,** so the Linux target
  could not link at all. Everything platform-specific now goes through the
  `Platform` trait.
- **Config lived in `~/Library/Application Support`** on every platform. Linux
  now uses XDG.

## How to move this to "verified"

1. Run it on X11: apply an image wallpaper, screenshot the desktop.
2. Run it on Wayland under Hyprland and Sway: same.
3. Soak it: 500 image swaps, watching RSS.
4. Get the CI Linux job green.
5. Implement `wl_surface` creation for the layer-shell path, then repeat 2.