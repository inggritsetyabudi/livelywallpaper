# Development

## Requirements

| | |
|---|---|
| Rust | stable, with `rustfmt` and `clippy` |
| Xcode | command line tools (Objective-C, Swift, Metal) |
| Xcode selection | `sudo xcode-select -s /Applications/Xcode_15.4.app` |

There are no third-party crates beyond `clap`, `serde`, `serde_json`,
`thiserror`, `log`, and `env_logger`. The macOS backend is plain Objective-C
compiled by `build.rs` via `cc`.

## Build

```console
$ cargo build --release          # binary only
$ ./scripts/make-app.sh          # binary + dist/Lively.app
$ ./scripts/make-app.sh --run    # ...and launch it
```

`make-app.sh` compiles the Rust binary, compiles and links the Swift sources,
writes `Info.plist`, generates the icon, and assembles the bundle.

## Test

```console
$ cargo test                                # 100 tests
$ cargo fmt --all -- --check                # formatting
$ cargo clippy --all-targets -- -D warnings # lints, warnings are errors
$ LIVELY_UI_SELFTEST=1 ./dist/Lively.app/Contents/MacOS/Lively
```

The UI self-test is only meaningful after `make-app.sh`, since it runs inside
the built app. Set `LIVELY_NO_AUTOSTART=1` if you want to control daemon
lifetime yourself.

### Writing UI tests

The self-test goes through the real `Swift → Rust → Swift` path on purpose.
A test that calls the Rust side directly will pass while the Swift wiring is
completely broken — that exact failure shipped once.

If you add a check, confirm it can fail by breaking the behaviour on purpose
and watching it go red.

### Memory

```console
$ ./scripts/leak-check.sh
```

Drives repeated wallpaper swaps and reports RSS. A renderer that leaks shows
RSS climbing without bound; a settled one plateaus and may even fall slightly.
Raw `leaks` totals in the tens of KB are normal for Cocoa.

## Helper scripts

| Script | Purpose |
|---|---|
| `scripts/make-app.sh` | Build and assemble the app bundle |
| `scripts/leak-check.sh` | Memory soak |
| `scripts/make-icon.swift` | Generate the app icon |
| `scripts/render-shader.swift` | Offline Metal shader renderer |

## Continuous integration

`.github/workflows/build.yml` runs on macOS 14 and Ubuntu 24.04. The Linux
matrix covers `default`, `linux-video`, and `linux-web`.

The Linux job has never been observed green. Until it is, macOS is the only
verified build and the Linux backend is experimental — see
[linux.md](linux.md).

## Layout

```
src/
├── main.rs              CLI, daemon lifecycle, command dispatch, run loop
├── engine.rs            target resolution, renderer selection, per-screen state
├── platform/
│   ├── mod.rs           Platform trait — the seam
│   ├── macos.rs         macOS backend
│   ├── linux.rs         Linux session detection and capability gating
│   ├── linux_sys.rs     Linux FFI (unbuilt)
│   └── native/          Objective-C and C implementations
├── ui/
│   ├── AppModel.swift   view model, FFI exports
│   ├── ContentView.swift  SwiftUI views
│   └── UiSelfTest.swift   self-test through the real path
├── ui.rs                JSON snapshot builder
├── ipc.rs               socket transport
├── library.rs           library, built-in seeding
├── config.rs            persisted settings
└── automation.rs        fullscreen / power watchers
```

## Conventions

- **Comments explain why, not what.** A comment restating the code is noise;
  one recording a constraint, a past bug, or a non-obvious invariant is worth
  the line.
- **Failures should be visible.** A renderer that cannot work reports why
  rather than rendering black. A bad wallpaper fails one command rather than
  taking the daemon down.
- **No `unwrap()` on input from disk, the network, or a wallpaper.** All three
  are user-controlled.
- **Never assume a library is available.** Check that the codebase already uses
  it before reaching for it.