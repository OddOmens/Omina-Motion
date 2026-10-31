// Draws one layer's source rectangle into a composition-sized texture.
// The vertex shader maps source pixels to clip space with the layer's world transform;
// the fragment shader fills it with a solid color, a texture (text, images) or a
// "no media yet" checker. Output is premultiplied, linear light.

struct LayerUniform {
    // Source pixels -> clip space: clip = (a*x + c*y + tx, b*x + d*y + ty).
    abcd: vec4<f32>,
    // tx, ty, source width, source height.
    translate_size: vec4<f32>,
    // Premultiplied linear color for solids and the placeholder tint.
    color: vec4<f32>,
    // x: mode (0 solid, 1 texture, 2 placeholder), y: checker cell size in source pixels.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: LayerUniform;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // Two triangles covering the unit square.
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let uv = corners[index];
    let local = uv * u.translate_size.zw;
    let clip = vec2(
        u.abcd.x * local.x + u.abcd.z * local.y + u.translate_size.x,
        u.abcd.y * local.x + u.abcd.w * local.y + u.translate_size.y,
    );
    var out: VertexOut;
    out.position = vec4(clip, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let mode = u32(u.params.x);
    if (mode == 1u) {
        // Textures hold premultiplied colors, sRGB encoded; the sRGB format linearizes on
        // sampling, so filtering already happened on premultiplied values.
        return textureSample(source, source_sampler, in.uv);
    }
    if (mode == 2u) {
        // Media that is not decoded yet: a checker in the layer's label color.
        let cell = max(u.params.y, 1.0);
        let pixel = in.uv * u.translate_size.zw;
        let checker = (u32(floor(pixel.x / cell)) + u32(floor(pixel.y / cell))) % 2u;
        let shade = select(0.55, 0.35, checker == 1u);
        return vec4(u.color.rgb * shade, 1.0);
    }
    return u.color;
}
