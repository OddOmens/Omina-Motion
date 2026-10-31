//! Measures decoding speed for a clip: sequential frames (playback) and random jumps (scrubbing).
//! `cargo run -p studio-media --release --example decode_speed -- clip.mov [max_width]`

use std::time::Instant;

use studio_media::{VideoDecoder, probe};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().ok_or("usage: decode_speed <clip> [max_width]")?);
    let max_width: u32 = args.next().map_or(Ok(1920), |value| value.parse())?;
    let info = probe(&path)?;
    println!("{}: {}×{} {} at {}, {} frames", path.display(), info.width, info.height, info.codec, info.frame_rate.label(), info.frame_count);

    for use_hardware in [false, true] {
        let mut decoder = VideoDecoder::open(&path, max_width, use_hardware)?;
        println!("  {}:", decoder.backend());
        measure(&mut decoder, info.frame_count)?;
    }
    Ok(())
}

fn measure(decoder: &mut VideoDecoder, frame_count: i64) -> Result<(), Box<dyn std::error::Error>> {
    let count = frame_count.min(240);
    let started = Instant::now();
    for index in 0..count {
        decoder.frame(index)?;
    }
    let sequential = started.elapsed().as_secs_f64() * 1000.0 / count as f64;
    println!("    sequential: {sequential:.2} ms per frame ({:.0} fps)", 1000.0 / sequential);

    // Pseudo-random jumps across the clip.
    let jumps = 20;
    let started = Instant::now();
    for step in 0..jumps {
        decoder.frame((step * 7919) % frame_count)?;
    }
    println!("    random jump: {:.1} ms each", started.elapsed().as_secs_f64() * 1000.0 / jumps as f64);
    Ok(())
}
