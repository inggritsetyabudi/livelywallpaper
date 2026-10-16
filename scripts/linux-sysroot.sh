#!/bin/bash
#
# Build a Linux sysroot for cross-compiling from macOS.
#
# Takes a compiler sysroot (glibc, from the Homebrew cross toolchain) and adds
# the X11, Cairo and Wayland development files the backend needs, pulled from
# Ubuntu package archives. Nothing is installed system-wide and nothing is
# compiled -- this only assembles headers and shared libraries in one
# directory for the linker to find.
#
# Only needed when verifying the Linux backend from a Mac. On Linux itself,
# install the distribution packages listed in docs/linux.md instead.

set -euo pipefail

TARGET="${LIVELY_LINUX_TARGET:-x86_64-unknown-linux-gnu}"
SYSROOT="${LIVELY_LINUX_SYSROOT:-${HOME}/.lively-linux-sysroot}"
WORK="${TMPDIR:-/tmp}/lively-sysroot"
MIRROR="${LIVELY_APT_MIRROR:-http://archive.ubuntu.com/ubuntu}"
SUITE="${LIVELY_APT_SUITE:-noble}"

PREFIX="${TARGET%*-unknown-*}-linux-gnu"
PREFIX="${PREFIX/x86_64-i686/i686}"
CC="$(command -v "$PREFIX-gcc" || true)"

if [ -z "$CC" ]; then
    echo "error: $PREFIX-gcc not found" >&2
    echo "  brew install --force-bottle messense/macos-cross-toolchains/$TARGET" >&2
    exit 1
fi

# Where the Homebrew toolchain keeps its own sysroot.
BASE_SYSROOT="$(dirname "$(dirname "$CC")")/$PREFIX/sysroot"
if [ ! -d "$BASE_SYSROOT" ]; then
    echo "error: toolchain sysroot not found at $BASE_SYSROOT" >&2
    exit 1
fi

echo "==> sysroot: $SYSROOT"
mkdir -p "$SYSROOT" "$WORK/debs" "$WORK/extract"

if [ ! -d "$BASE_SYSROOT/usr/include/stdio.h" ] && [ ! -f "$BASE_SYSROOT/usr/include/stdio.h" ]; then
    echo "    seeding from the toolchain sysroot"
    mkdir -p "$SYSROOT/usr"
    cp -R "$BASE_SYSROOT/usr/include" "$SYSROOT/usr/" 2>/dev/null || true
    mkdir -p "$SYSROOT/usr/lib/x86_64-linux-gnu"
    cp -a "$BASE_SYSROOT/usr/lib/x86_64-linux-gnu/." \
          "$SYSROOT/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true
fi

echo "==> package index"
cd "$WORK"
if [ ! -f Packages ]; then
    curl -sL -o Packages.gz "$MIRROR/dists/$SUITE/main/binary-amd64/Packages.gz"
    gunzip -f Packages.gz
fi
echo "    $(wc -l < Packages) lines"

# -dev packages carry headers and the unversioned .so symlinks; the runtime
# packages carry the .so.N targets those symlinks point at. Both are needed, or
# the linker either cannot find a header or cannot resolve a library.
DEV_PACKAGES=(
    libxcb1-dev libxcb-randr0-dev libxcb-shm0-dev libxcb-util-dev libxcb-xkb-dev
    libxcb-dri2-0-dev libxcb-dri3-dev libxcb-present-dev libxcb-sync-dev
    libxcb-render0-dev libxcb-image0-dev libxcb-keysyms1-dev libxcb-xinerama0-dev
    libx11-dev libx11-xcb-dev libxext-dev libxau-dev libxdmcp-dev libxrandr-dev
    libcairo2-dev libpixman-1-dev
    libwayland-dev wayland-protocols
    libegl-dev libgl-dev libglvnd-dev
    libxkbcommon-dev
)
RUNTIME_PACKAGES=(
    libxcb1 libxcb-randr0 libxcb-shm0 libxcb-util1 libxcb-xkb1 libxcb-image0
    libxcb-keysyms1 libxcb-render0 libxcb-dri2-0 libxcb-dri3-0 libxcb-present0
    libxcb-sync1 libxcb-xinerama0 libxau6 libxdmcp6 libx11-6 libx11-xcb1
    libxext6 libxrender1 libcairo2 libpixman-1-0 libfontconfig1 libfreetype6
    libpng16-16t64 libharfbuzz0b wayland-client0 libwayland-server0 libxkbcommon0
    libegl1 libegl-mesa0 libgl1 libglvnd0 libglapi-mesa
    libbz2-1.0 libxml2 libzstd1 liblzma5 libdeflate0 libffi8 libpcre2-8-0
    libbsd0 libmd0 libselinux1 libsystemd0 libcap2 liblz4-1 libexpat1 libz-1
)

fetch() {
    local pkg="$1"
    local file
    file="$(awk -v p="$pkg" '$1=="Package:"&&$2==p{f=1;next} f&&$1=="Filename:"{print $2;exit}' Packages)"
    if [ -z "$file" ]; then
        echo "    skip (not in $SUITE): $pkg" >&2
        return 0
    fi
    local base; base="$(basename "$file")"
    [ -f "debs/$base" ] || curl -sL -o "debs/$base" "$MIRROR/$file"
}

echo "==> downloading"
for p in "${DEV_PACKAGES[@]}" "${RUNTIME_PACKAGES[@]}"; do fetch "$p"; done
echo "    $(ls debs | wc -l | tr -d ' ') packages"

echo "==> extracting"
cd "$WORK/extract"
for d in ../debs/*.deb; do
    rm -f data.tar.* 2>/dev/null || true
    ar x "$d" 2>/dev/null || continue
    for t in data.tar.*; do
        [ -f "$t" ] && tar xf "$t" 2>/dev/null
    done
done

mkdir -p "$SYSROOT/usr/lib/x86_64-linux-gnu"
cp -R usr/include/. "$SYSROOT/usr/include/" 2>/dev/null || true
cp -a usr/lib/x86_64-linux-gnu/. "$SYSROOT/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true
cp -a lib/x86_64-linux-gnu/. "$SYSROOT/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true

echo "==> done"
echo "    headers: $(find "$SYSROOT/usr/include" -name '*.h' | wc -l | tr -d ' ')"
echo "    libs   : $(ls "$SYSROOT/usr/lib/x86_64-linux-gnu" | wc -l | tr -d ' ')"
echo
echo "Next: ./scripts/linux-check.sh --link"