//! Renders one frame of the sample project to a PNG, without any window:
//! `cargo run -p studio-render --release --example render_frame -- out.png [frame] [width]`

use studio_core::sample::{ids, sample_project};
use studio_render::Renderer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: render_frame <out.png> [frame] [width]")?;
    let frame: i64 = args.next().map_or(Ok(0), |value| value.parse())?;
    let width: u32 = args.next().map_or(Ok(1920), |value| value.parse())?;

    let project = sample_project();
    let comp = project.composition(ids::MAIN_COMP).ok_or("the sample composition is missing")?;
    let height = (width as f32 * comp.height as f32 / comp.width as f32).round() as u32;

    let mut renderer = Renderer::new()?;
    let started = std::time::Instant::now();
    let rendered = renderer.render(&project, ids::MAIN_COMP, frame, width, height, &studio_core::media::NoVideo)?;
    println!("frame {frame} at {width}×{height}: first render {:.1} ms (includes shader and text setup)", started.elapsed().as_secs_f64() * 1000.0);
    // Steady state, as during playback: render the following frames.
    let runs = 60;
    let started = std::time::Instant::now();
    for offset in 1..=runs {
        renderer.render(&project, ids::MAIN_COMP, (frame + offset) % comp.duration_frames, width, height, &studio_core::media::NoVideo)?;
    }
    println!("steady state: {:.2} ms per frame", started.elapsed().as_secs_f64() * 1000.0 / runs as f64);

    // The renderer returns BGRA; PNG wants RGBA.
    let mut rgba = rendered.bgra;
    for pixel in rgba.chunks_mut(4) {
        pixel.swap(0, 2);
    }
    image::save_buffer(&path, &rgba, width, height, image::ExtendedColorType::Rgba8)?;
    println!("saved {path}");
    Ok(())
}
