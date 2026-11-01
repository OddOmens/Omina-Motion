//! Runs the real model. It needs the downloaded model file, so it is skipped unless asked for:
//! `CUTOUT_MODEL=<folder with mobile_sam-tiny-vitt.safetensors> cargo test -p studio-cutout -- --ignored`

use studio_cutout::{Click, Segmenter};

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
