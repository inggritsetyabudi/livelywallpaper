# Quality and verification

This document records what has actually been verified, how, and what has not.
Numbers here were measured on the development machine; anything claimed
without a reproduction command is labelled as such.

---

## 1. Automated gates

Run on every push and pull request (`.github/workflows/build.yml`).

| Gate | Command | Status |
|---|---|---|
| Unit and integration tests | `cargo test` | **100 passed** |
| Lints | `cargo clippy --all-targets -- -D warnings` | **0 errors** |
| Formatting | `cargo fmt --all -- --check` | **clean** |
| UI self-test | `LIVELY_UI_SELFTEST=1 dist/Lively.app/Contents/MacOS/Lively` | **28/28** |
| Release build | `cargo build --release` | success |

The Linux job is declared in CI but **has not been executed** — see §5.

## 2. The UI self-test, and why it exists

`src/ui/UiSelfTest.swift` runs the real path: **Swift → Rust → Swift**. It
drives the actual view model through the actual FFI and asserts on what comes
back.

This design is a direct response to a bug that every data-layer test passed
straight through. Both `lib.rs` and `AppModel.swift` exported a symbol named
`lively_ui_set_handler`. Whichever side registered last won, and in practice
**Swift never received the handler**. The daemon was healthy, every CLI test
passed, and the library loaded fine over IPC — but the window rendered an
empty library and a "not connected to daemon" banner.

A probe that calls the Rust side directly cannot catch this, because the probe
skips Swift entirely. So the self-test is written to go through Swift.

### Coverage

Library load and built-in partitioning · search · selection · metadata ·
apply · clear · volume · mute · play/pause · config read/write · error paths
· per-wallpaper property controls.

### The test is verified to be able to fail

A test that cannot fail is not a test. The suite was validated by mutation:

```
$ # sabotage built-in partitioning
$ LIVELY_UI_SELFTEST=1 ... 
FAIL library loads built-ins (0)
```

The suite correctly fails when the behaviour is broken. This is checked again
whenever the self-test changes.

## 3. Memory

`scripts/leak-check.sh` drives repeated wallpaper swaps and samples RSS.

| Scenario | Before | After | Status |
|---|---|---|---|
| Video, 400 swaps | 128 MB → 645 MB | 60 MB → 56 MB | flat |
| Image, 200 swaps | 411 MB → 844 MB | 82 MB → 82 MB | flat |
| Web (WKWebView), 560 swaps | — | 104 → 89.6 → 88.2 MB | plateau |
| `leaks` after soak | — | ~20 KB | stable |

The web figure is a **plateau, not a leak**: RSS rises, then falls, then
settles. Repeating WKWebView create/destroy 560 times left the daemon alive,
raised zero crash reports, and held leaked bytes flat at ~20 KB.

Root cause of the original growth was decoding a fresh bitmap per wallpaper
change — a 2560×1440 PNG is ~15 MB — and not releasing the previous one. The
native path now sets the new image and releases the old bitmap.

## 4. Stability

| Scenario | Result |
|---|---|
| All five renderers applied in sequence | ok |
| 480 mixed switches across 4 renderers | 0 crashes, 0 exceptions |
| Remote URL wallpaper | ok |
| Stale socket on daemon start | recovered |
| Concurrent daemons | prevented |

### A crash worth recording

`lively set https://example.com` terminated the daemon:

```
fatal runtime error: Rust cannot catch foreign exceptions, aborting
caught NSInvalidArgumentException: https:/ is not a file URL
```

Two independent defects. The macOS backend passed `is_local_file = true`
unconditionally, so remote URLs went to `loadFileURL:allowingReadAccessToURL:`.
And when WebKit raised, the Objective-C exception unwound into a Rust frame,
which aborts the process — no CLI-level error was possible.

Fixed by splitting `SurfaceKind::Web` into `WebFile` / `WebUrl`, and by
guarding the WebKit lifecycle so any Objective-C exception becomes a `NULL`
return. A bad wallpaper now fails one command and the daemon stays up. This
is the class of bug that only a soak test surfaces: every happy-path test
passed while the feature was fundamentally broken.

## 5. What has **not** been verified

Stated plainly, because the gap is larger than the verified surface.

**Linux — builds, has never been run.** The backend now compiles and links
into a working ELF binary, verified with `./scripts/linux-check.sh --link`
from macOS against a cross toolchain. That is a real advance: before this, it
had never been compiled, and it turned out not to even link.

What remains unproven:

- **Nothing has been executed.** No wallpaper has been shown on X11 or
  Wayland. There was no container or VM available on the development machine,
  and a Linux binary cannot run on macOS.
- The Wayland path detects `zwlr_layer_shell_v1` via the registry but does not
  create a `wl_surface`.
- The Linux shader renderer and GUI are unimplemented.
- The CI Linux job has still never been observed green.

The defects found by making it compile are listed in
[linux.md](linux.md#what-was-fixed-to-get-here). Several of them — a
double-free across the FFI boundary, a call with the wrong arity into
`xcb_put_image`, X11 screens mistaken for monitors — would not have been
caught by any amount of reading.

**Visual verification** was done by capturing the real window
(`screencapture -l<windowid>`), confirming: all six built-in thumbnails
render their real artwork, metadata and badges populate, long and CJK titles
truncate correctly, selection and the detail panel update, applying a
wallpaper surfaces a confirmation and updates the sidebar, per-type property
controls appear (web → palette, image → contrast/tint, shader → intensity),
and the wallpaper is genuinely visible on the desktop — verified by applying
*Graphite*, whose entire purpose is to be near-invisible, and observing the
desktop go dark.

The **Screens** and **Settings** pages were not visually confirmed. The
synthetic click harness mis-targeted the sidebar, so this remains open.

**Not implemented:** random-wallpaper rotation, the audio visualiser, and any
Linux GUI.

## 6. Reproducing the verification

```console
# Automated gates
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test

# Build and launch the app
./scripts/make-app.sh

# UI self-test through the real Swift path
LIVELY_UI_SELFTEST=1 ./dist/Lively.app/Contents/MacOS/Lively

# Memory soak
./scripts/leak-check.sh

# Linux backend: C shim, cargo check, test build, link
./scripts/linux-check.sh --link

# Confirm the self-test can fail (expect FAIL)
# then revert the sabotage
```