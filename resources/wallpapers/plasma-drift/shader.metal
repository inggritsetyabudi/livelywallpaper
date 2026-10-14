// Plasma Drift — a GPU-rendered animated wallpaper.
//
// The native layer compiles this source directly and looks up `vertex_main`
// and `fragment_main`. Uniforms arrive in `ShaderUniforms` at buffer(0):
//   resolution (pixels), time (seconds), timeDelta, frame, mouse, audioBands.
//
// Deliberately small and deterministic: no texture sampling, no state beyond
// the uniforms, so it stays cheap enough to run at 60fps behind a desktop.

#include <metal_stdlib>
using namespace metal;

struct ShaderUniforms {
    float2 resolution;
    float  time;
    float  timeDelta;
    int    frame;
    float4 mouse;
    float  audioBands[128];
};

struct VertexOut {
    float4 position [[position]];
    float2 uv;
};

vertex VertexOut vertex_main(uint vid [[vertex_id]]) {
    // The host issues exactly one draw call:
    //     drawPrimitives(.triangleStrip, vertexStart: 0, vertexCount: 4)
    // so the vertex count and primitive type here are not free choices — a
    // three-vertex triangle would read past this array and render nothing.
    float2 positions[4] = {
        float2(-1.0, -1.0),
        float2( 1.0, -1.0),
        float2(-1.0,  1.0),
        float2( 1.0,  1.0),
    };

    VertexOut out;
    float2 pos = positions[vid];
    out.position = float4(pos, 0.0, 1.0);
    out.uv = float2((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    return out;
}

// Classic value-noise, fbm'd over a few octaves.
static float hash21(float2 p) {
    p = fract(p * float2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

static float value_noise(float2 p) {
    float2 i = floor(p);
    float2 f = fract(p);
    float2 u = f * f * (3.0 - 2.0 * f);

    float a = hash21(i);
    float b = hash21(i + float2(1.0, 0.0));
    float c = hash21(i + float2(0.0, 1.0));
    float d = hash21(i + float2(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

static float fbm(float2 p) {
    float sum = 0.0;
    float amplitude = 0.5;
    for (int i = 0; i < 5; i++) {
        sum += value_noise(p) * amplitude;
        p *= 2.02;
        amplitude *= 0.5;
    }
    return sum;
}

fragment float4 fragment_main(VertexOut in [[stage_in]],
                              constant ShaderUniforms &u [[buffer(0)]]) {
    float2 uv = in.uv;
    float t = u.time * 0.12;

    // Aspect-correct the coordinates so the pattern is not stretched on
    // ultrawide displays.
    float2 p = float2((uv.x - 0.5) * (u.resolution.x / u.resolution.y), uv.y - 0.5);

    // Warp the domain by the noise field, then sample it again: this is what
    // produces the slow, liquid motion rather than a sliding texture.
    float2 q = float2(fbm(p + float2(0.0, t)),
                      fbm(p + float2(5.2, 1.3 - t)));
    float2 r = float2(fbm(p + 3.4 * q + float2(1.7, 9.2) + 0.15 * t),
                      fbm(p + 3.4 * q + float2(8.3, 2.8) - 0.12 * t));

    float f = fbm(p + 3.6 * r);

    // Cool palette: deep indigo through violet into a warm highlight.
    float3 indigo = float3(0.05, 0.07, 0.22);
    float3 violet = float3(0.36, 0.18, 0.74);
    float3 cyan   = float3(0.06, 0.68, 0.76);
    float3 warm   = float3(0.99, 0.70, 0.38);

    // Shape the noise into usable ranges before colouring. fbm() clusters
    // around 0.5, so feeding it straight in flattens everything into a wash;
    // stretching it is what gives the image its contrast.
    float shaped = clamp((f - 0.44) * 1.9, 0.0, 1.0);

    float3 colour = mix(indigo, violet, shaped);
    colour = mix(colour, cyan, clamp(length(q) * 0.60 - 0.10, 0.0, 1.0));

    // Warm only in the brightest ridges, or it floods the whole frame.
    float highlight = clamp((r.x - 0.66) * 2.8, 0.0, 1.0);
    colour = mix(colour, warm, highlight * highlight * 0.6);

    // Vignette so the centre of the desktop stays readable.
    float2 centred = in.uv - 0.5;
    float vig = 1.0 - 0.55 * smoothstep(0.32, 0.86, length(centred));
    colour *= vig;

    return float4(colour, 1.0);
}