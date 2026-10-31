//! Builds a proxy for a clip and reports how long it took:
//! `cargo run -p studio-media --release --example build_proxy -- clip.mp4 out.mov`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = std::path::PathBuf::from(args.next().ok_or("usage: build_proxy <clip> <out.mov>")?);
    let output = std::path::PathBuf::from(args.next().ok_or("usage: build_proxy <clip> <out.mov>")?);
    let info = studio_media::probe(&source)?;
    let started = std::time::Instant::now();
    studio_media::build_proxy(&source, &output, studio_media::PROXY_WIDTH, &|_| {}, &|| false)?;
    let seconds = started.elapsed().as_secs_f64();
    println!("{} seconds of {}×{} {} → proxy in {seconds:.2} s ({:.1}× real time)", info.duration_seconds, info.width, info.height, info.codec, info.duration_seconds / seconds);
    Ok(())
}
