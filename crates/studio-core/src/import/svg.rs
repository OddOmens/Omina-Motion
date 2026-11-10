//! Reading SVG files into a composition with one shape layer per SVG shape, so each piece of a
//! logo or illustration can be animated on its own. The `usvg` library does the hard parts of
//! SVG (styles and CSS, `<use>`, nested transforms, arcs, rectangles and circles): it hands back
//! plain paths in drawing order, which become Studio's pen paths.
//!
//! What comes in: every visible shape, with its solid or gradient fill (and fill rule, so
//! letters keep their holes), its stroke, and its opacity and that of the groups around it.
//! What doesn't (the notes say so): text (convert it to outlines first), embedded images,
//! patterns, dashes, clip paths, masks and filters.

use std::path::Path;

use usvg::tiny_skia_path::{PathSegment, Point, Transform};

use super::{Scene, SceneComp, SceneContent, SceneLayer, SceneTransform, SceneVector, Track};
use crate::animation::Vec2;
use crate::gradient::{Gradient, GradientStop};
use crate::model::{Paint, Rgba, Stroke};
use crate::path::{PathPoint, ShapePath};

/// The most shapes one file may bring in, against hostile files.
const MAX_SHAPES: usize = 2_000;

/// Reads an SVG file's text into a scene: one composition the size of the drawing, at `fps`
/// and `seconds` long (the open composition's, so it can be broken into shapes there), with a
/// layer per shape (the last drawn on top), named by its `id` where it has one.
pub fn read(text: &str, file: &Path, fps: f64, seconds: f64) -> Result<Scene, String> {
    let options = usvg::Options { resources_dir: file.parent().map(Path::to_path_buf), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(text, &options).map_err(|error| format!("This SVG couldn't be read: {error}."))?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    if !(width.is_finite() && height.is_finite() && (1.0..=30_000.0).contains(&width) && (1.0..=30_000.0).contains(&height)) {
        return Err("This SVG has an impossible size.".into());
    }
    if !(fps > 0.0 && fps <= 999.0 && seconds > 0.0 && seconds.is_finite()) {
        return Err("The composition to bring the SVG into has an impossible frame rate or length.".into());
    }
    let mut reader = Reader { layers: Vec::new(), notes: Vec::new(), count: 0, seconds };
    reader.group(tree.root(), 1.0);
    if text.contains("<text") {
        reader.note("Text isn't brought in from SVG files; convert text to outlines first.");
    }
    if reader.layers.is_empty() {
        return Err("This SVG has no shapes to bring in.".into());
    }
    if reader.count > MAX_SHAPES {
        reader.note(&format!("Only the first {MAX_SHAPES} shapes came in."));
    }
    // Scene layers go top first; the SVG draws its first shape at the bottom.
    reader.layers.reverse();
    // Named with its extension, so it reads as the SVG it came from in the Project panel.
    let name = file.file_name().map_or_else(|| "SVG".to_owned(), |name| name.to_string_lossy().into_owned());
    let comp = SceneComp {
        key: 1,
        name,
        folder: None,
        width: width.round().max(1.0) as u32,
        height: height.round().max(1.0) as u32,
        fps,
        seconds,
        background: Rgba::TRANSPARENT,
        layers: reader.layers,
    };
    Ok(Scene { comps: vec![comp], notes: reader.notes, loose: true, ..Scene::default() })
}

/// Walks the drawing, collecting a layer per shape.
struct Reader {
    layers: Vec<SceneLayer>,
    notes: Vec<String>,
    /// Shapes seen, including any past the limit.
    count: usize,
    /// How long each layer lasts: the whole composition.
    seconds: f64,
}

impl Reader {
    fn note(&mut self, text: &str) {
        if !self.notes.iter().any(|note| note == text) {
            self.notes.push(text.to_owned());
        }
    }

    /// The shapes in `group`, in drawing order, with `opacity` from the groups around it.
    fn group(&mut self, group: &usvg::Group, opacity: f32) {
        if group.clip_path().is_some() || group.mask().is_some() {
            self.note("Clip paths and masks aren't brought in; the shapes inside them show whole.");
        }
        if !group.filters().is_empty() {
            self.note("SVG filters (blurs, shadows) aren't brought in.");
        }
        let opacity = opacity * group.opacity().get();
        for node in group.children() {
            match node {
                usvg::Node::Group(inner) => self.group(inner, opacity),
                usvg::Node::Path(path) => self.path(path, opacity),
                usvg::Node::Image(_) => self.note("Images inside SVG files aren't brought in."),
                usvg::Node::Text(_) => self.note("Text isn't brought in from SVG files; convert text to outlines first."),
            }
        }
    }

    /// One shape as a layer.
    fn path(&mut self, path: &usvg::Path, opacity: f32) {
        if !path.is_visible() || (path.fill().is_none() && path.stroke().is_none()) {
            return;
        }
        self.count += 1;
        if self.count > MAX_SHAPES {
            return;
        }
        let transform = path.abs_transform();
        let filled = path.fill().is_some();
        let Some(mut outline) = outlines(path.data(), transform, filled) else { return };
        outline.even_odd = path.fill().is_some_and(|fill| fill.rule() == usvg::FillRule::EvenOdd);
        let Some((min, size, _)) = crate::path::fit_to_box(&outline) else { return };
        // Gradients are laid over the layer's box, in fractions of it.
        let in_box = |point: Point| Vec2::new((point.x - min.x) / size.x, (point.y - min.y) / size.y);
        let fill = path.fill().and_then(|fill| self.paint(fill.paint(), fill.opacity().get(), transform, &in_box));
        let stroke = path.stroke().and_then(|stroke| {
            if stroke.dasharray().is_some() {
                self.note("Dashed strokes come in solid.");
            }
            let paint = self.paint(stroke.paint(), stroke.opacity().get(), transform, &in_box)?;
            // The stroke grows with the shape's scale (the average of its two directions).
            let (scale_x, scale_y) = transform.get_scale();
            let width = (stroke.width().get() * (scale_x + scale_y) * 0.5).min(crate::model::MAX_STROKE_WIDTH);
            (width > 0.0).then_some(Stroke { paint, width })
        });
        let name = if path.id().is_empty() { format!("Shape {}", self.count) } else { path.id().to_owned() };
        let center = Vec2::new(min.x + size.x * 0.5, min.y + size.y * 0.5);
        self.layers.push(SceneLayer {
            key: self.count as u64,
            name,
            parent: None,
            start: 0.0,
            in_point: 0.0,
            out_point: self.seconds,
            visible: true,
            content: SceneContent::Vector(Box::new(SceneVector { path: outline, fill, stroke })),
            transform: SceneTransform {
                position: Some(Track::fixed(center)),
                opacity: (opacity < 1.0).then(|| Track::fixed((opacity * 100.0).clamp(0.0, 100.0))),
                ..SceneTransform::default()
            },
        });
    }

    /// An SVG paint as Studio's: a color, or a gradient placed over the layer's box (`in_box`
    /// turns drawing coordinates into fractions of it). Patterns come in as nothing.
    fn paint(&mut self, paint: &usvg::Paint, opacity: f32, shape: Transform, in_box: &dyn Fn(Point) -> Vec2) -> Option<Paint> {
        let color = |color: usvg::Color, alpha: f32| Rgba::hex((u32::from(color.red) << 16) | (u32::from(color.green) << 8) | u32::from(color.blue)).with_alpha(alpha);
        let stops = |stops: &[usvg::Stop]| -> Vec<GradientStop> {
            stops.iter().take(crate::gradient::MAX_STOPS).map(|stop| GradientStop { at: stop.offset().get(), color: color(stop.color(), stop.opacity().get() * opacity) }).collect()
        };
        // Where a point of the gradient lands on the layer's box.
        let place = |gradient: Transform, x: f32, y: f32| {
            let mut point = Point::from_xy(x, y);
            shape.pre_concat(gradient).map_point(&mut point);
            in_box(point)
        };
        match paint {
            usvg::Paint::Color(value) => Some(Paint::Solid(color(*value, opacity))),
            usvg::Paint::LinearGradient(gradient) => {
                let stops = stops(gradient.stops());
                let (from, to) = (place(gradient.transform(), gradient.x1(), gradient.y1()), place(gradient.transform(), gradient.x2(), gradient.y2()));
                gradient_or_color(&stops, from, to).map(|(gradient, solid)| solid.map_or(Paint::Linear(gradient), Paint::Solid))
            }
            usvg::Paint::RadialGradient(gradient) => {
                let stops = stops(gradient.stops());
                let center = place(gradient.transform(), gradient.cx(), gradient.cy());
                let edge = place(gradient.transform(), gradient.cx() + gradient.r().get(), gradient.cy());
                gradient_or_color(&stops, center, edge).map(|(gradient, solid)| solid.map_or(Paint::Radial(gradient), Paint::Solid))
            }
            usvg::Paint::Pattern(_) => {
                self.note("Pattern fills aren't brought in.");
                None
            }
        }
    }
}

/// A gradient from `stops`, or (with a single stop) that stop's color, since a gradient needs two.
fn gradient_or_color(stops: &[GradientStop], from: Vec2, to: Vec2) -> Option<(Gradient, Option<Rgba>)> {
    match stops {
        [] => None,
        [only] => Some((Gradient::two(only.color, only.color, from, to), Some(only.color))),
        _ => Some((Gradient::new(stops, from, to).sanitized(), None)),
    }
}

/// An SVG path's outlines (one per "move to"), with every point placed by `transform`, as one
/// compound Studio path: the first outline, with the rest in its `compound` list. A filled
/// shape's outlines are all closed, as SVG fills them. `None` when nothing drawable is left.
fn outlines(data: &usvg::tiny_skia_path::Path, transform: Transform, filled: bool) -> Option<ShapePath> {
    let place = |point: Point| {
        let mut point = point;
        transform.map_point(&mut point);
        Vec2::new(point.x, point.y)
    };
    let mut done: Vec<ShapePath> = Vec::new();
    let mut current: Vec<PathPoint> = Vec::new();
    let mut closed = false;
    let mut finish = |points: &mut Vec<PathPoint>, closed: bool| {
        let mut points = std::mem::take(points);
        if closed && points.len() > 2 {
            // A closing point on top of the first is the first one: keep its incoming handle.
            let (first, last) = (points[0].at, points[points.len() - 1].at);
            if (first - last).length() < 1e-3 {
                let last = points.pop().unwrap_or(PathPoint::corner(first));
                points[0].handle_in = last.handle_in;
            }
        }
        if points.len() >= 2 && points.len() <= crate::path::MAX_PATH_POINTS {
            done.push(ShapePath { points, closed: closed || filled, ..ShapePath::default() });
        }
    };
    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(to) => {
                finish(&mut current, closed);
                closed = false;
                current.push(PathPoint::corner(place(to)));
            }
            PathSegment::LineTo(to) => current.push(PathPoint::corner(place(to))),
            PathSegment::QuadTo(control, to) => {
                // A quadratic curve as a cubic: each control two thirds of the way to the quadratic's.
                let Some(start) = current.last().map(|point| point.at) else { continue };
                let (control, end) = (place(control), place(to));
                let a = start + (control - start) * (2.0 / 3.0);
                let b = end + (control - end) * (2.0 / 3.0);
                cubic_to(&mut current, a, b, end);
            }
            PathSegment::CubicTo(a, b, to) => cubic_to(&mut current, place(a), place(b), place(to)),
            PathSegment::Close => closed = true,
        }
    }
    finish(&mut current, closed);
    let mut outlines = done.into_iter();
    let first = outlines.next()?;
    Some(ShapePath { compound: outlines.take(crate::path::MAX_COMPOUND_OUTLINES).collect(), ..first })
}

/// Adds a cubic curve to `points`: the last point's outgoing handle reaches `a`, and the new
/// point at `end` has its incoming handle at `b`. (Handles are offsets from their point.)
fn cubic_to(points: &mut Vec<PathPoint>, a: Vec2, b: Vec2, end: Vec2) {
    let Some(last) = points.last_mut() else { return };
    last.handle_out = a - last.at;
    points.push(PathPoint { at: end, handle_in: b - end, handle_out: Vec2::default() });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_text(svg: &str) -> Scene {
        read(svg, Path::new("/art/logo.svg"), 30.0, 10.0).unwrap()
    }

    fn vector(layer: &SceneLayer) -> &SceneVector {
        let SceneContent::Vector(vector) = &layer.content else { panic!("a vector layer") };
        vector
    }

    #[test]
    fn each_shape_becomes_a_layer_named_by_its_id_with_the_last_on_top() {
        let scene = read_text(r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <rect id="back" x="0" y="0" width="200" height="100" fill="#ff0000"/>
            <circle cx="150" cy="50" r="20" fill="#00ff00" opacity="0.5"/>
        </svg>"##);
        let comp = &scene.comps[0];
        assert_eq!((comp.name.as_str(), comp.width, comp.height), ("logo.svg", 200, 100));
        assert_eq!(comp.layers.len(), 2);
        // Top first: the circle, drawn last, then the rectangle.
        assert_eq!(comp.layers[0].name, "Shape 2");
        assert_eq!(comp.layers[1].name, "back");
        let back = vector(&comp.layers[1]);
        assert_eq!(back.fill, Some(Paint::Solid(Rgba::hex(0xff0000))));
        assert!(back.path.closed);
        // The circle's layer sits on the circle, half see-through.
        let circle = &comp.layers[0];
        let position = circle.transform.position.as_ref().unwrap();
        assert!((position.value - Vec2::new(150.0, 50.0)).length() < 0.5);
        assert_eq!(circle.transform.opacity.as_ref().map(|opacity| opacity.value), Some(50.0));
    }

    #[test]
    fn group_transforms_place_shapes_and_compound_paths_keep_their_holes() {
        let scene = read_text(r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <g transform="translate(10 20) scale(2)">
                <path id="ring" fill-rule="evenodd" fill="#000" d="M0 0 H30 V30 H0 Z M10 10 H20 V20 H10 Z"/>
            </g>
        </svg>"##);
        let ring = vector(&scene.comps[0].layers[0]);
        assert!(ring.path.even_odd);
        assert_eq!(ring.path.compound.len(), 1, "the inner square is a second outline");
        let (min, max) = ring.path.bounds().unwrap();
        assert!((min - Vec2::new(10.0, 20.0)).length() < 1e-3 && (max - Vec2::new(70.0, 80.0)).length() < 1e-3, "{min:?} {max:?}");
        assert_eq!(ring.path.points.len(), 4, "the closing point on top of the first is dropped");
    }

    #[test]
    fn strokes_curves_and_gradients_come_in() {
        let scene = read_text(r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient></defs>
            <path d="M10 50 Q50 0 90 50" stroke="#0000ff" stroke-width="4" fill="none" stroke-dasharray="2 2"/>
            <rect x="0" y="60" width="100" height="40" fill="url(#g)"/>
        </svg>"##);
        let comp = &scene.comps[0];
        let curve = vector(&comp.layers[1]);
        assert!(!curve.path.closed && curve.fill.is_none());
        assert_eq!(curve.stroke.map(|stroke| stroke.width), Some(4.0));
        assert_ne!(curve.path.points[0].handle_out, Vec2::default(), "the curve has handles");
        let Some(Paint::Linear(gradient)) = vector(&comp.layers[0]).fill else { panic!("a linear gradient") };
        // Left edge to right edge of its own box.
        assert!((gradient.from - Vec2::new(0.0, 0.0)).length() < 0.01 && (gradient.to - Vec2::new(1.0, 0.0)).length() < 0.01, "{gradient:?}");
        assert!(scene.notes.iter().any(|note| note.contains("Dashed")));
    }

    #[test]
    fn an_svg_comes_in_as_one_composition_and_breaks_into_its_shapes_in_another() {
        use crate::model::LayerContent;
        use crate::sample::{ids, sample_project};
        let mut project = sample_project();
        let main = project.composition(ids::MAIN_COMP).unwrap();
        let (fps, seconds) = (main.frame_rate.fps(), main.duration_frames as f64 / main.frame_rate.fps());
        let scene = read(r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <path id="letter" fill="#222" d="M0 0 H50 V50 H0 Z M10 10 V40 H40 V10 Z"/>
            <circle id="dot" cx="150" cy="50" r="20" fill="#e11"/>
        </svg>"##, Path::new("/art/logo.svg"), fps, seconds).unwrap();
        let imported = super::super::build(&mut project, &scene, "logo", &|_| None).unwrap();
        assert_eq!(imported.folder, None, "no folder for a single composition");
        let svg = imported.compositions[0];
        assert_eq!(project.item(svg).unwrap().parent, None);
        let inner = project.composition(svg).unwrap();
        assert_eq!(inner.layers.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["dot", "letter"]);
        let LayerContent::Shape { path: Some(path), .. } = &inner.layers[1].content else { panic!("a path layer") };
        assert_eq!(path.compound.len(), 1, "the letter keeps its hole");

        // As one layer in the main composition, then broken into its shapes there.
        let layer = crate::edit::add_layer_from_item(&mut project, ids::MAIN_COMP, svg, 0, 0).unwrap();
        let broken = crate::nesting::ungroup(&mut project, ids::MAIN_COMP, layer).unwrap();
        assert_eq!(broken.layers.len(), 2);
        let main = project.composition(ids::MAIN_COMP).unwrap();
        assert!(main.layer(layer).is_none(), "the composition layer gave way to its shapes");
        assert!(broken.layers.iter().all(|id| matches!(main.layer(*id).unwrap().content, LayerContent::Shape { .. })));
    }

    #[test]
    fn what_cant_come_in_is_noted_and_broken_files_are_refused() {
        let scene = read_text(r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"/><text x="0" y="9">Hi</text></svg>"##);
        assert!(scene.notes.iter().any(|note| note.contains("Text")));
        assert!(read("not svg", Path::new("x.svg"), 30.0, 10.0).is_err());
        assert!(read(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#, Path::new("x.svg"), 30.0, 10.0).is_err(), "no shapes");
    }
}
