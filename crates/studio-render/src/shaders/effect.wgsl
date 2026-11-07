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

// `p` turned by `angle` radians. Pixel rows run downward, so a positive angle turns clockwise
// on screen.
fn rotate2(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2(p.x * c - p.y * s, p.x * s + p.y * c);
}

// A color turned around the gray axis by `degrees`, which shifts its hue (red to green at
// 120°) while keeping its gray level. Used to treat any key color as if it were green.
fn hue_rotate(v: vec3<f32>, degrees: f32) -> vec3<f32> {
    let axis = vec3(0.57735027);
    let a = radians(degrees);
    return v * cos(a) + cross(axis, v) * sin(a) + axis * dot(axis, v) * (1.0 - cos(a));
}

// The color part (Cb, Cr) of a display-value color in BT.709 YCbCr, which separates color from
// brightness the way the eye does. The Keyer compares colors here.
fn chroma_709(v: vec3<f32>) -> vec2<f32> {
    let y = dot(v, LUMA);
    return vec2((v.b - y) / 1.8556, (v.r - y) / 1.5748);
}

// The Keyer's matte for one straight display-value color: 0 where it matches the key color
// (p0.xyz, 0-255), 1 where it is kept. Its color is first brought to the key's brightness, so
// darker and brighter patches of an unevenly lit screen still match. The distance is measured
// as a fraction of the key's own colorfulness, so Tolerance (p0.w %) and Softness (p1.x %) act
// the same for green and blue screens.
fn keyer_keep(color: vec3<f32>) -> f32 {
    let v = clamp(color, vec3(0.0), vec3(1.0));
    let key = clamp(u.p0.xyz / 255.0, vec3(0.0), vec3(1.0));
    let key_chroma = chroma_709(key);
    let gain = min(dot(key, LUMA) / max(dot(v, LUMA), 0.001), 4.0);
    let distance = length(chroma_709(v) * gain - key_chroma) / max(length(key_chroma), 0.02);
    let tolerance = u.p0.w / 100.0;
    return smoothstep(tolerance, tolerance + max(u.p1.x / 100.0, 0.001), distance);
}

// A sample's alpha after the Keyer's matte, for pulling the edge in. Reads past the frame's
// edge clamp to it, so footage filling the frame isn't eaten from its borders.
fn keyed_alpha_at(uv: vec2<f32>) -> f32 {
    let s = textureSampleLevel(src, linear_sampler, clamp(uv, vec2(0.0), vec2(1.0)), 0.0);
    return s.a * keyer_keep(to_display(straight(s)));
}

// A sample's display-value color with its alpha applied, so transparent areas read as black.
// Edge and relief effects use it to see the layer's outline as well as its detail.
fn display_at(uv: vec2<f32>) -> vec3<f32> {
    let s = sample(uv);
    return to_display(straight(s)) * s.a;
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
        // Linear Color Key: p0.x key hue (deg), p0.y tolerance (%), p0.z softness (%). Colors whose
        // hue lies within the tolerance of the key, and that are saturated enough, become clear.
        case 61u: {
            let v = clamp(to_display(rgb), vec3(0.0), vec3(1.0));
            let hsl = rgb_to_hsl(v);
            let distance = abs(fract(hsl.x - u.p0.x / 360.0 + 0.5) - 0.5) * 360.0;
            let tolerance = u.p0.y / 100.0 * 180.0;
            let softness = max(u.p0.z / 100.0 * 180.0, 0.5);
            let chroma = max(v.r, max(v.g, v.b)) - min(v.r, min(v.g, v.b));
            let keyed = (1.0 - smoothstep(tolerance, tolerance + softness, distance)) * smoothstep(0.08, 0.2, chroma);
            return c * (1.0 - keyed);
        }
        // Spill Suppressor: p0.x key hue (deg), p0.y suppression (100 = full). The color is turned so
        // the key hue sits on green, green is limited to the larger of red and blue, and it is
        // turned back.
        case 62u: {
            var v = hue_rotate(to_display(rgb), 120.0 - u.p0.x);
            let spill = max(v.g - max(v.r, v.b), 0.0);
            v.g -= spill * u.p0.y / 100.0;
            return with_alpha(to_linear(hue_rotate(v, u.p0.x - 120.0)), c.a);
        }
        // Key Cleaner: p0.x edge radius (px). The alpha is averaged over rings at that radius and
        // only well-surrounded pixels are kept, which pulls the edge in and softens it while solid
        // areas stay solid.
        case 63u: {
            let radius = u.p0.x;
            if (radius < 0.25 || c.a <= 0.0) {
                return c;
            }
            var total = 0.0;
            for (var i = 0; i < 16; i++) {
                let angle = f32(i) / 16.0 * 2.0 * PI;
                let direction = vec2(cos(angle), sin(angle)) / size;
                total += sample(uv + direction * radius).a * 2.0 + sample(uv + direction * radius * 0.5).a;
            }
            // Compared with the pixel's own alpha, so an evenly see-through layer is left alone.
            let around = clamp(total / 48.0 / c.a, 0.0, 1.0);
            return c * smoothstep(0.5, 1.0, around);
        }
        // Median: p0.x radius (px). The middle value, per channel, of a 3 × 3 grid of samples
        // spaced by the radius, sorted with a 19-step compare-and-swap network.
        case 64u: {
            let radius = u.p0.x;
            if (radius < 0.25) {
                return c;
            }
            var v: array<vec4<f32>, 9>;
            for (var i = 0; i < 9; i++) {
                let offset = vec2(f32(i % 3) - 1.0, f32(i / 3) - 1.0) * radius / size;
                v[i] = sample(uv + offset);
            }
            var pairs = array<vec2<u32>, 19>(
                vec2(1u, 2u), vec2(4u, 5u), vec2(7u, 8u), vec2(0u, 1u), vec2(3u, 4u), vec2(6u, 7u), vec2(1u, 2u),
                vec2(4u, 5u), vec2(7u, 8u), vec2(0u, 3u), vec2(5u, 8u), vec2(4u, 7u), vec2(3u, 6u), vec2(1u, 4u),
                vec2(2u, 5u), vec2(4u, 7u), vec2(4u, 2u), vec2(6u, 4u), vec2(4u, 2u),
            );
            for (var i = 0; i < 19; i++) {
                let pair = pairs[i];
                let a = v[pair.x];
                let b = v[pair.y];
                v[pair.x] = min(a, b);
                v[pair.y] = max(a, b);
            }
            return v[4];
        }
        // Stroke: p0.x brush size (px), p0.y opacity (%). The most coverage found within the brush
        // size around each pixel, as white, placed under the layer so it shows only outside it.
        case 65u: {
            let brush = u.p0.x;
            if (brush < 0.25) {
                return c;
            }
            var reach = 0.0;
            for (var ring = 1; ring <= 3; ring++) {
                let radius = brush * f32(ring) / 3.0;
                for (var i = 0; i < 24; i++) {
                    let angle = (f32(i) + f32(ring) * 0.33) / 24.0 * 2.0 * PI;
                    reach = max(reach, sample(uv + vec2(cos(angle), sin(angle)) * radius / size).a);
                }
            }
            let outline = reach * u.p0.y / 100.0;
            return c + vec4(outline) * (1.0 - c.a);
        }
        // Crop: p0 left, right, top, bottom (% of the frame cut away), p1.x feather (px, inward).
        case 66u: {
            let low = vec2(u.p0.x, u.p0.z) / 100.0 * size;
            let high = (vec2(1.0) - vec2(u.p0.y, u.p0.w) / 100.0) * size;
            let inside = min(min(pixel.x - low.x, high.x - pixel.x), min(pixel.y - low.y, high.y - pixel.y));
            return c * clamp((inside + 0.5) / max(u.p1.x, 1.0), 0.0, 1.0);
        }
        // Mirror: p0.x axis angle (deg; 0 reflects the left half onto the right, 90 the top onto
        // the bottom), p0.y center offset (% of the frame along the reflection).
        case 67u: {
            let angle = radians(u.p0.x);
            let normal = vec2(cos(angle), sin(angle));
            let extent = abs(normal.x) * size.x + abs(normal.y) * size.y;
            let origin = size * 0.5 + normal * u.p0.y / 100.0 * extent;
            let distance = dot(pixel - origin, normal);
            return sample(select(pixel, pixel - 2.0 * distance * normal, distance > 0.0) / size);
        }
        // Offset: p0.x, p0.y shift (% of the frame). The picture wraps around, so what leaves one
        // edge comes back at the other.
        case 68u: { return textureSampleLevel(src, linear_sampler, fract(uv - u.p0.xy / 100.0), 0.0); }
        // Motion Tile: p0.x tiles across, p0.y tiles down, p0.z mirror edges (1 flips every other
        // tile so neighbours meet seamlessly). The middle tile stays centered.
        case 69u: {
            let tiles = max(u.p0.xy, vec2(1.0));
            let q = (uv - center) * tiles + center;
            let cell = floor(q);
            var f = fract(q);
            if (u.p0.z >= 0.5) {
                let odd = abs(cell - 2.0 * floor(cell * 0.5));
                f = mix(f, vec2(1.0) - f, odd);
            }
            return textureSampleLevel(src, linear_sampler, f, 0.0);
        }
        // Twirl: p0.x angle (deg), p0.y radius (% of half the shorter side). Turns most at the
        // center, easing to nothing at the radius.
        case 70u: {
            let offset = pixel - size * 0.5;
            let radius = max(u.p0.y / 100.0 * 0.5 * min(size.x, size.y), 1.0);
            let distance = length(offset);
            if (distance >= radius) {
                return c;
            }
            let t = 1.0 - distance / radius;
            return sample((size * 0.5 + rotate2(offset, -radians(u.p0.x) * t * t)) / size);
        }
        // Ripple: p0.x amplitude (px), p0.y wavelength (px), p0.z speed (cycles per second). Rings
        // spread outward from the center.
        case 71u: {
            let offset = pixel - size * 0.5;
            let distance = length(offset);
            let direction = select(offset / distance, vec2(0.0), distance < 0.001);
            let push = sin((distance / max(u.p0.y, 1.0) - u.info.z * u.p0.z) * 2.0 * PI) * u.p0.x;
            return sample((pixel + direction * push) / size);
        }
        // Kaleidoscope: p0.x segments, p0.y rotation (deg). One wedge around the center is
        // repeated and mirrored all the way round.
        case 72u: {
            let offset = pixel - size * 0.5;
            let turn = radians(u.p0.y);
            let wedge = 2.0 * PI / max(u.p0.x, 2.0);
            let angle = atan2(offset.y, offset.x) - turn;
            var within = angle - wedge * floor(angle / wedge);
            within = min(within, wedge - within);
            let p = length(offset) * vec2(cos(within + turn), sin(within + turn));
            return sample((size * 0.5 + p) / size);
        }
        // Polar Coordinates: p0.x amount (%). Rectangular to polar: the picture's width wraps
        // clockwise round the center from the top, its top edge at the center.
        case 73u: {
            let offset = (pixel - size * 0.5) / (0.5 * min(size.x, size.y));
            let angle = atan2(offset.x, -offset.y);
            let polar = vec2(fract(angle / (2.0 * PI)), length(offset));
            return sample(mix(uv, polar, u.p0.x / 100.0));
        }
        // Halftone: p0.x dot size (px), p0.y screen angle (deg). Black dots on white, each sized
        // by the darkness at its cell's center. The layer's alpha is kept.
        case 74u: {
            let cell_size = max(u.p0.x, 2.0);
            let angle = radians(u.p0.y);
            let turned = rotate2(pixel, -angle);
            let cell_center = (floor(turned / cell_size) + 0.5) * cell_size;
            let s = sample(rotate2(cell_center, angle) / size);
            let lightness = mix(1.0, clamp(dot(to_display(straight(s)), LUMA), 0.0, 1.0), s.a);
            let radius = sqrt(1.0 - lightness) * 0.70710678 * cell_size;
            let ink = clamp(radius - length(turned - cell_center) + 0.5, 0.0, 1.0) * min(radius * 2.0, 1.0);
            return with_alpha(vec3(1.0 - ink), c.a);
        }
        // Find Edges: p0.x amount (%). A Sobel filter on each channel, shown as dark lines on white.
        case 75u: {
            let t = 1.0 / size;
            let tl = display_at(uv + vec2(-t.x, -t.y));
            let tc = display_at(uv + vec2(0.0, -t.y));
            let tr = display_at(uv + vec2(t.x, -t.y));
            let ml = display_at(uv + vec2(-t.x, 0.0));
            let mr = display_at(uv + vec2(t.x, 0.0));
            let bl = display_at(uv + vec2(-t.x, t.y));
            let bc = display_at(uv + vec2(0.0, t.y));
            let br = display_at(uv + vec2(t.x, t.y));
            let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
            let gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
            let edges = vec3(1.0) - clamp(sqrt(gx * gx + gy * gy) * 0.5, vec3(0.0), vec3(1.0));
            return with_alpha(to_linear(mix(to_display(rgb), edges, u.p0.x / 100.0)), c.a);
        }
        // Emboss: p0.x amount (%), p0.y light direction (deg, clockwise from up). A gray relief
        // whose sides facing the light are bright.
        case 76u: {
            let angle = radians(u.p0.y);
            let toward = vec2(sin(angle), -cos(angle)) * 1.5 / size;
            let lit = dot(display_at(uv - toward), LUMA) - dot(display_at(uv + toward), LUMA);
            let gray = clamp(0.5 + lit * u.p0.x / 100.0, 0.0, 1.0);
            return with_alpha(to_linear(vec3(gray)), c.a);
        }
        // Duotone: p0.x shadow hue, p0.y highlight hue (deg), p0.z amount (%). Brightness is mapped
        // from a deep version of the shadow hue to a light version of the highlight hue.
        case 77u: {
            let v = to_display(rgb);
            let y = clamp(dot(v, LUMA), 0.0, 1.0);
            let dark = hsl_to_rgb(vec3(fract(u.p0.x / 360.0), 0.85, 0.22));
            let light = hsl_to_rgb(vec3(fract(u.p0.y / 360.0), 0.9, 0.75));
            let toned = mix(dark, light, smoothstep(0.0, 1.0, y));
            return with_alpha(to_linear(mix(v, toned, u.p0.z / 100.0)), c.a);
        }
        // Iris Wipe: p0.x completion (%), p0.y feather (px). A circle closes on the center; at 0%
        // it covers the whole frame (corners too), at 100% nothing is left.
        case 78u: {
            let feather = max(u.p0.y, 0.5);
            let reach = length(size) * 0.5 + feather;
            let radius = (1.0 - u.p0.x / 100.0) * reach - feather * 0.5;
            return c * (1.0 - smoothstep(radius - feather * 0.5, radius + feather * 0.5, length(pixel - size * 0.5)));
        }
        // Venetian Blinds: p0.x completion (%), p0.y direction (deg; 0 gives horizontal slats),
        // p0.z slat width (px), p0.w feather (px). Each slat closes from one side.
        case 79u: {
            let angle = radians(u.p0.y);
            let across = dot(pixel - size * 0.5, vec2(sin(angle), -cos(angle)));
            let width = max(u.p0.z, 1.0);
            let feather = max(u.p0.w, 0.5);
            let within = across - width * floor(across / width);
            let edge = u.p0.x / 100.0 * (width + feather) - feather * 0.5;
            return c * smoothstep(edge - feather * 0.5, edge + feather * 0.5, within);
        }
        // Gradient Wipe: p0.x completion (%), p0.y softness (%). The layer's own brightness is the
        // order: the darkest parts disappear first and the brightest last.
        case 80u: {
            let y = clamp(dot(to_display(rgb), LUMA), 0.0, 1.0);
            let softness = max(u.p0.y / 100.0, 0.002);
            let edge = u.p0.x / 100.0 * (1.0 + softness) - softness * 0.5;
            return c * smoothstep(edge - softness * 0.5, edge + softness * 0.5, y);
        }
        // Pulse: p0.x amount (% of extra size at the peak), p0.y pulses per second. Grows and
        // shrinks smoothly about the center.
        case 81u: {
            let grow = 1.0 + u.p0.x / 100.0 * 0.5 * (1.0 - cos(u.info.z * u.p0.y * 2.0 * PI));
            return sample((size * 0.5 + (pixel - size * 0.5) / grow) / size);
        }
        // Swing: p0.x angle (deg, the furthest it turns each way), p0.y swings per second. A
        // pendulum's rotation about the center.
        case 82u: {
            let turn = radians(u.p0.x) * sin(u.info.z * u.p0.y * 2.0 * PI);
            return sample((size * 0.5 + rotate2(pixel - size * 0.5, -turn)) / size);
        }
        // Keyer: p0.xyz key color (0-255 display values), p0.w tolerance (%), p1.x softness (%),
        // p1.y spill suppression (%), p1.z choke (px). Colors near the key turn clear (see
        // `keyer_keep`); Choke keeps the smallest alpha within its radius, which pulls the edge
        // in; and spill suppression turns the color so the key's hue sits on green, limits green
        // to the larger of red and blue (as Spill Suppressor does), and turns it back.
        case 83u: {
            let v = to_display(rgb);
            var alpha = c.a * keyer_keep(v);
            let choke = u.p1.z;
            if (choke >= 0.25 && alpha > 0.0) {
                for (var i = 0; i < 16; i++) {
                    let angle = f32(i) / 16.0 * 2.0 * PI;
                    let direction = vec2(cos(angle), sin(angle)) / size;
                    alpha = min(alpha, min(keyed_alpha_at(uv + direction * choke), keyed_alpha_at(uv + direction * choke * 0.5)));
                }
            }
            var kept = v;
            let suppression = u.p1.y / 100.0;
            if (suppression > 0.0) {
                let key_hue = rgb_to_hsl(clamp(u.p0.xyz / 255.0, vec3(0.0), vec3(1.0))).x * 360.0;
                var turned = hue_rotate(v, 120.0 - key_hue);
                turned.g -= max(turned.g - max(turned.r, turned.b), 0.0) * suppression;
                kept = hue_rotate(turned, key_hue - 120.0);
            }
            return with_alpha(to_linear(kept), alpha);
        }
        // Motion blur's running average (not a catalog effect): p0.x is the weight of the new
        // moment in `aux`. Mixing moment k in at 1/k leaves every moment weighted equally.
        case 84u: {
            return mix(c, textureSampleLevel(aux, linear_sampler, uv, 0.0), u.p0.x);
        }
        default: { return c; }
    }
}
