//! Small pictures of what each LUT does, for the LUT browser in the Color panel and the LUTs in
//! the Effects panel.
//!
//! The current frame is drawn small once, without the Apply LUT effect the Color panel is
//! choosing for (so the previews show each look in its place rather than on top of it). Then
//! each LUT is applied to that picture on the CPU, a few per redraw, as the GPU would. A frame
//! with nothing in it is swapped for a sample scene (sky, skin and leaves) so the looks still
//! show. Nothing is redrawn while playing or dragging; the pictures catch up afterwards.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::{Context, RenderImage, Window};
use studio_core::grade::Lut;
use studio_core::model::{Id, LutSource, Project};

use crate::workspace::{PanelId, Workspace};

/// How wide the frame is drawn for the previews, in pixels (twice the largest tile, for sharp
/// pictures on high-density screens).
const PREVIEW_WIDTH: u32 = 192;
/// LUT pictures made per redraw while they fill in.
const PICTURES_PER_FRAME: usize = 6;
/// How many times the frame is drawn again while its video is still being decoded, before the
/// previews settle for what there is.
const MAX_REDRAWS: u32 = 30;
/// The gray shown through see-through parts of the frame, as the other panel pictures use.
const BACKDROP: f32 = 46.0 / 255.0;

/// The frame the previews are made from, as straight display colors (0 to 1).
#[derive(PartialEq)]
struct Base {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 3]>,
}

/// What the previews were made from, so they are only made again when it changes.
#[derive(Clone, PartialEq)]
struct Source {
    project: Project,
    comp: Id,
    frame: i64,
}

#[derive(Default)]
pub struct LutPreviews {
    /// The project, composition, frame and left-out effect last looked at, to notice changes.
    seen: Option<(Source, Option<(Id, Id)>)>,
    /// The project (with the left-out effect removed) the frame was drawn from.
    drawn: Option<Source>,
    base: Option<Base>,
    /// Whether the frame should be drawn again (it changed, or its video was still decoding),
    /// and how many times it has been drawn for the same project and frame.
    redraw: bool,
    redraws: u32,
    pictures: HashMap<LutSource, Arc<RenderImage>>,
    /// The LUTs whose pictures were made from the current frame.
    fresh: HashSet<LutSource>,
    /// The lookup tables read so far (`None` for files that can't be read).
    tables: HashMap<LutSource, Option<Arc<Lut>>>,
}

impl LutPreviews {
    /// The picture of `source`, once it has been made.
    pub fn picture(&self, source: &LutSource) -> Option<Arc<RenderImage>> {
        self.pictures.get(source).cloned()
    }

    /// The lookup table for `source`, read the first time it is asked for.
    fn table(&mut self, source: &LutSource) -> Option<Arc<Lut>> {
        self.tables
            .entry(source.clone())
            .or_insert_with(|| match source {
                LutSource::Look(name) => studio_core::grade::look(name).map(Arc::new),
                LutSource::File(path) => studio_render::read_cube(path).ok().map(Arc::new),
            })
            .clone()
    }
}

/// The Apply LUT effect the Color panel is showing, if its tab is open: the previews leave it
/// out of the frame they are made from.
pub fn chosen_effect(workspace: &Workspace) -> Option<(Id, Id)> {
    if workspace.ui.tab(PanelId::Properties) != 2 {
        return None;
    }
    let layer = workspace.state.grade_layer()?;
    Some((layer, workspace.state.effect_named(layer, "Apply LUT")?))
}

/// Every LUT there is a preview of: Studio's looks, the imported LUTs, and the file the Color
/// panel's LUT uses when it is not in the library.
pub fn sources(workspace: &Workspace) -> Vec<LutSource> {
    let mut sources: Vec<LutSource> = studio_core::grade::looks().map(|look| LutSource::Look(look.to_owned())).collect();
    sources.extend(workspace.ui.user_luts.iter().map(|path| LutSource::File(path.clone())));
    if let Some((layer, effect)) = chosen_effect(workspace)
        && let Some(source @ LutSource::File(_)) = workspace.state.effect_lut(layer, effect)
        && !sources.contains(source)
    {
        sources.push(source.clone());
    }
    sources
}

/// Whether a panel showing LUT previews is open.
fn wanted(workspace: &Workspace) -> bool {
    workspace.showing(PanelId::Effects) || chosen_effect(workspace).is_some()
}

/// Makes the frame and a few missing LUT pictures, and asks for another redraw while any are
/// left. Called from the workspace's render.
pub fn keep_up(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let dragging = cx.has_active_drag() || workspace.ui.grade_pressed || workspace.state.is_dragging();
    if !wanted(workspace) || workspace.state.is_playing() || dragging {
        return;
    }
    notice_changes(workspace);
    if workspace.ui.lut_previews.redraw {
        draw_frame(workspace);
    }
    if workspace.ui.lut_previews.base.is_none() {
        return;
    }
    let missing: Vec<LutSource> = sources(workspace).into_iter().filter(|source| !workspace.ui.lut_previews.fresh.contains(source)).collect();
    for source in missing.iter().take(PICTURES_PER_FRAME) {
        let previews = &mut workspace.ui.lut_previews;
        previews.fresh.insert(source.clone());
        let Some(table) = previews.table(source) else { continue };
        let Some(base) = previews.base.as_ref() else { return };
        let Some(picture) = apply(&table, base) else { continue };
        if let Some(old) = previews.pictures.insert(source.clone(), picture) {
            // Free the old picture's texture, as the viewer does for its frames.
            cx.drop_image(old, Some(window));
        }
    }
    if missing.len() > PICTURES_PER_FRAME || workspace.ui.lut_previews.redraw {
        cx.on_next_frame(window, |_, _, cx| cx.notify());
    }
}

/// Checks whether the project, frame or left-out effect changed since the last look, and if the
/// change shows in the frame the previews are made from, asks for it to be drawn again.
fn notice_changes(workspace: &mut Workspace) {
    let state = &workspace.state;
    let without = chosen_effect(workspace);
    let previews = &workspace.ui.lut_previews;
    let unchanged = previews.seen.as_ref().is_some_and(|(seen, left_out)| {
        *left_out == without && seen.comp == state.active_comp && seen.frame == state.current_frame && seen.project == state.project
    });
    if unchanged {
        return;
    }
    let seen = Source { project: state.project.clone(), comp: state.active_comp, frame: state.current_frame };
    // The frame leaves out the chosen LUT, so choosing a different one (or changing its
    // strength) changes nothing in it.
    let mut drawn = seen.clone();
    if let Some((layer, effect)) = without
        && let Some(layer) = drawn.project.composition_mut(drawn.comp).and_then(|composition| composition.layer_mut(layer))
    {
        layer.effects.retain(|candidate| candidate.id != effect);
    }
    let previews = &mut workspace.ui.lut_previews;
    if previews.drawn.as_ref() != Some(&drawn) {
        previews.drawn = Some(drawn);
        previews.redraw = true;
        previews.redraws = 0;
    }
    previews.seen = Some((seen, without));
}

/// Draws the frame the previews are made from. When it comes out different from before, every
/// LUT picture is made again (the old ones stay on screen until then).
fn draw_frame(workspace: &mut Workspace) {
    let Some(drawn) = workspace.ui.lut_previews.drawn.clone() else { return };
    let Some(composition) = drawn.project.composition(drawn.comp) else { return };
    let height = ((PREVIEW_WIDTH as f32 * composition.height as f32 / composition.width.max(1) as f32).round() as u32).clamp(1, PREVIEW_WIDTH * 2);
    let Some((frame, exact)) = workspace.render.preview_frame(&drawn.project, drawn.comp, drawn.frame, PREVIEW_WIDTH, height) else {
        // Without a GPU there is nothing to show; stop asking.
        workspace.ui.lut_previews.redraw = false;
        return;
    };
    let pixels: Vec<[f32; 3]> = frame
        .bgra
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            let alpha = f32::from(pixel[3]) / 255.0;
            let over = |channel: u8| f32::from(channel) / 255.0 * alpha + BACKDROP * (1.0 - alpha);
            [over(pixel[2]), over(pixel[1]), over(pixel[0])]
        })
        .collect();
    let base = if is_blank(&pixels) { sample_scene(frame.width, frame.height) } else { Base { width: frame.width, height: frame.height, pixels } };
    let previews = &mut workspace.ui.lut_previews;
    previews.redraws += 1;
    previews.redraw = !exact && previews.redraws < MAX_REDRAWS;
    if previews.base.as_ref() != Some(&base) {
        previews.base = Some(base);
        previews.fresh.clear();
    }
}

/// Whether a frame is one flat color (an empty composition), with nothing a look could show on.
fn is_blank(pixels: &[[f32; 3]]) -> bool {
    let Some(first) = pixels.first() else { return true };
    pixels.iter().all(|pixel| (0..3).all(|c| (pixel[c] - first[c]).abs() < 0.02))
}

/// A stand-in picture for an empty frame: a sky fading from blue to warm, green ground, and a
/// skin-colored circle, so warm, cool, green and skin changes all show.
fn sample_scene(width: u32, height: u32) -> Base {
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t) };
    let (w, h) = (width as f32, height as f32);
    let pixels = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x as f32 + 0.5, y as f32 + 0.5)))
        .map(|(x, y)| {
            let (dx, dy) = ((x - w * 0.62) / h, (y - h * 0.52) / h);
            if (dx * dx + dy * dy).sqrt() < 0.18 {
                mix([0.95, 0.78, 0.65], [0.66, 0.41, 0.29], (dx * dx + dy * dy).sqrt() / 0.18)
            } else if y < h * 0.58 {
                mix([0.37, 0.66, 0.91], [0.97, 0.75, 0.54], y / (h * 0.58))
            } else {
                mix([0.30, 0.48, 0.23], [0.11, 0.16, 0.09], (y - h * 0.58) / (h * 0.42))
            }
        })
        .collect();
    Base { width, height, pixels }
}

/// The frame with `table` applied, as a picture for the screen.
fn apply(table: &Lut, base: &Base) -> Option<Arc<RenderImage>> {
    let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let bgra: Vec<u8> = base
        .pixels
        .iter()
        .flat_map(|&pixel| {
            let [r, g, b] = table.apply(pixel);
            [to_byte(b), to_byte(g), to_byte(r), 255]
        })
        .collect();
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(base.width, base.height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_frame_counts_as_blank() {
        assert!(is_blank(&[[0.0; 3]; 16]));
        assert!(is_blank(&[]));
        assert!(!is_blank(&[[0.0; 3], [0.5, 0.5, 0.5]]));
        // The sample scene is not, so its previews show the looks.
        assert!(!is_blank(&sample_scene(32, 18).pixels));
    }

    #[test]
    fn previews_show_the_lut_applied() {
        let base = Base { width: 2, height: 1, pixels: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] };
        // Swaps red and blue.
        let swap = Lut::from_fn("Swap", 5, |[r, g, b]| [b, g, r]);
        let picture = apply(&swap, &base).unwrap();
        let bytes = picture.as_bytes(0).unwrap();
        // Stored blue, green, red, alpha: the red pixel is now blue, the blue one red.
        assert_eq!(&bytes[..8], &[255, 0, 0, 255, 0, 0, 255, 255]);
    }
}
