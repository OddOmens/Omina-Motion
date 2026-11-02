// Converts the finished composition (premultiplied, linear light) to display sRGB with
// straight alpha, so transparent parts of a composition stay transparent.

@group(0) @binding(0) var frame_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOut;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    return out;
}

fn to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3(0.0), vec3(1.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let c = textureLoad(frame_tex, vec2<i32>(in.position.xy), 0);
    // Light added without coverage (a glow's halo over transparency) has more color than alpha.
    // Straight alpha cannot hold that, so such a pixel gets just enough alpha to carry its light:
    // over black it looks the same, instead of the light being thrown away with zero alpha.
    let alpha = min(max(c.a, max(c.r, max(c.g, c.b))), 1.0);
    let straight = select(c.rgb / alpha, vec3(0.0), alpha <= 0.0001);
    return vec4(to_srgb(straight), alpha);
}
