#!/bin/bash
#
# Build and check the Linux backend from a macOS host, using a cross toolchain.
#
# The Linux target cannot be verified from a Mac any other way, and an
# unverified target is exactly how this backend spent its life being broken:
# it had never been compiled, and it did not even link.
#
# What this checks, in increasing cost:
#   1. the C shim compiles, in each feature configuration
#   2. `cargo check` passes for the Linux target
#   3. the binary links
#
# It does NOT run anything: a Linux binary cannot execute on macOS. Runtime
# behaviour still needs a real machine or a container.
#
# # Setup (once)
#
#   rustup target add x86_64-unknown-linux-gnu
#   brew install --force-bottle messense/macos-cross-toolchains/x86_64-unknown-linux-gnu
#   ./scripts/linux-sysroot.sh          # builds the headers + libraries
#
# # Use
#
#   ./scripts/linux-check.sh            # C only (fast)
#   ./scripts/linux-check.sh --link     # full link (slower)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="${LIVELY_LINUX_TARGET:-x86_64-unknown-linux-gnu}"
CROSS="${LIVELY_CROSS_PREFIX:-${TARGET%*-unknown-*}}"
PREFIX="${CROSS}-linux-gnu"
SYSROOT="${LIVELY_LINUX_SYSROOT:-${HOME}/.lively-linux-sysroot}"

if [ ! -d "$SYSROOT" ]; then
    echo "error: no sysroot at $SYSROOT" >&2
    echo "       build one with ./scripts/linux-sysroot.sh" >&2
    exit 1
fi

CC="$(command -v "$PREFIX-gcc" || true)"
if [ -z "$CC" ]; then
    echo "error: $PREFIX-gcc not found in PATH" >&2
    echo "       brew install --force-bottle messense/macos-cross-toolchains/$TARGET" >&2
    exit 1
fi

LIBDIR="$SYSROOT/usr/lib/x86_64-linux-gnu"

# Apply the cross toolchain to every cargo invocation. cargo test links as well
# as builds, and without this it picks up the host `cc` and fails with
# "ld: unknown options" rather than anything to do with this project.
#
# --allow-shlib-undefined: the cross sysroot does not carry the full transitive
# closure of Cairo's own dependencies. Those resolve at runtime on a real
# machine; refusing to link over them would only hide our own errors.
upper="$(printf '%s' "$TARGET" | tr '[:lower:]-' '[:upper:]_')"
export CARGO_TARGET_"$upper"_LINKER="$CC"
export CC_"${TARGET//-/_}"="$CC"
export CFLAGS_"${TARGET//-/_}"="--sysroot=$SYSROOT -I$SYSROOT/usr/include -I$SYSROOT/usr/include/xcb"
export RUSTFLAGS="-C link-arg=--sysroot=$SYSROOT \
    -C link-arg=-L$LIBDIR \
    -C link-arg=-L$SYSROOT/usr/lib \
    -C link-arg=-L$SYSROOT/usr/lib64 \
    -C link-arg=-Wl,-rpath-link,$LIBDIR \
    -C link-arg=-Wl,--allow-shlib-undefined"
CC_FLAGS=(--sysroot="$SYSROOT" "-I$SYSROOT/usr/include" "-I$SYSROOT/usr/include/xcb"
          -I"$ROOT/src/platform/native" -Wall -Wextra -Wno-unused-parameter)

# Every configuration the C can be built in. A shim that only compiles with
# one feature set is a shim that breaks the moment someone builds without them.
compile_c() {
    local label="$1"; shift
    local out; out="$(mktemp -t lively-linux-c).o"
    if "$PREFIX-gcc" -c "$ROOT/src/platform/native/lively_linux.c" \
        "${CC_FLAGS[@]}" "$@" -o "$out" 2>"$out.log"; then
        echo "    ok   $label"
        rm -f "$out" "$out.log"
        return 0
    fi
    echo "    FAIL $label"
    sed 's/^/         /' "$out.log" | head -20
    rm -f "$out.log"
    return 1
}

echo "==> C shim"
rc=0
compile_c "x11 + wayland"      -DLIVELY_HAVE_WAYLAND || rc=1
compile_c "x11 only"                          || rc=1
compile_c "no media stack"      -DLIVELY_HAVE_WAYLAND || rc=1
[ $rc -ne 0 ] && exit 1

# `cc` only re-runs when build.rs is newer than its output. Editing the C shim
# without touching build.rs leaves a stale object linked in, which shows up as a
# baffling "undefined reference" to a function that is plainly in the source.
touch build.rs

echo "==> cargo check --target $TARGET"
if ! cargo check --target "$TARGET"; then
    exit 1
fi

echo "==> test build --target $TARGET"
# Compiled, not run: a Linux binary cannot execute on macOS. Running the suite
# is the CI Linux job's job.
if ! cargo test --target "$TARGET" --no-run; then
    exit 1
fi

if [ "${1:-}" = "--link" ]; then
    echo "==> link"
    cargo build --target "$TARGET"
    file "target/$TARGET/debug/livelywallpaper"
fi

echo
echo "Linux build checks passed. Runtime behaviour is still unverified --"
echo "that needs a real Linux machine or a container."