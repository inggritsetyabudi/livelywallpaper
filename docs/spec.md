# Specification

Two formats are specified here: the **wallpaper package** format that the
library reads, and the **IPC protocol** that the CLI and UI speak.

---

# Part 1 — Wallpaper package format

A wallpaper is a directory containing a metadata file and a payload. The
format is compatible with Lively Wallpaper's project layout so existing
projects can be used unmodified.

```
my-wallpaper/
├── LivelyInfo.json          required — metadata
├── LivelyProperties.json    optional — user-adjustable properties
├── index.html               payload  (web)
├── wallpaper.png            payload  (image)
├── shader.metal             payload  (shader)
├── clip.mp4                 payload  (video)
└── thumb.png                optional — library thumbnail
```

## 1.1 `LivelyInfo.json`

| Key | Type | Required | Meaning |
|---|---|---|---|
| `Title` | string | yes | Display name in the library |
| `Type` | int | yes | Renderer, see §1.2 |
| `FileName` | string | yes | Payload file, relative to the package root |
| `Desc` | string | no | Long description shown in the detail panel |
| `Author` | string | no | Attribution |
| `License` | string | no | Content licence, free-form |
| `AppVersion` | string | no | Version the package was authored for |
| `Thumbnail` | string | no | Library thumbnail |
| `Preview` | string | no | Preview image; falls back to `Thumbnail` |

```json
{
  "AppVersion": "2.0.0.0",
  "Title": "Plasma Drift",
  "Desc": "GPU-rendered domain-warped noise.",
  "Author": "Lively",
  "License": "Apache-2.0",
  "Type": 6,
  "FileName": "shader.metal",
  "Thumbnail": "thumb.png",
  "Preview": "thumb.png"
}
```

## 1.2 Type codes

These match Lively's numbering so third-party packages work as-is.

| Code | Label | Renderer | Notes |
|---|---|---|---|
| 0 | Video | `Video` | AVPlayer loop |
| 1 | Web | `WebFile` | Local `FileName`, read access granted to its directory |
| 2 | Web | `WebUrl` | Remote URL |
| 3 | App | — | Not implemented |
| 4 | GIF | — | Not implemented |
| 5 | Image | `Image` | Static, decoded once |
| 6 | Shader | `Shader` | Metal shader, animated |

Codes 1 and 2 both display as "Web" but select **different** loaders. See
[architecture.md §3](architecture.md#3-the-platform-seam) for why conflating
them is not safe.

## 1.3 `LivelyProperties.json`

Optional. Each key is a property name; each value is a control definition.

**Slider**

```json
{
  "type": "slider",
  "text": "Intensity",
  "value": 50,
  "min": 0,
  "max": 100,
  "step": 1
}
```

`value` may be a float. `step` defaults to 1.

**Colour**

```json
{ "type": "color", "text": "Accent", "value": "#ff7a29" }
```

Value is a `#rrggbb` string.

**Checkbox**

```json
{ "type": "checkbox", "text": "Vignette", "value": true }
```

### How properties reach the wallpaper

The contract differs per renderer:

- **Web** — `window.livelySetProperty(name, value)` is defined in every page.
- **Shader** — the value is written into the uniform buffer by name.
- **Image / Video** — consumed by the native renderer (brightness, tint,
  contrast).

The UI renders controls from this file, so a package gets a properties panel
with no extra work. Properties are also settable from the CLI:

```console
$ lively set-prop --screen 0 --name Intensity --value 80
```

## 1.4 Target resolution order

`lively set <target>` resolves in this order:

1. An existing **path** on disk — file or directory.
2. An **http(s) URL** — used as a web wallpaper.
3. A **library id** — resolved against `~/Library/Application Support/LivelyWallpaper/Library/`.

If the target looks like a library *title* rather than an id, the error names
the ids that do exist:

```
error: `Plasma` is a library item name, not an id. Did you mean: Plasma Drift (id: plasma-drift)
```

## 1.5 Built-in stamping

Built-in packages carry a `.lively-builtin` stamp file containing a version
string. Seeding compares the stamp and re-copies when it differs, which makes
upgrading the shipped wallpapers idempotent and lets an old copy be repaired
if a user deletes it.

---

# Part 2 — IPC protocol

The daemon listens on a Unix domain socket:

```
$TMPDIR/livelywallpaper-<uid>/daemon.sock
```

The transport is a length-prefixed JSON frame. Requests and responses are
newline-terminated JSON objects.

## 2.1 Requests

```jsonc
{"cmd": "ping"}
{"cmd": "set_wallpaper",    "path": "plasma-drift", "monitor": 0}
{"cmd": "close_wallpaper",  "monitor": 0}        // -1 = every screen
{"cmd": "play_pause",       "play": true}
{"cmd": "volume",           "value": "-10"}      // "50" or "+10" / "-5"
{"cmd": "seek",             "monitor": 0, "value": "30"}
{"cmd": "set_prop",         "monitor": 0, "name": "Intensity", "value": "80"}
{"cmd": "set_layout",       "mode": 0}           // 0 PerScreen, 1 Span, 2 Duplicate
{"cmd": "screenshot",       "monitor": 0}
```

## 2.2 Responses

```jsonc
{"ok": true,  "message": "ok"}
{"ok": false, "error": "screen 2 not found"}
```

## 2.3 `playbackStatus`

Deliberately **not** a full snapshot. The window polls this every 700 ms, and
returning the whole state meant re-reading the entire library and config from
disk twice a second for data the caller discarded.

```jsonc
{
  "ok": true,
  "playback": {
    "state": "playing",      // playing | paused | stopped
    "volume": 55,
    "muted": false,
    "position": 12.4,        // percent, video only
    "screen": 0
  }
}
```

## 2.4 UI channel

The UI uses a separate JSON command channel that carries UI intent rather
than daemon intent:

```jsonc
{"action": "state"}                                  // full snapshot
{"action": "select",  "id": "drift"}
{"action": "apply",   "id": "drift", "screen": 0}
{"action": "clear",   "screen": 0}
{"action": "search",  "query": "plasma"}
{"action": "properties", "id": "plasma-drift"}
{"action": "set_property", "id": "plasma-drift", "name": "Intensity", "value": "80"}
{"action": "config"}
{"action": "playbackStatus"}
```

Note the two verbs: `cmd` for CLI-shaped operations, `action` for UI intent.
Keeping them separate means the UI cannot invoke something the CLI cannot, and
neither can be added without a decision in both places.

---

# Part 3 — Configuration

`~/Library/Application Support/LivelyWallpaper/config.json`

```json
{
  "version": 1,
  "volume": 0.55,
  "is_muted": false,
  "is_paused": false,
  "pause_on_fullscreen": true,
  "pause_on_battery": true,
  "layout_mode": 0,
  "active_wallpapers": { "0": "plasma-drift" },
  "audio_visualizer_enabled": true,
  "interactive_mouse": false
}
```

Every field carries a `#[serde(default)]`, so a config written by an older
version — or hand-edited to something partial — still loads. `version` is
present for future migrations and is currently not enforced.

`active_wallpapers` maps screen index to a package path or URL and is what
makes a wallpaper survive a restart.

| `layout_mode` | Meaning |
|---|---|
| 0 | PerScreen — each display independent |
| 1 | Span — one wallpaper stretched across all displays |
| 2 | Duplicate — same wallpaper on every display |