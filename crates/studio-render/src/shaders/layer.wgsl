// Draws one layer's source rectangle into a composition-sized texture.
// The vertex shader maps source pixels to clip space with the layer's world transform;
// the fragment shader fills it with a solid color, a texture (text, images), a
// "no media yet" checker, or a shape. Output is premultiplied, linear light.
//
// Shapes are drawn from signed distances (negative inside, in source pixels), which give
// smooth edges at any zoom and make rounded corners exact: a shape inset by the radius,
// then grown back by it, has round corners of that radius.

struct LayerUniform {
    // Source pixels -> clip space: clip = (a*x + c*y + tx, b*x + d*y + ty).
    abcd: vec4<f32>,
    // tx, ty, source width, source height.
    translate_size: vec4<f32>,
    // Premultiplied linear color for solids and the placeholder tint.
    color: vec4<f32>,
    // x: mode (0 solid, 1 texture, 2 placeholder, 3 shape), y: checker cell size in source
    // pixels, z: shape stroke width in source pixels.
    params: vec4<f32>,
    // For shapes: x kind (0 rectangle, 1 ellipse, 2 triangle, 3 polygon, 4 star),
    // y corner radius in source pixels, z sides or points, w a star's inner radius (fraction).
    shape: vec4<f32>,
    // The shape's box within the drawn rectangle (which is larger by the stroke): origin, size.
    // For textures, the part of the texture drawn (uv origin, size).
    shape_box: vec4<f32>,
    // Fill and stroke paints (see `Paint` below).
    fill: Paint,
    stroke: Paint,
};

// A fill or stroke: x mode (0 none, 1 solid, 2 linear, 3 radial), y how many stops; its stops'
// premultiplied linear colors (one for a solid) and locations (0-1, four to a vec4); and the
// gradient's from and to points in fractions of the shape's box.
struct Paint {
    mode: vec4<f32>,
    colors: array<vec4<f32>, 8>,
    stops: array<vec4<f32>, 2>,
    points: vec4<f32>,
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
        // Drawn at the opacity in the color's alpha (1 for ordinary textures; less for fades).
        // `shape_box` picks the part of the texture drawn: all of it (0, 0, 1, 1), or a slice
        // (one letter of text set along a path).
        return textureSample(source, source_sampler, u.shape_box.xy + in.uv * u.shape_box.zw) * u.color.a;
    }
    if (mode == 2u) {
        // Media that is not decoded yet: a checker in the layer's label color.
        let cell = max(u.params.y, 1.0);
        let pixel = in.uv * u.translate_size.zw;
        let checker = (u32(floor(pixel.x / cell)) + u32(floor(pixel.y / cell))) % 2u;
        let shade = select(0.55, 0.35, checker == 1u);
        return vec4(u.color.rgb * shade, 1.0);
    }
    if (mode == 3u) {
        let half = u.shape_box.zw * 0.5;
        let p = in.uv * u.translate_size.zw + u.shape_box.xy - half;
        let d = shape_distance(p, half);
        // About one output pixel of soft edge, whatever the layer's scale.
        let edge = max(fwidth(d), 1e-4);
        var out = vec4(0.0);
        if (u.fill.mode.x > 0.5) {
            out = paint_at(p, half, u.fill) * clamp(0.5 - d / edge, 0.0, 1.0);
        }
        let width = u.params.z;
        if (u.stroke.mode.x > 0.5 && width > 0.0) {
            // Centered on the outline, drawn over the fill.
            let band = abs(d) - width * 0.5;
            let stroke = paint_at(p, half, u.stroke) * clamp(0.5 - band / edge, 0.0, 1.0);
            out = stroke + out * (1.0 - stroke.a);
        }
        return out;
    }
    return u.color;
}

const PI: f32 = 3.14159265;

// A paint's color at `p` (pixels from the box center), for a box with half size `half`: one
// color, or a gradient. A linear gradient runs between its points measured in pixels, so its
// bands stay square to the line; a radial one is measured in fractions of the box, so its
// circle stretches with the box as an ellipse (as `path.rs` draws the same paint).
fn paint_at(p: vec2<f32>, half: vec2<f32>, paint: Paint) -> vec4<f32> {
    let mode = u32(paint.mode.x);
    if (mode == 1u) {
        return paint.colors[0];
    }
    let size = max(2.0 * half, vec2(1e-4));
    let f = p / size + 0.5;
    var t = 0.0;
    if (mode == 2u) {
        let a = paint.points.xy * size;
        let along = paint.points.zw * size - a;
        t = dot(f * size - a, along) / max(dot(along, along), 1e-4);
    } else {
        t = length(f - paint.points.xy) / max(length(paint.points.zw - paint.points.xy), 1e-4);
    }
    return gradient_color(clamp(t, 0.0, 1.0), paint);
}

// The color `t` (0-1) along a gradient, blended between the stops on either side of it.
fn gradient_color(t: f32, paint: Paint) -> vec4<f32> {
    // Copied to locals so they can be indexed by a loop counter.
    var colors = paint.colors;
    var stops = paint.stops;
    let count = clamp(u32(paint.mode.y), 1u, 8u);
    if (t <= stops[0].x) {
        return colors[0];
    }
    for (var i = 1u; i < count; i = i + 1u) {
        let b = stops[i / 4u][i % 4u];
        if (t <= b) {
            let a = stops[(i - 1u) / 4u][(i - 1u) % 4u];
            return mix(colors[i - 1u], colors[i], (t - a) / max(b - a, 1e-6));
        }
    }
    return colors[count - 1u];
}

// Signed distance to the layer's shape at `p` (source pixels from the box center), for a box
// with half size `half`.
fn shape_distance(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let kind = u32(u.shape.x);
    let radius = clamp(u.shape.y, 0.0, min(half.x, half.y));
    let points = max(u.shape.z, 3.0);
    if (kind == 0u) {
        let q = abs(p) - half + radius;
        return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    }
    if (kind == 1u) {
        // Ellipse: a scaled circle; dividing by the gradient keeps the distance close to pixels.
        let n = p / half;
        let f = length(n) - 1.0;
        let gradient = length(n / half) / max(length(n), 1e-4);
        return f / max(gradient, 1e-6);
    }
    if (kind == 2u) {
        // Point up, base along the bottom of the box (y grows downward).
        let a = vec2(0.0, -half.y);
        let b = vec2(half.x, half.y);
        let c = vec2(-half.x, half.y);
        return rounded_triangle(p, a, b, c, radius);
    }
    // Polygons and stars are regular, so they fit a circle; a wide box stretches them.
    let r = min(half.x, half.y);
    let q = vec2(p.x, -p.y) * (r / half);
    if (kind == 3u) {
        let apothem = r * cos(PI / points);
        return regular_polygon(q, max(apothem - radius, 0.0), points) - radius;
    }
    let outer = max(r - radius, 0.0);
    return star(q, outer, outer * clamp(u.shape.w, 0.05, 1.0), points) - radius;
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// A triangle with rounded corners: shrunk toward its incenter by `radius`, then grown back.
fn rounded_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, radius: f32) -> f32 {
    let la = length(b - c);
    let lb = length(c - a);
    let lc = length(a - b);
    let perimeter = la + lb + lc;
    let center = (a * la + b * lb + c * lc) / perimeter;
    let inradius = abs(cross2(b - a, c - a)) / perimeter;
    let k = max(inradius - radius, 0.0) / max(inradius, 1e-4);
    return triangle(p, center + (a - center) * k, center + (b - center) * k, center + (c - center) * k) - radius;
}

// Exact signed distance to a triangle (after Inigo Quilez).
fn triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);
    let s = sign(cross2(e0, e2));
    let d = min(min(vec2(dot(pq0, pq0), s * cross2(v0, e0)), vec2(dot(pq1, pq1), s * cross2(v1, e1))), vec2(dot(pq2, pq2), s * cross2(v2, e2)));
    return -sqrt(d.x) * sign(d.y);
}

// Signed distance to a regular polygon with a vertex on top, given its apothem (the distance
// from the center to the middle of an edge). `p` has y pointing up.
fn regular_polygon(p: vec2<f32>, apothem: f32, sides: f32) -> f32 {
    let sector = 2.0 * PI / sides;
    // The angle from straight up, folded into one edge's sector so the edge's middle is at 0.
    let angle = atan2(p.x, p.y);
    let folded = angle - sector * floor(angle / sector) - sector * 0.5;
    let q = length(p) * vec2(sin(folded), cos(folded));
    // That edge is now horizontal at height `apothem`.
    let half_edge = apothem * tan(PI / sides);
    let nearest = vec2(clamp(q.x, -half_edge, half_edge), apothem);
    return length(q - nearest) * sign(q.y - apothem);
}

// Signed distance to a star with `points` points between radii `outer` and `inner`, a point on
// top. `p` has y pointing up. By symmetry only one half-point matters: the edge from a notch
// (on the y axis after folding) out to the tip beside it.
fn star(p: vec2<f32>, outer: f32, inner: f32, points: f32) -> f32 {
    let half_sector = PI / points;
    let angle = atan2(p.x, p.y);
    let folded = angle - 2.0 * half_sector * floor(angle / (2.0 * half_sector));
    let b = abs(folded - half_sector);
    let q = length(p) * vec2(sin(b), cos(b));
    let notch = vec2(0.0, inner);
    let tip = outer * vec2(sin(half_sector), cos(half_sector));
    let edge = tip - notch;
    let along = clamp(dot(q - notch, edge) / dot(edge, edge), 0.0, 1.0);
    let outside = cross2(edge, q - notch);
    return length(q - notch - edge * along) * sign(outside);
}
