//! Times each stage of segmenting one video frame with the real model, to see where the time
//! goes. Needs the downloaded model, so it only runs when asked:
//! `CUTOUT_MODEL=<model folder> cargo test -p studio-cutout --release --test speed -- --ignored --nocapture`

use std::path::Path;
use std::time::Instant;

use studio_cutout::{Click, Segmenter};

#[test]
#[ignore = "needs the downloaded model; set CUTOUT_MODEL"]
fn time_each_stage() {
    let folder = std::env::var("CUTOUT_MODEL").unwrap();
    let segmenter = Segmenter::load(Path::new(&folder)).unwrap();
    let (width, height) = (1024u32, 576u32);
    // An orange disc on a dark, slightly textured background.
    let picture: Vec<u8> = (0..width * height)
        .flat_map(|i| {
            let (x, y) = ((i % width) as f32, (i / width) as f32);
            if (x - 400.0).hypot(y - 288.0) < 120.0 { [255, 140, 0, 255] } else { [30, (30.0 + 20.0 * (x / 9.0).sin()) as u8, 40, 255] }
        })
        .collect();
    let click = [Click { x: 400.0 / 1024.0, y: 0.5, keep: true }];
    // Warm up (the first run compiles GPU kernels).
    let prepared = segmenter.prepare(&picture, width, height).unwrap();
    let first = segmenter.segment(&prepared, &click).unwrap();
    let runs = 5;
    let started = Instant::now();
    for _ in 0..runs {
        let prepared = segmenter.prepare(&picture, width, height).unwrap();
        // `prepare` returns before the GPU finishes; segmenting reads the result back.
        segmenter.segment(&prepared, &click).unwrap();
    }
    let whole = started.elapsed() / runs;
    let prepared = segmenter.prepare(&picture, width, height).unwrap();
    segmenter.segment(&prepared, &click).unwrap();
    let started = Instant::now();
    for _ in 0..runs {
        segmenter.follow(&prepared, &first, &[]).unwrap();
    }
    let follow = started.elapsed() / runs;
    let started = Instant::now();
    let path = std::env::temp_dir().join("studio-cutout-speed.png");
    for _ in 0..runs {
        first.save(&path).unwrap();
    }
    let save = started.elapsed() / runs;
    println!("encode + one mask: {whole:?}; following on a prepared frame: {follow:?}; saving the mask: {save:?}");
}
