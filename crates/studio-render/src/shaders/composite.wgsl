// Composites one rendered layer onto everything below it (the backdrop), with the
// layer's opacity, blend mode and track matte. All colors are premultiplied, linear.
// Adjustment layers use kind 1: the "layer" texture holds the backdrop with the
// adjustment's effects applied, and the matte holds the adjustment layer's shape.

struct CompositeUniform {
    // x: blend mode, y: matte mode, z: kind (0 layer, 1 adjustment), w: unused.
    modes: vec4<u32>,
    // x: opacity 0-1.
    values: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: CompositeUniform;
@group(0) @binding(1) var backdrop_tex: texture_2d<f32>;
@group(0) @binding(2) var layer_tex: texture_2d<f32>;
@group(0) @binding(3) var matte_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // One triangle that covers the whole target.
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOut;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    return out;
}

const LUMA = vec3(0.2126, 0.7152, 0.0722);

fn unpremultiply(c: vec4<f32>) -> vec3<f32> {
    // Pure light (color with no coverage, such as glow) has no straight color; treat it as none.
    return select(c.rgb / c.a, vec3(0.0), c.a <= 0.0001);
}

// Separable blend functions B(backdrop, source), on straight colors (W3C compositing spec).
fn blend(mode: u32, cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 2u: { return cb * cs; }                                   // Multiply
        case 3u: { return cb + cs - cb * cs; }                         // Screen
        case 4u: { return hard_light(cs, cb); }                        // Overlay = hard light, swapped
        case 5u: { return soft_light(cb, cs); }                        // Soft Light
        case 6u: { return hard_light(cb, cs); }                        // Hard Light
        case 7u: { return abs(cb - cs); }                              // Difference
        default: { return cs; }                                        // Normal
    }
}

fn hard_light(cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    let multiply = cb * 2.0 * cs;
    let screen = cb + (2.0 * cs - 1.0) - cb * (2.0 * cs - 1.0);
    return select(screen, multiply, cs <= vec3(0.5));
}

fn soft_light(cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    let d = select(sqrt(cb), ((16.0 * cb - 12.0) * cb + 4.0) * cb, cb <= vec3(0.25));
    let darker = cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb);
    let lighter = cb + (2.0 * cs - 1.0) * (d - cb);
    return select(lighter, darker, cs <= vec3(0.5));
}

// How much of the layer the track matte lets through.
fn matte_amount(mode: u32, matte: vec4<f32>) -> f32 {
    let luma = dot(matte.rgb, LUMA);
    switch mode {
        case 1u: { return matte.a; }          // Alpha
        case 2u: { return 1.0 - matte.a; }    // Alpha inverted
        case 3u: { return luma; }             // Luma (premultiplied, so transparent areas are dark)
        case 4u: { return 1.0 - luma; }       // Luma inverted
        default: { return 1.0; }
    }
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let b = textureLoad(backdrop_tex, pixel, 0);
    let layer = textureLoad(layer_tex, pixel, 0);
    let matte = textureLoad(matte_tex, pixel, 0);
    let opacity = u.values.x;

    if (u.modes.z == 1u) {
        // Adjustment layer: blend the adjusted backdrop in where the layer covers.
        return mix(b, layer, clamp(matte.a * opacity, 0.0, 1.0));
    }

    // Coverage can never exceed 1; anything else would subtract the backdrop.
    let s = vec4(layer.rgb, clamp(layer.a, 0.0, 1.0)) * opacity * matte_amount(u.modes.y, matte);
    if (u.modes.x == 1u) {
        // Add: light adds up.
        return vec4(b.rgb + s.rgb, s.a + b.a * (1.0 - s.a));
    }
    if (u.modes.x == 0u) {
        return s + b * (1.0 - s.a);
    }
    let mixed = blend(u.modes.x, unpremultiply(b), unpremultiply(s));
    let rgb = s.rgb * (1.0 - b.a) + b.rgb * (1.0 - s.a) + s.a * b.a * mixed;
    return vec4(rgb, s.a + b.a * (1.0 - s.a));
}
