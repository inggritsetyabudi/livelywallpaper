# Contributing

Thanks for looking. A few things will make a contribution land cleanly.

## Getting set up

```console
$ git clone https://github.com/your-org/livelywallpaper
$ cd livelywallpaper
$ ./scripts/make-app.sh --run
```

See [docs/development.md](docs/development.md) for the full toolchain.

## Before you open a pull request

All three must pass:

```console
$ cargo fmt --all -- --check
$ cargo clippy --all-targets -- -D warnings
$ cargo test
```

If you touched the UI, also run the self-test through the real path:

```console
$ LIVELY_UI_SELFTEST=1 ./dist/Lively.app/Contents/MacOS/Lively
```

## The rule that matters most

**If you touched the UI, test the UI through Swift.**

A test that calls the Rust side directly will pass while the Swift wiring is
entirely broken. That is not hypothetical — the settings window shipped with a
dead command handler, rendering an empty library, while every data-layer test
was green. The self-test exists because of that.

If you add a UI check, confirm it can fail: break the behaviour deliberately
and watch it go red.

## If you touched a renderer

Run the soak:

```console
$ ./scripts/leak-check.sh
```

RSS should plateau. It may rise slightly at first and even fall — that is
settling, not a leak. A renderer that grows without bound has a problem.

## Code conventions

- **Comments explain why.** A comment restating the code is noise; one
  recording a constraint, a past bug, or a non-obvious invariant earns its
  line.
- **Failures stay visible.** A renderer that cannot work reports why rather
  than drawing black. A bad wallpaper should fail one command, not take the
  daemon down.
- **No `unwrap()` on anything from disk, the network, or a wallpaper.** All
  three are user-controlled.
- **Check before using a dependency.** Do not assume a crate is available.

## Linux

The Linux backend has never been compiled. If you touch it, the first thing
to do is get it to build — that alone is a real contribution. See
[docs/linux.md](docs/linux.md) for the specific unproven assumptions.

## Commit messages

Describe the defect, not just the change. "Pass remote URLs to the WebKit URL
loader" beats "fix web url bug" — the first explains why the code looks like
it does, and the second will be unreadable in a year.

## Licence

Contributions are licensed under the Apache License 2.0, the same as the
project. See [LICENSE](LICENSE).