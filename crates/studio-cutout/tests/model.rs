//! Runs the real model. It needs the downloaded model file, so it is skipped unless asked for:
//! `CUTOUT_MODEL=<folder with mobile_sam-tiny-vitt.safetensors> cargo test -p studio-cutout -- --ignored`
//! Optionally, `CUTOUT_CLIP=<folder of numbered PNG or JPEG frames>` also follows the subject at the
//! middle of the first frame through real footage and saves the masks beside the system's
//! temporary folder, for looking at.

use studio_cutout::{Click, Mask, Segmenter, Stroke};

/// A 640 × 360 picture: a dark gray background with a bright orange disc on the left and a blue
/// square on the right.
fn picture(disc_x: f32) -> Vec<u8> {
    let (width, height) = (640u32, 360u32);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32, y as f32);
            let in_disc = (fx - disc_x).hypot(fy - 180.0) < 90.0;
            let in_square = (430.0..570.0).contains(&fx) && (110.0..250.0).contains(&fy);
            let color = if in_disc { [250, 140, 30] } else if in_square { [40, 90, 230] } else { [45, 45, 48] };
            rgba.extend([color[0], color[1], color[2], 255]);
        }
    }
    rgba
}

#[test]
#[ignore = "needs the downloaded model; set CUTOUT_MODEL"]
fn a_click_picks_out_the_object_and_it_is_followed_as_it_moves() {
    let folder = std::env::var("CUTOUT_MODEL").expect("CUTOUT_MODEL names the model folder");
    let segmenter = Segmenter::load(std::path::Path::new(&folder)).unwrap();
    let started = std::time::Instant::now();
    let first = segmenter.prepare(&picture(180.0), 640, 360).unwrap();
    let mask = segmenter.segment(&first, &[Click { x: 180.0 / 640.0, y: 0.5, keep: true }]).unwrap();
    println!("first frame in {:?}", started.elapsed());
    let at = |mask: &studio_cutout::Mask, x: u32, y: u32| mask.alpha[(y * mask.width + x) as usize];
    // The work size is 1024 wide here, so picture pixels scale by 1.6.
    let scale = mask.width as f32 / 640.0;
    let px = |x: f32, y: f32| ((x * scale) as u32, (y * scale) as u32);
    let (cx, cy) = px(180.0, 180.0);
    assert!(at(&mask, cx, cy) > 200, "the disc's middle is kept");
    let (sx, sy) = px(500.0, 180.0);
    assert!(at(&mask, sx, sy) < 50, "the square is not");
    let (bx, by) = px(20.0, 20.0);
    assert!(at(&mask, bx, by) < 50, "nor the background");

    // The disc moves right by 60 pixels; the next frame follows it.
    let started = std::time::Instant::now();
    let next = segmenter.prepare(&picture(240.0), 640, 360).unwrap();
    let followed = segmenter.follow(&next, &mask, &[]).unwrap();
    println!("next frame in {:?}", started.elapsed());
    let (nx, ny) = px(300.0, 180.0);
    assert!(at(&followed, nx, ny) > 200, "the moved disc's right side is kept");
    let (ox, oy) = px(110.0, 180.0);
    assert!(at(&followed, ox, oy) < 50, "where it was is not");
    assert!(at(&followed, sx, sy) < 50, "and still not the square");
}

/// A 640 × 360 frame of a figure (a head and a body) at `figure_x` on a textured, noisy
/// background, beside a box of a similar colour. Returns the RGBA picture and the figure's true
/// mask.
fn figure_frame(figure_x: f32, seed: u32) -> (Vec<u8>, Mask) {
    let (width, height) = (640u32, 360u32);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    let mut alpha = Vec::with_capacity((width * height) as usize);
    let mut noise_state = seed.wrapping_mul(2_654_435_761) | 1;
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32, y as f32);
            // A small xorshift generator: the same grain every run.
            noise_state ^= noise_state << 13;
            noise_state ^= noise_state >> 17;
            noise_state ^= noise_state << 5;
            let noise = (noise_state % 21) as f32 - 10.0;
            let figure = (fx - figure_x).hypot(fy - 110.0) < 38.0 || ((fx - figure_x).abs() < 45.0 && (150.0..330.0).contains(&fy));
            let near_twin = (360.0..470.0).contains(&fx) && (170.0..320.0).contains(&fy);
            let texture = 60.0 + 25.0 * ((fx / 13.0).sin() * (fy / 17.0).cos());
            let color = if figure { [225.0, 130.0, 60.0] } else if near_twin { [200.0, 120.0, 70.0] } else { [texture, texture * 0.9, texture * 0.8] };
            rgba.extend(color.map(|c: f32| (c + noise).clamp(0.0, 255.0) as u8));
            rgba.push(255);
            alpha.push(if figure { 255 } else { 0 });
        }
    }
    (rgba, Mask { width, height, alpha })
}

/// `mask` scaled to `width × height`, nearest pixel, to compare with a mask of that size.
fn resized(mask: &Mask, width: u32, height: u32) -> Mask {
    let alpha = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| mask.alpha[((y * mask.height / height) * mask.width + x * mask.width / width) as usize]).collect();
    Mask { width, height, alpha }
}

#[test]
#[ignore = "needs the downloaded model; set CUTOUT_MODEL"]
fn strokes_pick_out_a_figure_and_propagation_follows_it() {
    let folder = std::env::var("CUTOUT_MODEL").expect("CUTOUT_MODEL names the model folder");
    let segmenter = Segmenter::load(std::path::Path::new(&folder)).unwrap();

    // One stroke down the middle of the figure picks out all of it, not a sliver.
    let (picture, truth) = figure_frame(120.0, 0);
    let prepared = segmenter.prepare(&picture, 640, 360).unwrap();
    let stroke = Stroke { points: vec![(120.0 / 640.0, 110.0 / 360.0), (120.0 / 640.0, 300.0 / 360.0)], radius: 0.01, add: true };
    let first = segmenter.segment_strokes(&prepared, std::slice::from_ref(&stroke), None).unwrap();
    let first_truth = resized(&truth, first.width, first.height);
    assert!(first.overlap(&first_truth) > 0.9, "the whole figure: {}", first.overlap(&first_truth));

    // Refining a head-only mask with a stroke over the body grows it to the whole figure, and a
    // remove stroke over the head takes it away again.
    let head_only = Mask { alpha: first_truth.alpha.iter().enumerate().map(|(i, &a)| if (i as u32 / first.width) as f32 > first.height as f32 * 140.0 / 360.0 { 0 } else { a }).collect(), ..first_truth.clone() };
    let body = Stroke { points: vec![(120.0 / 640.0, 180.0 / 360.0), (120.0 / 640.0, 310.0 / 360.0)], radius: 0.015, add: true };
    let grown = segmenter.segment_strokes(&prepared, &[body], Some(&head_only)).unwrap();
    assert!(grown.overlap(&first_truth) > 0.9, "grown to the figure: {}", grown.overlap(&first_truth));
    let head = Stroke { points: vec![(120.0 / 640.0, 110.0 / 360.0)], radius: 0.07, add: false };
    let headless = segmenter.segment_strokes(&prepared, &[head], Some(&grown)).unwrap();
    let at = |mask: &Mask, x: f32, y: f32| mask.alpha[((y / 360.0 * mask.height as f32) as u32 * mask.width + (x / 640.0 * mask.width as f32) as u32) as usize];
    assert_eq!(at(&headless, 120.0, 110.0), 0, "the remove stroke is obeyed");
    assert!(at(&headless, 120.0, 250.0) > 200, "the body stays");

    // The figure walks right 14 pixels a frame; propagation keeps it until it reaches the
    // similar box beside it.
    let mut previous = first;
    for frame in 1..18 {
        let (picture, truth) = figure_frame(120.0 + frame as f32 * 14.0, frame);
        let prepared = segmenter.prepare(&picture, 640, 360).unwrap();
        previous = segmenter.propagate(&prepared, &previous).unwrap();
        let likeness = previous.overlap(&resized(&truth, previous.width, previous.height));
        assert!(likeness > 0.9, "frame {frame}: {likeness}");
    }

    if let Ok(clip) = std::env::var("CUTOUT_CLIP") {
        follow_through_clip(&segmenter, std::path::Path::new(&clip));
    }
}

/// Follows the subject at the middle of a real clip's first frame through the rest, printing how
/// much of each frame is kept and how much each mask overlaps the one before (a steady subject
/// gives steady numbers), and saving the masks.
fn follow_through_clip(segmenter: &Segmenter, folder: &std::path::Path) {
    let mut frames: Vec<_> = std::fs::read_dir(folder).unwrap().filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.extension().is_some_and(|extension| ["png", "jpg", "jpeg"].contains(&extension.to_ascii_lowercase().to_str().unwrap_or("")))).collect();
    frames.sort();
    let output = std::env::temp_dir().join("studio-cutout-clip");
    std::fs::create_dir_all(&output).unwrap();
    let mut previous: Option<Mask> = None;
    for (index, path) in frames.iter().enumerate() {
        let picture = image::open(path).unwrap().to_rgba8();
        let prepared = segmenter.prepare(picture.as_raw(), picture.width(), picture.height()).unwrap();
        let mask = match &previous {
            Some(previous) => segmenter.propagate(&prepared, previous).unwrap(),
            None => segmenter.segment_strokes(&prepared, &[Stroke { points: vec![(0.5, 0.35), (0.5, 0.65)], radius: 0.01, add: true }], None).unwrap(),
        };
        let steadiness = previous.as_ref().map_or(1.0, |previous| mask.overlap(previous));
        println!("frame {index}: {:.1}% kept, overlap with the previous frame {steadiness:.3}", mask.coverage() * 100.0);
        mask.save(&output.join(format!("{index:05}.png"))).unwrap();
        previous = Some(mask);
    }
    println!("masks saved in {}", output.display());
}
