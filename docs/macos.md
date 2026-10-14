# macOS

The macOS backend is the reference implementation and the only platform
currently verified end to end.

## Composition model

A wallpaper is an `NSWindow` on its own desktop layer, placed below every
other window but above the desktop picture. This is why wallpapers work on
macOS the way they do, and why the same approach does not port to Wayland.

```
  ┌─────────────────────────────────────────┐
  │  application windows        (above)     │
  ├─────────────────────────────────────────┤
  │  ▓▓▓▓▓▓▓▓  Lively wallpaper window  ▓▓▓  │  ← NSWindow, below
  ├─────────────────────────────────────────┤
  │  desktop picture / Finder icons         │
  └─────────────────────────────────────────┘
```

The wallpaper window is non-activating, non-focusable, and spans the full
bounds of its screen. It is created once per screen and its contents are
swapped, so changing wallpaper does not recreate the window.

## Renderers

| Renderer | Technology | Notes |
|---|---|---|
| Image | `NSImage` | Decoded once, cached as a bitmap |
| Video | `AVPlayerLayer` | Looping; position and volume controlled |
| Web (local) | `WKWebView` + `loadFileURL:` | Read access granted to the file's own directory |
| Web (remote) | `WKWebView` + `loadRequest:` | Separate code path — see below |
| Shader | `CAMetalLayer` | Compiled `.metal`, rendered at display rate |

### Why web local and remote are separate paths

`loadFileURL:allowingReadAccessToURL:` and `loadRequest:` are not
interchangeable. Passing an `https` URL to the file loader raises
`NSInvalidArgumentException`, which aborts the process when it unwinds into
Rust. `SurfaceKind::WebFile` and `SurfaceKind::WebUrl` exist so the correct
loader is chosen by construction. See
[architecture.md §3](architecture.md#3-the-platform-seam).

### Objective-C exceptions

Anything crossing from Rust into WebKit is wrapped:

```objc
static void *lively_guarded(void *(^block)(void), void *fallback) {
    @try    { return block(); }
    @catch (NSException *e) { return fallback; }
}
```

Rust cannot catch a foreign exception. Without this guard, one bad wallpaper
takes the whole daemon down. With it, the command returns an error and the
daemon survives.

## Window and activation policy

The daemon is a menu-bar app with no Dock icon (`LSUIElement`), which is what
you want while it is only painting a desktop. Showing the settings window
requires a focusable window, so `presentWindow()` switches the activation
policy to `.regular` and switches back to `.accessory` on close:

```swift
if window == nil { NSApp.setActivationPolicy(.regular) }
```

The LaunchAgent starts the daemon at login in accessory mode; the window only
appears when explicitly requested.

## Run loop

The Rust main loop polls the IPC socket and pumps AppKit every pass. See
[architecture.md §4](architecture.md#4-the-main-loop) for why main-thread work
has a tight budget and what that means for renderer implementation.

## Building

```console
$ ./scripts/make-app.sh
$ ./scripts/make-app.sh --run      # build, then launch
```

Produces `dist/Lively.app`. Requirements: stable Rust, Xcode command line
tools, and Swift. `build.rs` compiles the Objective-C sources with `cc`.

## Verifying

```console
# The UI self-test exercises the real Swift path
LIVELY_UI_SELFTEST=1 ./dist/Lively.app/Contents/MacOS/Lively
```

Capture the window for a visual check:

```console
$ screencapture -x -o -l$(window_id) shot.png
```

`window_id` comes from `CGWindowListCopyWindowInfo`. Note that `System Events`
reports zero windows for the app unless Terminal itself holds the Accessibility
permission — the window exists regardless, which makes the accessibility count
a misleading health check. The window name is `Lively Wallpaper`, not the
process name `Lively`.

Terminal needs **Screen Recording** permission for `screencapture`. This is
granted in System Settings → Privacy & Security → Screen & System Audio
Recording, and Terminal must be fully quit and reopened afterwards.