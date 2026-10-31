// Converts the finished composition (premultiplied, linear light) to display sRGB.

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
    // The composition background is opaque, so alpha is 1 wherever it matters.
    return vec4(to_srgb(c.rgb), 1.0);
}
