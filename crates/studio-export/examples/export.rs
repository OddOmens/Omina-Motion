//! Exports a project from the command line, without the editor: for render machines, and
//! for measuring export speed. Exports the project's saved composition and work area.
//!
//! cargo run -p studio-export --release --example export -- <project.studio | --sample> <out.mp4 | out.mov> [--no-audio]

use std::path::PathBuf;
use std::time::Instant;

use studio_core::project_file;
use studio_core::sample::sample_project;
use studio_export::{ExportRequest, Outcome, export};
use studio_media::MovieFormat;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(source), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("usage: export <project.studio | --sample> <out.mp4 | out.mov> [--no-audio]");
        std::process::exit(2);
    };
    let (project, composition, frames) = if source == "--sample" {
        let project = sample_project();
        let comp = project.compositions[0].id;
        let duration = project.compositions[0].duration_frames;
        (project, comp, 0..duration)
    } else {
        let text = std::fs::read_to_string(source).unwrap_or_else(|error| fail(&format!("{source}: {error}")));
        let (mut project, view) = project_file::from_text(&text).unwrap_or_else(|error| fail(&error));
        let folder = PathBuf::from(source).parent().map(PathBuf::from).unwrap_or_default();
        for missing in project_file::relink_missing_media(&mut project, &folder, |path| path.exists()) {
            eprintln!("missing media: {}", missing.display());
        }
        (project, view.active_composition, view.work_area.start as i64..view.work_area.end as i64)
    };
    let path = PathBuf::from(output);
    let format = match path.extension().and_then(|e| e.to_str()) {
        Some("mov") => MovieFormat::ProRes422Hq,
        Some("mp4") => MovieFormat::H264,
        _ => fail("the output must end in .mp4 (H.264) or .mov (ProRes 422 HQ)"),
    };
    let request = ExportRequest { include_audio: !args.iter().any(|a| a == "--no-audio"), ..ExportRequest::new(composition, frames, format, path) };

    let started = Instant::now();
    let mut last_report = Instant::now();
    let outcome = export(&project, &request, &mut |progress| {
        if last_report.elapsed().as_secs_f64() > 0.5 || progress.done == progress.total {
            eprint!("\rframe {} of {}", progress.done, progress.total);
            last_report = Instant::now();
        }
    }, &|| false)
    .unwrap_or_else(|error| fail(&error));
    let seconds = started.elapsed().as_secs_f64();
    let frames = (request.frames.end - request.frames.start) as f64;
    if outcome == Outcome::Finished {
        eprintln!("\nexported {} in {seconds:.2} s ({:.1} frames per second)", request.path.display(), frames / seconds);
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
