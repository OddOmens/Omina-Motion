//! Renders every template side by side into one PNG, without any window, to judge the designs
//! at a glance: `cargo run -p studio-render --release --example template_sheet -- out.png [wide|portrait|vertical]`

use studio_core::model::Project;
use studio_core::animation::Vec2;
use studio_core::templates::{Format, PhotoKind, SamplePhoto, TEMPLATES, build, fill_photo_spots};
use studio_render::Renderer;

/// The sample photos in the repository's `assets/template-photos`, by the kind their file names
/// start with ("portrait-1.jpg"), the same pictures the app builds in.
fn sample_photos() -> Vec<SamplePhoto> {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/template-photos");
    let mut paths: Vec<_> = std::fs::read_dir(folder).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let kind = match path.file_stem()?.to_str()?.split('-').next()? {
                "portrait" => PhotoKind::Portrait,
                "product" => PhotoKind::Product,
                "scene" => PhotoKind::Scene,
                "art" => PhotoKind::Art,
                _ => return None,
            };
            let (width, height) = image::image_dimensions(&path).ok()?;
            Some(SamplePhoto { kind, path, size: Vec2::new(width as f32, height as f32) })
        })
        .collect()
}

/// Templates across one row of the sheet, and the space between pictures in pixels.
const COLUMNS: usize = 12;
const GAP: usize = 12;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: template_sheet <out.png> [wide|portrait|vertical]")?;
    let format = match args.next().as_deref() {
        Some("wide") => Format::Wide,
        Some("portrait") => Format::Portrait,
        _ => Format::Vertical,
    };
    let (full_width, full_height) = format.size();
    let width = 270usize;
    let height = width * full_height as usize / full_width as usize;
    let rows = TEMPLATES.len().div_ceil(COLUMNS);
    let (sheet_width, sheet_height) = (COLUMNS * (width + GAP) + GAP, rows * (height + GAP) + GAP);
    let mut sheet = vec![40u8; sheet_width * sheet_height * 4];

    let photos = sample_photos();
    let mut renderer = Renderer::new()?;
    for (index, template) in TEMPLATES.iter().enumerate() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template, format).ok_or("a template could not be built")?;
        fill_photo_spots(&mut project, comp, &photos);
        // The same moment the Templates panel pictures: most of the way in.
        let frame = ((template.seconds * 0.55).min(7.5) * 30.0) as i64;
        let mut picture = renderer.render(&project, comp, frame, width as u32, height as u32, &studio_core::media::NoVideo)?;
        // Text boxes are fitted to their measured words, then the frame is drawn again.
        let composition = project.composition_mut(comp).ok_or("the template's composition is missing")?;
        for (layer, size) in std::mem::take(&mut picture.text_sizes) {
            studio_core::edit::set_text_box(composition, layer, size);
        }
        let picture = renderer.render(&project, comp, frame, width as u32, height as u32, &studio_core::media::NoVideo)?;
        let (left, top) = (GAP + (index % COLUMNS) * (width + GAP), GAP + (index / COLUMNS) * (height + GAP));
        for y in 0..height {
            for x in 0..width {
                let pixel = &picture.bgra[(y * width + x) * 4..][..4];
                let alpha = f32::from(pixel[3]) / 255.0;
                let over = |channel: u8| (f32::from(channel) * alpha + 52.0 * (1.0 - alpha)) as u8;
                let at = ((top + y) * sheet_width + left + x) * 4;
                // The renderer returns BGRA; PNG wants RGBA.
                sheet[at..at + 4].copy_from_slice(&[over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]);
            }
        }
    }
    image::save_buffer(&path, &sheet, sheet_width as u32, sheet_height as u32, image::ExtendedColorType::Rgba8)?;
    println!("saved {path}");
    Ok(())
}
