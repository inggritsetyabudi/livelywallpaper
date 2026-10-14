# Lively Wallpaper

A native live wallpaper engine for macOS, with a SwiftUI settings window and a
scriptable CLI. Image, video, web, and Metal shader wallpapers, with a library
of six built-ins that ship with the app.

> **Status: macOS is verified. Linux is not.**
> The Linux backend has never been compiled or run — see
> [docs/linux.md](docs/linux.md). Nothing in this README should be read as
> claiming otherwise.

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

## Install

```console
$ git clone https://github.com/your-org/livelywallpaper
$ cd livelywallpaper
$ ./scripts/make-app.sh
$ ./scripts/make-app.sh --run
```

Requires stable Rust and the Xcode command line tools.

## Use

```console
$ lively set plasma-drift          # by library id
$ lively set ./photo.png --monitor 1
$ lively set https://example.com   # remote web wallpaper
$ lively screens
$ lively pause
$ lively volume +10
$ lively set-prop --name Intensity --value 80
$ lively close --monitor 0
```

A daemon is started automatically if one is not running. Set
`LIVELY_NO_AUTOSTART=1` to disable that.

## Make a wallpaper

A wallpaper is a directory with a `LivelyInfo.json` and a payload:

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

Add a `LivelyProperties.json` and the UI generates a controls panel for it —
sliders, colours, and checkboxes. Full reference in
[docs/spec.md](docs/spec.md).

## Documentation

| Document | Contents |
|---|---|
| [Architecture](docs/architecture.md) | Process model, module map, the platform seam, the run loop, memory ownership |
| [Specification](docs/spec.md) | Wallpaper package format, property controls, IPC protocol, configuration |
| [Quality](docs/quality.md) | What is verified, how, and what is not — including the gaps |
| [macOS](docs/macos.md) | Window layering, renderers, the exception hazard |
| [Linux](docs/linux.md) | Design intent per compositor, and why GNOME is out |
| [Development](docs/development.md) | Build, test, conventions, layout |

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
confirmed; the Linux backend is unbuilt; random-wallpaper rotation, the audio
visualiser, and any Linux GUI are unimplemented. All of this is enumerated in
[docs/quality.md §5](docs/quality.md#5-what-has-not-been-verified).

## Licence

Apache License 2.0 — see [LICENSE](LICENSE).