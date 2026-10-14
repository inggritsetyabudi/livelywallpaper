#!/usr/bin/env python3
"""Generate the still-image wallpapers that ship with the app.

Writing the PNGs by hand is impractical, and shipping opaque binaries makes them
impossible to tweak. This renders them from a small procedural description
instead, so a palette change is a one-line edit and re-run:

    ./scripts/make-wallpapers.py

Output lands in resources/wallpapers/<name>/, each a valid Lively project folder.
"""

from __future__ import annotations

import math
import pathlib
import struct
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "resources" / "wallpapers"


# --------------------------------------------------------------------------
# Minimal PNG writer
#
# The stdlib has no image encoder, and pulling in Pillow for a handful of
# gradients would add a build dependency for no benefit.
# --------------------------------------------------------------------------


def write_png(path: pathlib.Path, width: int, height: int, pixel) -> None:
    """Write an 8-bit RGB PNG.

    `pixel(x, y)` returns an (r, g, b) triple in 0..255.
    """
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter type 0 (None) for each scanline
        for x in range(width):
            r, g, b = pixel(x, y)
            raw += bytes((r & 255, g & 255, b & 255))

    def chunk(tag: bytes, data: bytes) -> bytes:
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


# --------------------------------------------------------------------------
# Colour helpers
# --------------------------------------------------------------------------


def mix(a, b, t: float):
    t = max(0.0, min(1.0, t))
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def clamp8(v: float) -> int:
    return max(0, min(255, int(round(v))))


# --------------------------------------------------------------------------
# Wallpaper definitions
# --------------------------------------------------------------------------


def mesh_gradient(width: int, height: int, points, base, tightness=140.0):
    """Smooth radial blend of several coloured points over `base`.

    Inverse-square falloff normalised into a weighted average. `tightness`
    controls locality: too low and every point contributes everywhere, which
    averages the palette down to a flat wash; high enough and each point owns a
    visible region that fades into its neighbours.
    """
    px = [(x, y, c) for x, y, c in points]

    def pixel(x, y):
        u = x / width
        v = y / height
        acc = [0.0, 0.0, 0.0]
        total = 0.0
        for cx, cy, colour in px:
            d2 = (u - cx) ** 2 + (v - cy) ** 2
            w = 1.0 / (d2 * tightness + 0.006)
            total += w
            for i in range(3):
                acc[i] += colour[i] * w

        if total <= 0:
            return tuple(clamp8(c) for c in base)

        out = [acc[i] / total for i in range(3)]
        # Pull the result toward the base so the darker areas stay deep instead
        # of washing out to the average of the palette.
        blended = mix(base, out, 0.88)
        return tuple(clamp8(c) for c in blended)

    return pixel


def diagonal_bands(width: int, height: int, stops):
    """Smooth vertical gradient with optional soft bands.

    `stops` is a list of (position 0..1, colour).
    """

    def pixel(x, y):
        t = (x / width * 0.35) + (y / height * 0.65)
        t = max(0.0, min(1.0, t))

        # Locate the surrounding stops and interpolate.
        lo = stops[0]
        hi = stops[-1]
        for i in range(len(stops) - 1):
            if stops[i][0] <= t <= stops[i + 1][0]:
                lo, hi = stops[i], stops[i + 1]
                break

        span = max(1e-6, hi[0] - lo[0])
        local = (t - lo[0]) / span
        # Smoothstep keeps the transition free of a visible crease.
        local = local * local * (3 - 2 * local)
        return tuple(clamp8(c) for c in mix(lo[1], hi[1], local))

    return pixel


def radial_glow(width: int, height: int, stops, centre=(0.5, 0.5), scale=1.0):
    """Radial gradient from the centre outward."""

    def pixel(x, y):
        u = (x / width - centre[0]) * 2 * scale
        v = (y / height - centre[1]) * 2 * scale
        t = min(1.0, math.sqrt(u * u + v * v))
        return tuple(clamp8(c) for c in mix(stops[0], stops[1], t))

    return pixel


# --------------------------------------------------------------------------
# Build
# --------------------------------------------------------------------------

# Large enough to look sharp on a 5K display, small enough to embed: ~380 KB
# raw per wallpaper before compression, a few tens of KB on disk.
FULL_W, FULL_H = 2560, 1440
THUMB_W, THUMB_H = 640, 360


def emit(name: str, maker) -> None:
    """Render a wallpaper and its thumbnail with the same generator."""
    folder = OUT / name
    folder.mkdir(parents=True, exist_ok=True)

    write_png(folder / "wallpaper.png", FULL_W, FULL_H, maker(FULL_W, FULL_H))
    write_png(folder / "thumb.png", THUMB_W, THUMB_H, maker(THUMB_W, THUMB_H))
    print(f"  {name}")


def main() -> None:
    print(f"Generating wallpapers into {OUT}")

    # 1. Midnight Mesh — deep blues and violet, the "calm default".
    emit(
        "midnight-mesh",
        lambda w, h: mesh_gradient(
            w,
            h,
            points=[
                (0.18, 0.22, (52, 62, 190)),
                (0.82, 0.14, (120, 48, 200)),
                (0.62, 0.78, (18, 132, 190)),
                (0.30, 0.90, (30, 30, 110)),
            ],
            base=(8, 10, 32),
        ),
    )

    # 2. Ember — warm sunset, the counterpart to Midnight Mesh.
    emit(
        "ember",
        lambda w, h: mesh_gradient(
            w,
            h,
            points=[
                (0.78, 0.80, (255, 122, 41)),
                (0.30, 0.62, (226, 60, 84)),
                (0.55, 0.18, (86, 32, 120)),
                (0.10, 0.12, (36, 18, 62)),
            ],
            base=(26, 12, 34),
        ),
    )

    # 3. Tidal — teal, soft top-down light like light through water.
    emit(
        "tidal",
        lambda w, h: diagonal_bands(
            w,
            h,
            stops=[
                (0.0, (8, 44, 62)),
                (0.45, (14, 116, 130)),
                (0.75, (46, 178, 168)),
                (1.0, (150, 226, 208)),
            ],
        ),
    )

    # 4. Graphite — near-monochrome, for people who want their desktop calm.
    emit(
        "graphite",
        lambda w, h: radial_glow(
            w,
            h,
            stops=[(46, 50, 58), (14, 15, 18)],
            centre=(0.34, 0.28),
            scale=1.1,
        ),
    )

    print("Done.")


if __name__ == "__main__":
    main()