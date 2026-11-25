//! Writes one decoded frame of a video as a raw RGBA file, to compare colours with other players:
//!     cargo run -p studio-media --example frame_dump -- <video> <frame> <out.rgba>
//! Prints the frame's width and height.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, video, frame, out] = args.as_slice() else { panic!("usage: frame_dump <video> <frame> <out.rgba>") };
    let mut decoder = studio_media::VideoDecoder::open(std::path::Path::new(video), 4096, false).expect("opens");
    let frame = decoder.frame(frame.parse().expect("a frame number")).expect("decodes");
    std::fs::write(out, &frame.rgba).expect("writes");
    println!("{} {}", frame.width, frame.height);
}
