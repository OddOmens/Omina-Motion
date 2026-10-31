// Layer effects. Each pass reads `src` (and `aux` for passes that combine two images)
// and writes a new image of the same size. Colors are premultiplied, linear light.
// Lengths in the parameters are already converted to output pixels by the renderer.

struct EffectUniform {
    // x: effect id (see `effects.rs`), y-w: unused.
    id: vec4<u32>,
    // Up to eight parameters.
    p0: vec4<f32>,
    p1: vec4<f32>,
    // x, y: output size in pixels; z: time in seconds; w: unused.
    info: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: EffectUniform;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var aux: texture_2d<f32>;
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

const LUMA = vec3(0.2126, 0.7152, 0.0722);
const PI = 3.14159265;

fn sample(uv: vec2<f32>) -> vec4<f32> {
    // Outside the frame is transparent, so distortions pull in nothing rather than smeared edges.
    if (any(uv < vec2(0.0)) || any(uv > vec2(1.0))) {
        return vec4(0.0);
    }
    return textureSampleLevel(src, linear_sampler, uv, 0.0);
}

fn straight(c: vec4<f32>) -> vec3<f32> {
    return select(c.rgb / c.a, vec3(0.0), c.a <= 0.0001);
}

fn with_alpha(rgb: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4(max(rgb, vec3(0.0)) * a, a);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2(1.0, 0.0));
    let c = hash(i + vec2(0.0, 1.0));
    let d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var total = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var octave = 0; octave < 6; octave++) {
        total += amplitude * value_noise(q);
        q = q * 2.03 + vec2(17.0, 9.0);
        amplitude *= 0.5;
    }
    return total;
}

fn rgb_to_hsl(c: vec3<f32>) -> vec3<f32> {
    let high = max(c.r, max(c.g, c.b));
    let low = min(c.r, min(c.g, c.b));
    let l = (high + low) * 0.5;
    if (high == low) {
        return vec3(0.0, 0.0, l);
    }
    let d = high - low;
    let s = select(d / (high + low), d / (2.0 - high - low), l > 0.5);
    var h: f32;
    if (high == c.r) {
        h = (c.g - c.b) / d + select(0.0, 6.0, c.g < c.b);
    } else if (high == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3(h / 6.0, s, l);
}

fn hue_channel(p: f32, q: f32, t_in: f32) -> f32 {
    var t = fract(t_in);
    if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
    if (t < 0.5) { return q; }
    if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    if (hsl.y == 0.0) {
        return vec3(hsl.z);
    }
    let q = select(hsl.z + hsl.y - hsl.z * hsl.y, hsl.z * (1.0 + hsl.y), hsl.z < 0.5);
    let p = 2.0 * hsl.z - q;
    return vec3(hue_channel(p, q, hsl.x + 1.0 / 3.0), hue_channel(p, q, hsl.x), hue_channel(p, q, hsl.x - 1.0 / 3.0));
}

// One direction of a separable Gaussian blur. p0.x = sigma (px), p0.yz = direction.
fn gaussian(uv: vec2<f32>) -> vec4<f32> {
    let sigma = u.p0.x;
    if (sigma < 0.3) {
        return sample(uv);
    }
    let texel = 1.0 / u.info.xy;
    let direction = u.p0.yz * texel;
    // At most 64 taps each side; wide blurs space their taps out.
    let reach = ceil(sigma * 3.0);
    let step = max(1.0, reach / 64.0);
    var total = vec4(0.0);
    var weight_sum = 0.0;
    for (var offset = -reach; offset <= reach; offset += step) {
        let weight = exp(-(offset * offset) / (2.0 * sigma * sigma));
        total += textureSampleLevel(src, linear_sampler, uv + direction * offset, 0.0) * weight;
        weight_sum += weight;
    }
    return total / weight_sum;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let c = sample(uv);
    let rgb = straight(c);
    let size = u.info.xy;
    let pixel = uv * size;
    let center = vec2(0.5);

    switch u.id.x {
        // Gaussian blur, one direction.
        case 1u: { return gaussian(uv); }
        // Directional blur: p0.x angle (deg), p0.y length (px).
        case 2u: {
            let angle = radians(u.p0.x);
            let along = vec2(sin(angle), -cos(angle)) * u.p0.y / size;
            var total = vec4(0.0);
            for (var i = 0; i < 32; i++) {
                total += sample(uv + along * (f32(i) / 31.0 - 0.5));
            }
            return total / 32.0;
        }
        // Radial (zoom) blur from the center: p0.x amount.
        case 3u: {
            let strength = u.p0.x / 200.0;
            var total = vec4(0.0);
            for (var i = 0; i < 32; i++) {
                let t = 1.0 - strength * f32(i) / 31.0;
                total += sample(center + (uv - center) * t);
            }
            return total / 32.0;
        }
        // Sharpen: p0.x amount.
        case 4u: {
            let texel = 1.0 / size;
            let around = (sample(uv + vec2(texel.x, 0.0)) + sample(uv - vec2(texel.x, 0.0)) + sample(uv + vec2(0.0, texel.y)) + sample(uv - vec2(0.0, texel.y))) * 0.25;
            return max(c + (c - around) * (u.p0.x / 50.0), vec4(0.0));
        }
        // Brightness & Contrast: p0.x brightness, p0.y contrast.
        case 5u: {
            let adjusted = (rgb - 0.5) * (1.0 + u.p0.y / 100.0) + 0.5 + u.p0.x / 255.0;
            return with_alpha(adjusted, c.a);
        }
        // Color Balance: p0.xyz shadows, midtones, highlights.
        case 6u: {
            let l = dot(rgb, LUMA);
            let lift = u.p0.x / 200.0 * (1.0 - l) * (1.0 - l) + u.p0.y / 200.0 * 4.0 * l * (1.0 - l) + u.p0.z / 200.0 * l * l;
            return with_alpha(rgb + lift, c.a);
        }
        // Exposure: p0.x stops.
        case 7u: { return vec4(c.rgb * exp2(u.p0.x), c.a); }
        // Hue/Saturation: p0.x hue (deg), p0.y saturation, p0.z lightness.
        case 8u: {
            var hsl = rgb_to_hsl(clamp(rgb, vec3(0.0), vec3(1.0)));
            hsl.x = fract(hsl.x + u.p0.x / 360.0);
            hsl.y = clamp(hsl.y * (1.0 + u.p0.y / 100.0), 0.0, 1.0);
            hsl.z = clamp(hsl.z + u.p0.z / 200.0, 0.0, 1.0);
            return with_alpha(hsl_to_rgb(hsl), c.a);
        }
        // Tint: p0.x amount (%), mapping black to black and white to white.
        case 9u: { return with_alpha(mix(rgb, vec3(dot(rgb, LUMA)), u.p0.x / 100.0), c.a); }
        // Vibrance: p0.x vibrance, p0.y saturation.
        case 10u: {
            let l = dot(rgb, LUMA);
            let saturation = max(rgb.r, max(rgb.g, rgb.b)) - min(rgb.r, min(rgb.g, rgb.b));
            let boost = 1.0 + u.p0.y / 100.0 + u.p0.x / 100.0 * (1.0 - saturation);
            return with_alpha(mix(vec3(l), rgb, boost), c.a);
        }
        // Bulge: p0.x height, p0.y radius (px), around the center.
        case 11u: {
            let offset = pixel - size * 0.5;
            let distance = length(offset);
            let radius = max(u.p0.y, 1.0);
            if (distance >= radius) {
                return c;
            }
            let t = distance / radius;
            let pull = pow(t, 1.0 + u.p0.x * 0.5 * (1.0 - t));
            return sample((size * 0.5 + offset * select(pull / t, 1.0, t == 0.0)) / size);
        }
        // Wave Warp: p0.x height (px), p0.y width (px), p0.z speed (cycles per second).
        case 12u: {
            let phase = pixel.x / max(u.p0.y, 1.0) * 2.0 * PI + u.info.z * u.p0.z * 2.0 * PI;
            return sample(uv + vec2(0.0, sin(phase) * u.p0.x / size.y));
        }
        // Turbulent Displace: p0.x amount (px), p0.y size (px).
        case 13u: {
            let p = pixel / max(u.p0.y, 1.0);
            let shift = vec2(fbm(p) - 0.5, fbm(p + vec2(31.7, 11.3)) - 0.5) * 2.0 * u.p0.x;
            return sample(uv + shift / size);
        }
        // Fill: p0.x opacity (%), white.
        case 14u: { return with_alpha(mix(rgb, vec3(1.0), u.p0.x / 100.0), c.a); }
        // Gradient Ramp: black at the top to white at the bottom; p0.x scatter.
        case 15u: {
            let scatter = (hash(pixel) - 0.5) * u.p0.x / 256.0;
            return with_alpha(vec3(clamp(uv.y + scatter, 0.0, 1.0)), c.a);
        }
        // Fractal Noise: p0.x contrast, p0.y brightness, p0.z evolution (deg).
        case 16u: {
            let n = fbm(pixel / 120.0 + vec2(u.p0.z / 90.0, u.p0.z / 140.0));
            let v = clamp((n - 0.5) * (u.p0.x / 100.0) + 0.5 + u.p0.y / 100.0, 0.0, 1.0);
            return with_alpha(vec3(v), c.a);
        }
        // Add Grain: p0.x intensity, p0.y size.
        case 17u: {
            let cell = floor(pixel / max(u.p0.y, 0.5));
            let grain = (hash(cell + vec2(u.info.z * 97.0, u.info.z * 53.0)) - 0.5) * 0.2 * u.p0.x;
            return with_alpha(rgb + grain, c.a);
        }
        // Mosaic: p0.x horizontal blocks, p0.y vertical blocks.
        case 18u: {
            let blocks = max(u.p0.xy, vec2(1.0));
            return sample((floor(uv * blocks) + 0.5) / blocks);
        }
        // Posterize: p0.x levels.
        case 19u: {
            let levels = max(u.p0.x, 2.0) - 1.0;
            return with_alpha(floor(rgb * levels + 0.5) / levels, c.a);
        }
        // Linear Wipe: p0.x completion (%), p0.y angle (deg), p0.z feather (px).
        case 20u: {
            let angle = radians(u.p0.y);
            let direction = vec2(sin(angle), -cos(angle));
            let extent = abs(direction.x) * size.x + abs(direction.y) * size.y;
            let position = dot(pixel - size * 0.5, direction) / extent + 0.5;
            let feather = max(u.p0.z, 0.5) / extent;
            let edge = u.p0.x / 100.0 * (1.0 + feather) - feather * 0.5;
            return c * smoothstep(edge - feather * 0.5, edge + feather * 0.5, position);
        }
        // Radial Wipe: p0.x completion (%), p0.y start angle (deg), p0.z feather (deg).
        case 21u: {
            let offset = pixel - size * 0.5;
            let angle = fract((atan2(offset.x, -offset.y) - radians(u.p0.y)) / (2.0 * PI));
            let feather = max(u.p0.z, 0.1) / 360.0;
            return c * smoothstep(u.p0.x / 100.0 - feather, u.p0.x / 100.0, angle);
        }
        // Block Dissolve: p0.x completion (%), p0.y block size (px).
        case 22u: {
            let cell = floor(pixel / max(u.p0.y, 1.0));
            return c * step(u.p0.x / 100.0, hash(cell));
        }
        // Glow, bright pass: the light brighter than p0.x (0-1). Glow is light, not coverage, so its
        // alpha is how bright it is, never the layer's own alpha.
        case 30u: {
            let light = max(rgb - u.p0.x, vec3(0.0)) / max(1.0 - u.p0.x, 0.01) * c.a;
            return vec4(light, max(light.r, max(light.g, light.b)));
        }
        // Glow, combine: add the blurred light (aux) × p0.x intensity. Glow is pure light: it adds
        // color without adding coverage, so it brightens whatever is behind instead of covering it.
        case 31u: {
            let glow = textureSampleLevel(aux, linear_sampler, uv, 0.0) * u.p0.x;
            return vec4(c.rgb + glow.rgb, c.a);
        }
        // Drop Shadow, shape: the layer's alpha moved by p0.xy (px), at p0.z opacity, black.
        case 32u: {
            let a = sample(uv - u.p0.xy / size).a * u.p0.z;
            return vec4(0.0, 0.0, 0.0, a);
        }
        // Drop Shadow, combine: the layer over its (blurred) shadow in aux.
        case 33u: {
            let shadow = textureSampleLevel(aux, linear_sampler, uv, 0.0);
            return c + shadow * (1.0 - c.a);
        }
        default: { return c; }
    }
}
