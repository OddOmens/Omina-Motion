//! Exports to the vector formats through `export`, with this machine's fonts, and checks the
//! files written and the notice of what was left out.

use std::path::PathBuf;

use studio_core::model::Project;
use studio_core::sample::{ids, sample_project};
use studio_export::{ExportFormat, ExportRequest, Outcome, Progress, export};

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-export-vector-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

#[test]
fn the_sample_project_exports_to_every_vector_format() {
    let project = sample_project();
    for (format, name) in [(ExportFormat::Lottie, "sample.json"), (ExportFormat::DotLottie, "sample.lottie"), (ExportFormat::AnimatedSvg, "sample.svg")] {
        let path = scratch(name);
        let mut reports = Vec::new();
        let request = ExportRequest::new(ids::MAIN_COMP, 0..48, format, path.clone());
        let outcome = export(&project, &request, &mut |progress| reports.push(progress), &|| false).unwrap();
        let Outcome::Finished { left_out } = outcome else { panic!("finished") };
        // The sample has footage, an adjustment layer and sound, none of which the formats hold.
        assert!(left_out.iter().any(|line| line.starts_with("Video layers were skipped") && line.contains("B-roll")), "{left_out:?}");
        assert!(left_out.iter().any(|line| line.starts_with("Adjustment layers were left out")), "{left_out:?}");
        assert_eq!(reports.last(), Some(&Progress { done: 48, total: 48 }));
        let bytes = std::fs::read(&path).unwrap();
        assert!(!studio_export::temporary_path(&path).exists(), "no partial file left behind");
        match format {
            ExportFormat::Lottie => {
                let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!((json["w"].as_i64(), json["h"].as_i64()), (Some(1920), Some(1080)));
                // With real fonts, the title is outlined: a shape layer of letter paths.
                let title = json["layers"].as_array().unwrap().iter().find(|layer| layer["nm"] == "Title").expect("the title layer");
                assert_eq!(title["ty"], 4);
                assert!(title.to_string().contains("\"ty\":\"sh\""), "letters as paths");
            }
            ExportFormat::DotLottie => assert_eq!(&bytes[..4], b"PK\x03\x04"),
            _ => assert!(String::from_utf8(bytes).unwrap().starts_with("<svg")),
        }
    }
}

#[test]
fn a_cancelled_vector_export_writes_nothing() {
    let path = scratch("cancelled.json");
    let request = ExportRequest::new(ids::MAIN_COMP, 0..48, ExportFormat::Lottie, path.clone());
    let outcome = export(&sample_project(), &request, &mut |_| {}, &|| true).unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(!path.exists());
}

/// Writes chosen templates as Lottie, dotLottie and SVG into the folder `VECTOR_OUT`, with this
/// machine's fonts, to look at in a player. `TEMPLATE_PICK` lists template numbers ("0,5,10").
/// Run with `VECTOR_OUT=/some/folder cargo test -p studio-export --test vector vector_files -- --ignored`.
#[test]
#[ignore = "writes files for a visual check"]
fn vector_files() {
    use studio_core::templates::{Format, TEMPLATES, build};
    let folder = PathBuf::from(std::env::var("VECTOR_OUT").expect("set VECTOR_OUT to a folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let picks: Vec<usize> = std::env::var("TEMPLATE_PICK").unwrap_or_else(|_| "0".into()).split(',').filter_map(|part| part.trim().parse().ok()).collect();
    for index in picks.into_iter().filter(|&index| index < TEMPLATES.len()) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, &TEMPLATES[index], Format::Wide).unwrap();
        // Text boxes fitted to their words, as the app does after its first render.
        let mut text = studio_render::text::TextRenderer::new();
        let composition = project.composition_mut(comp).unwrap();
        let measured: Vec<_> = composition
            .layers
            .iter()
            .filter_map(|layer| match &layer.content {
                studio_core::model::LayerContent::Text { text: words, style } => Some((layer.id, text.measure(words, style))),
                _ => None,
            })
            .collect();
        for (layer, size) in measured {
            studio_core::edit::set_text_box(composition, layer, size);
        }
        let duration = composition.duration_frames;
        for format in [ExportFormat::Lottie, ExportFormat::DotLottie, ExportFormat::AnimatedSvg] {
            let path = folder.join(format!("template-{index}.{}", format.extension().unwrap()));
            let outcome = export(&project, &ExportRequest::new(comp, 0..duration, format, path.clone()), &mut |_| {}, &|| false).unwrap();
            if let Outcome::Finished { left_out } = outcome {
                eprintln!("{}: {left_out:?}", path.display());
            }
        }
    }
}
