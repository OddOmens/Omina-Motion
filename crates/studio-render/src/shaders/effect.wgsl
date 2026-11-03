// Layer effects. Each pass reads `src` (and `aux` for passes that combine two images)
// and writes a new image of the same size. Colors are premultiplied, linear light.
// Lengths in the parameters are already converted to output pixels by the renderer.

struct EffectUniform {
    // x: effect id (see `effects.rs`), y-w: unused.
    id: vec4<u32>,
    // Up to twelve parameters.
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
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

// Linear light to display (sRGB-encoded) values and back. Grading works on display values, as
// colorists' tools do, so sliders act evenly from shadows to highlights. Values above 1 (bright
// light from a glow, say) carry on along a power curve instead of clipping.
fn to_display(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

// A color wheel's push: the hue (degrees) at `amount` (0-100), with its brightness taken out so
// it tints without lightening or darkening.
fn wheel_tint(hue: f32, amount: f32) -> vec3<f32> {
    let c = hsl_to_rgb(vec3(fract(hue / 360.0), 1.0, 0.5));
    return (c - vec3(dot(c, LUMA))) * amount / 100.0;
}

// A soft bump of brightness around `center`, for the Tone Curve's bands.
fn band(y: f32, center: f32) -> f32 {
    let d = (y - center) / 0.16;
    return exp(-d * d);
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
        // Black & White: p0.x amount (%), p0.y contrast (%), mixing toward a contrast-adjusted gray.
        case 40u: {
            let gray = clamp((dot(rgb, LUMA) - 0.5) * (1.0 + u.p0.y / 100.0) + 0.5, 0.0, 1.0);
            return with_alpha(mix(rgb, vec3(gray), u.p0.x / 100.0), c.a);
        }
        // Invert: p0.x amount (%).
        case 41u: { return with_alpha(mix(rgb, vec3(1.0) - rgb, u.p0.x / 100.0), c.a); }
        // Vignette: p0.x amount (%), p0.y size (% of the frame clear in the middle), p0.z softness (%).
        case 42u: {
            let offset = (uv - center) * vec2(size.x / max(size.y, 1.0), 1.0);
            let radius = u.p0.y / 100.0 * 0.75;
            let soft = max(u.p0.z / 100.0, 0.01) * 0.75;
            let shade = smoothstep(radius, radius + soft, length(offset)) * u.p0.x / 100.0;
            return vec4(c.rgb * (1.0 - shade), c.a);
        }
        // Letterbox: p0.x the picture's aspect ratio (width / height), p0.y bar opacity (%). Black
        // bars cover the top and bottom (or the sides, for ratios narrower than the frame).
        case 43u: {
            let frame = size.x / max(size.y, 1.0);
            let ratio = max(u.p0.x, 0.1);
            var bar = 0.0;
            if (ratio > frame) {
                let keep = frame / ratio * 0.5;
                bar = step(keep, abs(uv.y - 0.5));
            } else {
                let keep = ratio / frame * 0.5;
                bar = step(keep, abs(uv.x - 0.5));
            }
            let a = bar * u.p0.y / 100.0;
            return c * (1.0 - a) + vec4(0.0, 0.0, 0.0, a);
        }
        // Light Leak: p0.x amount (%), p0.y hue (deg), p0.z drift speed. Warm light drifting across
        // the frame, added to the layer (light, not coverage).
        case 44u: {
            let t = u.info.z * u.p0.z;
            let spot = vec2(0.25 + 0.5 * value_noise(vec2(t, 3.0)), 0.3 + 0.4 * value_noise(vec2(7.0, t)));
            let offset = (uv - spot) * vec2(size.x / max(size.y, 1.0), 1.0);
            let glow = exp(-dot(offset, offset) * 3.0) * (0.6 + 0.4 * fbm(uv * 3.0 + t)) * u.p0.x / 100.0;
            let tint = hsl_to_rgb(vec3(fract(u.p0.y / 360.0), 0.9, 0.6));
            return vec4(c.rgb + tint * glow * c.a, c.a);
        }
        // Strobe: p0.x flashes per second, p0.y flash length (% of each cycle), p0.z brightness (%).
        case 45u: {
            let phase = fract(u.info.z * u.p0.x);
            let on = step(phase, u.p0.y / 100.0) * u.p0.z / 100.0;
            return vec4(mix(c.rgb, vec3(c.a), on), c.a);
        }
        // Wiggle: p0.x amount (px), p0.y wiggles per second, p0.z seed. The layer drifts smoothly
        // around its place, like After Effects' wiggle() on position.
        case 46u: {
            let t = u.info.z * u.p0.y + u.p0.z * 17.0;
            let drift = (vec2(value_noise(vec2(t, 1.3)), value_noise(vec2(5.7, t))) - 0.5) * 2.0 * u.p0.x;
            return sample(uv - drift / size);
        }
        // Camera Shake: p0.x amount (px), p0.y shakes per second, p0.z rotation (deg), p0.w zoom (%).
        // Shakes, turns and zooms in a little, so the frame's edges stay hidden.
        case 47u: {
            let t = u.info.z * u.p0.y;
            let shake = (vec2(fbm(vec2(t, 2.0)), fbm(vec2(9.0, t))) - 0.5) * 2.0 * u.p0.x;
            let turn = radians((fbm(vec2(t, 13.0)) - 0.5) * 2.0 * u.p0.z);
            let zoom = 1.0 + u.p0.w / 100.0;
            var p = (pixel - size * 0.5 - shake) / zoom;
            p = vec2(p.x * cos(turn) - p.y * sin(turn), p.x * sin(turn) + p.y * cos(turn));
            return sample((p + size * 0.5) / size);
        }
        // Glitch: p0.x amount (%), p0.y block size (px), p0.z changes per second. Bands of the
        // picture jump sideways with their colors pulled apart, changing in steps.
        case 48u: {
            let amount = u.p0.x / 100.0;
            let step_time = floor(u.info.z * u.p0.z);
            let band = floor(pixel.y / max(u.p0.y, 1.0));
            let pick = hash(vec2(band, step_time));
            let hit = step(1.0 - amount * 0.6, pick);
            let shift = (hash(vec2(band + 3.1, step_time)) - 0.5) * 0.25 * amount * hit;
            let split = 0.01 * amount * (0.5 + hit);
            let r = sample(uv + vec2(shift + split, 0.0));
            let g = sample(uv + vec2(shift, 0.0));
            let b = sample(uv + vec2(shift - split, 0.0));
            return vec4(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
        }
        // RGB Split: p0.x distance (px), p0.y angle (deg). Red one way, blue the other.
        case 49u: {
            let a = radians(u.p0.y);
            let offset = vec2(cos(a), sin(a)) * u.p0.x / size;
            let r = sample(uv + offset);
            let b = sample(uv - offset);
            return vec4(r.r, c.g, b.b, max(max(r.a, c.a), b.a));
        }
        // VHS: p0.x amount (%), p0.y noise (%). Wobbling lines, bleeding color, soft detail and
        // tape noise.
        case 50u: {
            let amount = u.p0.x / 100.0;
            let t = u.info.z;
            let wobble = (value_noise(vec2(pixel.y * 0.05, t * 6.0)) - 0.5) * 0.006 * amount;
            let bleed = 0.004 * amount;
            let r = sample(uv + vec2(wobble + bleed, 0.0));
            let g = sample(uv + vec2(wobble, 0.0));
            let b = sample(uv + vec2(wobble - bleed, 0.0));
            var color = vec3(r.r, g.g, b.b);
            let a = max(max(r.a, g.a), b.a);
            let noise = (hash(pixel + fract(t) * 97.0) - 0.5) * u.p0.y / 100.0 * 0.3 * a;
            color = mix(color, vec3(dot(color, LUMA)), 0.25 * amount) + noise;
            return vec4(max(color, vec3(0.0)), a);
        }
        // Scanlines: p0.x line spacing (px), p0.y opacity (%), p0.z scroll speed (lines per second).
        case 51u: {
            let spacing = max(u.p0.x, 1.0);
            let line = fract((pixel.y + u.info.z * u.p0.z * spacing) / spacing);
            let dark = step(0.5, line) * u.p0.y / 100.0;
            return vec4(c.rgb * (1.0 - dark), c.a);
        }
        // Basic Correction. p0: temperature, tint (-100-100), exposure (stops), contrast;
        // p1: highlights, shadows, whites, blacks (-100-100); p2.x saturation (100 = unchanged),
        // p2.y vibrance. White balance and exposure act on light; the rest on display values.
        case 52u: {
            let t = u.p0.x / 100.0;
            let n = u.p0.y / 100.0;
            // Kept in step with `studio_core::grade::white_balance_gains`.
            let k = 0.5;
            var gains = vec3(exp2(k * (t + n * 0.5)), exp2(-k * n), exp2(k * (n * 0.5 - t)));
            gains = gains / dot(gains, LUMA);
            var v = to_display(rgb * gains * exp2(u.p0.z));
            v = (v - 0.5) * (1.0 + u.p0.w / 100.0) + 0.5;
            let y = clamp(dot(v, LUMA), 0.0, 1.0);
            let highlights = smoothstep(0.35, 1.0, y) * u.p1.x / 100.0 * 0.25;
            let shadows = (1.0 - smoothstep(0.0, 0.65, y)) * u.p1.y / 100.0 * 0.25;
            let whites = y * y * u.p1.z / 100.0 * 0.25;
            let blacks = (1.0 - y) * (1.0 - y) * (1.0 - y) * u.p1.w / 100.0 * 0.15;
            v = v + vec3(highlights + shadows + whites + blacks);
            let gray = dot(v, LUMA);
            v = mix(vec3(gray), v, u.p2.x / 100.0);
            // Vibrance boosts dull colors more than ones already strong, as in case 10.
            let spread = max(v.r, max(v.g, v.b)) - min(v.r, min(v.g, v.b));
            v = mix(vec3(gray), v, 1.0 + u.p2.y / 100.0 * (1.0 - clamp(spread, 0.0, 1.0)));
            return with_alpha(to_linear(v), c.a);
        }
        // Color Wheels: lift, gamma and gain. p0: shadows hue, amount, brightness, midtones hue;
        // p1: midtones amount, brightness, highlights hue, amount; p2.x highlights brightness.
        case 53u: {
            var v = to_display(rgb);
            let lift = wheel_tint(u.p0.x, u.p0.y) * 0.25 + u.p0.z / 100.0 * 0.25;
            let gamma = max(vec3(1.0) + wheel_tint(u.p0.w, u.p1.x) * 0.5 + u.p1.y / 100.0 * 0.5, vec3(0.1));
            let gain = vec3(1.0) + wheel_tint(u.p1.z, u.p1.w) * 0.5 + u.p2.x / 100.0 * 0.5;
            v = gain * (v + lift * (vec3(1.0) - v));
            v = pow(max(v, vec3(0.0)), 1.0 / gamma);
            return with_alpha(to_linear(v), c.a);
        }
        // Tone Curve: p0 highlights, lights, darks, shadows (-100-100), each lifting or lowering
        // its band of brightness. Black and white stay put.
        case 54u: {
            var v = to_display(rgb);
            let y = clamp(dot(v, LUMA), 0.0, 1.0);
            let bands = u.p0.x * band(y, 0.875) + u.p0.y * band(y, 0.625) + u.p0.z * band(y, 0.375) + u.p0.w * band(y, 0.125);
            let ends = clamp(8.0 * y * (1.0 - y), 0.0, 1.0);
            v = v + vec3(bands / 100.0 * 0.25 * ends);
            return with_alpha(to_linear(v), c.a);
        }
        // Levels, on display values from 0 to 255: p0 input black, input white, gamma, output
        // black; p1.x output white.
        case 55u: {
            let low = u.p0.x / 255.0;
            let high = max(u.p0.y / 255.0, low + 1.0 / 255.0);
            var v = clamp((to_display(rgb) - low) / (high - low), vec3(0.0), vec3(1.0));
            v = pow(v, vec3(1.0 / max(u.p0.z, 0.01)));
            v = mix(vec3(u.p0.w / 255.0), vec3(u.p1.x / 255.0), v);
            return with_alpha(to_linear(v), c.a);
        }
        // Mask: keep the image where the mask (in aux) draws, or with p0.x = 1 where it does not.
        case 60u: {
            let m = textureSampleLevel(aux, linear_sampler, uv, 0.0).a;
            return c * select(m, 1.0 - m, u.p0.x > 0.5);
        }
        default: { return c; }
    }
}
