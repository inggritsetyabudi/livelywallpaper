# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| `master` (unreleased) | yes |
| Anything older | no — there are no older releases |

## Reporting a vulnerability

Report privately through GitHub Security Advisories
(`Security` → `Report a vulnerability`) on the repository.

Please include: what you found, how to reproduce it, and what you think the
impact is. You should get an acknowledgement within a few days.

Do not open a public issue for a security problem.

## Threat model

Lively renders content it did not author, on the user's desktop, continuously
and unattended. The realistic risks are:

| Risk | Mitigation |
|---|---|
| A wallpaper crashes or wedges the daemon | Objective-C exceptions across the FFI boundary are caught and returned as errors |
| A wallpaper consumes unbounded memory | Renderers reuse bitmaps; soak-tested to flat RSS |
| A malformed file causes a panic | `panic = "abort"` is not set; `catch_unwind` at the FFI boundary; no `unwrap()` on disk, network, or wallpaper input |
| Path traversal via a package | Payloads resolve relative to the package root and are checked before use |
| Two daemons fighting over the socket | The stale socket is cleared at start; only one daemon runs per user |
| Web content escaping its directory | `loadFileURL:` grants read access to the file's own directory only |

## Trust boundaries

**Wallpaper content is untrusted.** A wallpaper package may come from a
third party. Web wallpapers run JavaScript in a `WKWebView` with file read
access restricted to their own directory. Shader wallpapers compile `.metal`
source at runtime.

This is inherent to a live wallpaper engine and is the same trust model as a
browser. Do not treat installing a wallpaper as harmless.

**The IPC socket is per-user.** It lives under the user's own temporary
directory and carries no authentication beyond filesystem permissions. Any
process running as that user can drive the daemon.

## Security-relevant notes

- The daemon runs as the invoking user. It is never installed as a privileged
  helper and does not require elevation.
- `LSUIElement` keeps the app out of the Dock while idle, so the wallpaper
  window can sit below everything without a visible app switcher entry.