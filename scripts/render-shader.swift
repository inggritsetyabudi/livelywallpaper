#!/usr/bin/env swift
//
// Compiles a wallpaper's Metal shader and renders one frame to a PNG.
//
// Two jobs in one:
//   1. the thumbnail matches what the wallpaper actually looks like, instead of
//      being a hand-waved approximation
//   2. a shader that does not compile fails at build time rather than at runtime,
//      where the native layer would silently fall back to a default fragment
//
//   ./scripts/render-shader.swift <shader.metal> <out.png> [width] [height]

import AppKit
import Foundation
import Metal

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write(
        "usage: render-shader.swift <shader.metal> <out.png> [width] [height]\n"
            .data(using: .utf8)!
    )
    exit(2)
}

let shaderPath = args[1]
let outPath = args[2]
let width = args.count > 3 ? Int(args[3]) ?? 640 : 640
let height = args.count > 4 ? Int(args[4]) ?? 360 : 360

let source = try String(contentsOfFile: shaderPath, encoding: .utf8)

guard let device = MTLCreateSystemDefaultDevice() else {
    FileHandle.standardError.write("no Metal device available\n".data(using: .utf8)!)
    exit(1)
}

// Compiling here is the real check: if this throws, the shader is broken.
let library: MTLLibrary
do {
    library = try device.makeLibrary(source: source, options: nil)
} catch {
    FileHandle.standardError.write(
        "shader failed to compile in \(shaderPath):\n\(error)\n".data(using: .utf8)!
    )
    exit(1)
}

guard
    let vertex = library.makeFunction(name: "vertex_main"),
    let fragment = library.makeFunction(name: "fragment_main")
else {
    FileHandle.standardError.write(
        "shader must define `vertex_main` and `fragment_main`\n".data(using: .utf8)!
    )
    exit(1)
}

let descriptor = MTLRenderPipelineDescriptor()
descriptor.vertexFunction = vertex
descriptor.fragmentFunction = fragment
descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm

let pipeline: MTLRenderPipelineState
do {
    pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
} catch {
    FileHandle.standardError.write("pipeline creation failed: \(error)\n".data(using: .utf8)!)
    exit(1)
}

// Mirror of the native layer's uniform struct. Field order matters: it is
// shared memory layout, not a Swift struct.
struct ShaderUniforms {
    var resolution = SIMD2<Float>(Float(width), Float(height))
    var time: Float = 0
    var timeDelta: Float = 1.0 / 60.0
    var frame: Int32 = 0
    var mouse = SIMD4<Float>(0.5, 0.5, 0, 0)
    var audioBands = [Float](repeating: 0, count: 128)
}

// A single instant: enough to produce a representative thumbnail. Advancing
// the animation would need a real draw loop, which the running wallpaper has.
var uniforms = ShaderUniforms()

guard let queue = device.makeCommandQueue() else {
    FileHandle.standardError.write("could not create a command queue\n".data(using: .utf8)!)
    exit(1)
}

guard let texture = device.makeTexture(
    descriptor: MTLTextureDescriptor.texture2DDescriptor(
        pixelFormat: .bgra8Unorm,
        width: width,
        height: height,
        mipmapped: false
    )
) else {
    FileHandle.standardError.write("could not allocate the render target\n".data(using: .utf8)!)
    exit(1)
}

// The render target belongs on the pass descriptor. This SDK's
// MTLRenderCommandEncoder has no setRenderTarget, matching how the native
// renderer drives MTKView.
let passDescriptor = MTLRenderPassDescriptor()
passDescriptor.colorAttachments[0].texture = texture
passDescriptor.colorAttachments[0].loadAction = .clear
passDescriptor.colorAttachments[0].storeAction = .store
passDescriptor.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 1)

guard let commandBuffer = queue.makeCommandBuffer(),
      let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: passDescriptor)
else {
    FileHandle.standardError.write("could not begin a render pass\n".data(using: .utf8)!)
    exit(1)
}

encoder.setRenderPipelineState(pipeline)
encoder.setFragmentBytes(&uniforms, length: MemoryLayout<ShaderUniforms>.stride, index: 0)
// Must match what the native layer issues: a triangle strip of four vertices.
encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
encoder.endEncoding()
commandBuffer.commit()
commandBuffer.waitUntilCompleted()

let bytesPerRow = (width * 4 + 63) / 64 * 64
var bgra = [UInt8](repeating: 0, count: bytesPerRow * height)
bgra.withUnsafeMutableBytes { raw in
    guard let base = raw.baseAddress else { return }
    texture.getBytes(
        base,
        bytesPerRow: bytesPerRow,
        from: MTLRegionMake2D(0, 0, width, height),
        mipmapLevel: 0
    )
}

// The render target is BGRA — the layout Metal prefers for textures the GPU
// writes. NSBitmapImageRep below expects RGBA, so swap here. Skipping this
// silently transposes red and blue, which turns every violet into brown.
var pixels = [UInt8](repeating: 0, count: bytesPerRow * height)
for y in 0..<height {
    let rowStart = y * bytesPerRow
    for x in 0..<width {
        let src = rowStart + x * 4
        let dst = rowStart + x * 4
        pixels[dst + 0] = bgra[src + 2]  // R <- B
        pixels[dst + 1] = bgra[src + 1]  // G
        pixels[dst + 2] = bgra[src + 0]  // B <- R
        pixels[dst + 3] = bgra[src + 3]  // A
    }
}

// NSBitmapImageRep wants the address of an optional plane pointer.
var bitmap: NSBitmapImageRep?
pixels.withUnsafeMutableBytes { (raw: UnsafeMutableRawBufferPointer) in
    guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
    var plane: UnsafeMutablePointer<UInt8>? = base
    bitmap = NSBitmapImageRep(
        bitmapDataPlanes: &plane,
        pixelsWide: width,
        pixelsHigh: height,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: bytesPerRow,
        bitsPerPixel: 32
    )
}

guard let image = bitmap,
      let data = image.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write("could not encode the PNG\n".data(using: .utf8)!)
    exit(1)
}

try data.write(to: URL(fileURLWithPath: outPath))
print("rendered \(shaderPath) -> \(outPath) (\(width)x\(height))")