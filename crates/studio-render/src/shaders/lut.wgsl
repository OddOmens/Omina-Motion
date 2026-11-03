// Apply LUT: looks each pixel up in a 3D lookup table. Colors arrive premultiplied in linear
// light; the table is indexed by display (sRGB-encoded) values, as `.cube` files expect, and the
// result is mixed with the original by the effect's intensity.

struct LutUniform {
    // x: intensity (0-1), y: samples per side of the table, z-w: unused.
    info: vec4<f32>,
    // The input range the table covers, per channel (w unused).
    domain_min: vec4<f32>,
    domain_max: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: LutUniform;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var table: texture_3d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOut;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn to_display(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, linear_sampler, in.uv, 0.0);
    if (c.a <= 0.0001) {
        return c;
    }
    let rgb = c.rgb / c.a;
    let span = max(u.domain_max.xyz - u.domain_min.xyz, vec3(1e-6));
    let place = clamp((to_display(rgb) - u.domain_min.xyz) / span, vec3(0.0), vec3(1.0));
    // Sample centers sit half a texel in from each edge, so 0 and 1 land exactly on the end samples.
    let size = u.info.y;
    let coord = (place * (size - 1.0) + 0.5) / size;
    let graded = to_linear(textureSampleLevel(table, linear_sampler, coord, 0.0).rgb);
    let mixed = mix(rgb, graded, clamp(u.info.x, 0.0, 1.0));
    return vec4(max(mixed, vec3(0.0)) * c.a, c.a);
}
