#!/usr/bin/env swift
//
// Generates the application icon.
//
// The icon is drawn programmatically rather than shipped as binary art so it
// stays crisp at every size and can be re-derived when the palette changes.
// Run it after editing the design:
//
//     ./scripts/make-icon.swift
//
// It writes assets/AppIcon.iconset/ and then produces assets/AppIcon.icns.

import AppKit
import Foundation

// MARK: - Design tokens

/// Brand palette. Violet into cyan reads as "moving light" at any size and
/// stays legible once it is down-sampled to a 16px menubar-adjacent icon.
struct Palette {
    static let topLeft = NSColor(srgbRed: 0.36, green: 0.19, blue: 0.92, alpha: 1)
    static let bottomRight = NSColor(srgbRed: 0.00, green: 0.68, blue: 0.90, alpha: 1)
    static let deep = NSColor(srgbRed: 0.12, green: 0.05, blue: 0.36, alpha: 1)
    static let glyph = NSColor.white
}

/// macOS icons use a superellipse ("squircle"), not a circular-corner rect.
/// `n` approximates the macOS shape; the value Apple uses is ~5.
let squircleExponent = 5.0

/// Fraction of the canvas the glyph occupies.
///
/// Kept modest on purpose: at 16px a larger triangle bleeds into the squircle
/// edge and the icon turns into an unreadable blob.
let glyphScale = 0.40

/// Horizontal offset of the glyph centre, as a fraction of the canvas.
///
/// A triangle's area sits left of its bounding box, so mathematically centred
/// looks visibly off. This nudges it right until it reads as centred.
let glyphOffsetX = 0.545

// MARK: - Geometry

/// Superellipse path centred on the canvas.
///
/// The regular rounded-rectangle that most drawing APIs give you reads as
/// "generic app" at a glance; the squircle is what makes an icon look native.
func squirclePath(in rect: CGRect, inset: CGFloat = 0) -> CGPath {
    let r = rect.insetBy(dx: inset, dy: inset)
    let cx = r.midX
    let cy = r.midY
    // Half-extent, slightly reduced so the shape never touches the canvas edge.
    let a = r.width / 2 * 0.94
    let b = r.height / 2 * 0.94

    let path = CGMutablePath()
    let steps = 512
    for i in 0...steps {
        let t = (Double(i) / Double(steps)) * 2 * Double.pi
        let ct = cos(t)
        let st = sin(t)
        let x = cx + a * CGFloat(pow(abs(ct), 2 / squircleExponent) * sign(ct))
        let y = cy + b * CGFloat(pow(abs(st), 2 / squircleExponent) * sign(st))
        if i == 0 {
            path.move(to: CGPoint(x: x, y: y))
        } else {
            path.addLine(to: CGPoint(x: x, y: y))
        }
    }
    path.closeSubpath()
    return path
}

private func sign(_ v: Double) -> Double { v < 0 ? -1 : 1 }

/// A play triangle with softened corners.
///
/// A raw triangle looks harsh at large sizes and loses its point at 16px, so
/// the corners are filleted and the whole glyph is drawn slightly oversized.
func playPath(center: CGPoint, size: CGFloat) -> CGPath {
    let h = size
    let w = size * 0.88
    let left = center.x - w / 2
    let top = center.y + h / 2
    let bottom = center.y - h / 2
    let tipX = left + w
    let radius = size * 0.16

    let path = CGMutablePath()
    // Start just after the bottom-left fillet.
    path.move(to: CGPoint(x: left + radius, y: bottom))
    path.addLine(to: CGPoint(x: tipX - radius, y: center.y))
    path.addLine(to: CGPoint(x: left + radius, y: top))
    path.addLine(to: CGPoint(x: left, y: top - radius))
    path.addLine(to: CGPoint(x: left, y: bottom + radius))
    path.addCurve(
        to: CGPoint(x: left + radius, y: bottom),
        control1: CGPoint(x: left, y: bottom + radius * 0.45),
        control2: CGPoint(x: left + radius * 0.45, y: bottom)
    )
    path.closeSubpath()
    return path
}

// MARK: - Drawing

func drawIcon(size: CGFloat) -> Data? {
    let side = Int(size)
    guard let context = CGContext(
        data: nil,
        width: side,
        height: side,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }

    let canvas = CGRect(x: 0, y: 0, width: size, height: size)
    context.setAllowsAntialiasing(true)
    context.interpolationQuality = .high

    // Keep the icon slightly inset from the canvas edge, the way macOS does, so
    // it does not touch the menu bar edge or a Dock divider.
    let body = canvas.insetBy(dx: size * 0.055, dy: size * 0.055)

    context.saveGState()
    context.addPath(squirclePath(in: body))
    context.clip()

    // Diagonal gradient plus a soft highlight in the upper left.
    let gradient = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: [
            Palette.topLeft.cgColor,
            Palette.bottomRight.cgColor,
            Palette.deep.cgColor,
        ] as CFArray,
        locations: [0.0, 0.55, 1.0]
    )!
    context.drawLinearGradient(
        gradient,
        start: CGPoint(x: body.minX, y: body.maxY),
        end: CGPoint(x: body.maxX, y: body.minY),
        options: []
    )

    // Radial highlight so the surface reads as lit rather than flat.
    let highlight = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: [
            NSColor.white.withAlphaComponent(0.38).cgColor,
            NSColor.white.withAlphaComponent(0.0).cgColor,
        ] as CFArray,
        locations: [0.0, 1.0]
    )!
    context.drawRadialGradient(
        highlight,
        startCenter: CGPoint(x: body.minX + body.width * 0.28, y: body.maxY - body.height * 0.24),
        startRadius: 0,
        endCenter: CGPoint(x: body.minX + body.width * 0.28, y: body.maxY - body.height * 0.24),
        endRadius: body.width * 0.72,
        options: []
    )

    // Glyph, nudged right so it looks optically centred rather than
    // mathematically centred — a triangle's mass sits left of its bounds.
    let glyphSize = size * glyphScale
    let glyphCenter = CGPoint(x: size * glyphOffsetX, y: size * 0.5)

    context.saveGState()
    context.setShadow(
        offset: CGSize(width: 0, height: -size * 0.012),
        blur: size * 0.05,
        color: NSColor.black.withAlphaComponent(0.35).cgColor
    )
    context.setFillColor(Palette.glyph.cgColor)
    context.addPath(playPath(center: glyphCenter, size: glyphSize))
    context.fillPath()
    context.restoreGState()

    // Darken the rim. Without it the white glyph sits on a mid-bright cyan at
    // the bottom right and the edge contrast collapses once downsampled.
    let vignette = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: [
            NSColor.black.withAlphaComponent(0.0).cgColor,
            NSColor.black.withAlphaComponent(0.28).cgColor,
        ] as CFArray,
        locations: [0.55, 1.0]
    )!
    context.drawRadialGradient(
        vignette,
        startCenter: CGPoint(x: body.midX, y: body.midY),
        startRadius: 0,
        endCenter: CGPoint(x: body.midX, y: body.midY),
        endRadius: max(body.width, body.height) * 0.62,
        options: []
    )

    context.restoreGState()

    // A hairline inner edge gives the icon a defined boundary against any
    // desktop wallpaper, including a light one.
    context.saveGState()
    context.addPath(squirclePath(in: body))
    context.setStrokeColor(NSColor.white.withAlphaComponent(0.22).cgColor)
    context.setLineWidth(max(1, size * 0.006))
    context.strokePath()
    context.restoreGState()

    guard let image = context.makeImage() else { return nil }
    let rep = NSBitmapImageRep(cgImage: image)
    rep.size = NSSize(width: size, height: size)
    return rep.representation(using: .png, properties: [:])
}

// MARK: - Output

let scriptURL = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
let repoRoot = scriptURL.deletingLastPathComponent().deletingLastPathComponent()
let assets = repoRoot.appendingPathComponent("assets")
let iconset = assets.appendingPathComponent("AppIcon.iconset")

try? FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

// The set iconutil needs for a complete .icns.
let variants: [(name: String, points: Int)] = [
    ("icon_16x16", 16), ("icon_16x16@2x", 32),
    ("icon_32x32", 32), ("icon_32x32@2x", 64),
    ("icon_128x128", 128), ("icon_128x128@2x", 256),
    ("icon_256x256", 256), ("icon_256x256@2x", 512),
    ("icon_512x512", 512), ("icon_512x512@2x", 1024),
]

for variant in variants {
    guard let data = drawIcon(size: CGFloat(variant.points)) else {
        FileHandle.standardError.write("failed to draw \(variant.name)\n".data(using: .utf8)!)
        exit(1)
    }
    let url = iconset.appendingPathComponent("\(variant.name).png")
    try data.write(to: url)
}

// `iconutil` does not guess: it needs an Info.plist saying which file is which
// size. This is the canonical macOS icon set.
struct IconEntry: Encodable {
    let size: Int
    let idiom: String
    let filename: String
    let scale: String
}
struct IconSetPlist: Encodable {
    let images: [IconEntry]
}

let sizes: [(Int, String, String)] = [
    (16, "icon_16x16.png", "1x"),
    (16, "icon_16x16@2x.png", "2x"),
    (32, "icon_32x32.png", "1x"),
    (32, "icon_32x32@2x.png", "2x"),
    (128, "icon_128x128.png", "1x"),
    (128, "icon_128x128@2x.png", "2x"),
    (256, "icon_256x256.png", "1x"),
    (256, "icon_256x256@2x.png", "2x"),
    (512, "icon_512x512.png", "1x"),
    (512, "icon_512x512@2x.png", "2x"),
]

let canonical = IconSetPlist(
    images: sizes.map { IconEntry(size: $0.0, idiom: "mac", filename: $0.1, scale: $0.2) }
)

let encoder = PropertyListEncoder()
encoder.outputFormat = .xml
try encoder.encode(canonical).write(to: iconset.appendingPathComponent("icon.iconset.plist"))

let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconset.path, "-o", assets.appendingPathComponent("AppIcon.icns").path]
try process.run()
process.waitUntilExit()

guard process.terminationStatus == 0 else {
    FileHandle.standardError.write("iconutil failed\n".data(using: .utf8)!)
    exit(1)
}

print("wrote \(iconset.path) and assets/AppIcon.icns")