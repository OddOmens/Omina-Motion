//! Converts rendered sRGB frames into the YUV layouts video codecs store, using BT.709
//! coefficients in limited ("TV") range: the standard for HD and UHD video, and what players
//! assume for files tagged BT.709. FFmpeg's scaler would pick its default (SD, BT.601)
//! matrix here, which shifts colors slightly, so the conversion is done here, exactly.
//!
//! Chroma is averaged over each 2×1 (4:2:2) or 2×2 (4:2:0) block. Rows are converted in
//! parallel bands on all cores.

/// The YUV layouts exports use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvLayout {
    /// 8-bit, chroma halved both ways (H.264 delivery).
    Yuv420p8,
    /// 10-bit little-endian, chroma halved horizontally (ProRes 422).
    Yuv422p10,
}

impl YuvLayout {
    fn ten_bit(self) -> bool {
        self == Self::Yuv422p10
    }

    /// Bytes per row and rows of the luma plane and of each chroma plane, tightly packed.
    pub fn plane_sizes(self, width: usize, height: usize) -> ((usize, usize), (usize, usize)) {
        let bytes = if self.ten_bit() { 2 } else { 1 };
        ((width * bytes, height), (width.div_ceil(2) * bytes, height.div_ceil(self.rows_per_chroma())))
    }

    /// Luma rows per chroma row.
    fn rows_per_chroma(self) -> usize {
        match self {
            Self::Yuv420p8 => 2,
            Self::Yuv422p10 => 1,
        }
    }
}

/// One image plane to write into: its bytes and the bytes per row.
pub struct Plane<'a> {
    pub data: &'a mut [u8],
    pub stride: usize,
}

/// BT.709 luma weights.
const KR: f32 = 0.2126;
const KB: f32 = 0.0722;
const KG: f32 = 1.0 - KR - KB;

/// Gamma-encoded R, G, B (0–1) to Y′ (0–1) and Cb, Cr (−0.5 to 0.5).
fn to_ycbcr(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let y = KR * r + KG * g + KB * b;
    (y, (b - y) / (2.0 * (1.0 - KB)), (r - y) / (2.0 * (1.0 - KR)))
}

/// Limited-range code values: luma 16–235 and chroma 16–240 at 8 bits (×4 at 10 bits).
fn luma_code(y: f32, ten_bit: bool) -> u16 {
    let (offset, range) = if ten_bit { (64.0, 876.0) } else { (16.0, 219.0) };
    (offset + range * y).round().clamp(0.0, if ten_bit { 1023.0 } else { 255.0 }) as u16
}

fn chroma_code(c: f32, ten_bit: bool) -> u16 {
    let (offset, range) = if ten_bit { (512.0, 896.0) } else { (128.0, 224.0) };
    (offset + range * c).round().clamp(0.0, if ten_bit { 1023.0 } else { 255.0 }) as u16
}

fn put(row: &mut [u8], x: usize, value: u16, ten_bit: bool) {
    if ten_bit {
        row[x * 2..x * 2 + 2].copy_from_slice(&value.to_le_bytes());
    } else {
        row[x] = value as u8;
    }
}

/// Converts a `source_width × source_height` BGRA image into `planes` (Y, Cb, Cr) of an
/// `out_width × out_height` frame. A larger frame (codecs need even sizes) repeats the
/// source's last column and row.
pub fn bgra_to_yuv(
    bgra: &[u8],
    (source_width, source_height): (usize, usize),
    (out_width, out_height): (usize, usize),
    layout: YuvLayout,
    [y_plane, u_plane, v_plane]: [Plane<'_>; 3],
) {
    let ten_bit = layout.ten_bit();
    let per_chroma = layout.rows_per_chroma();
    let chroma_rows = out_height.div_ceil(per_chroma);
    // Gamma-encoded components of the source pixel nearest to (x, y), edges repeated.
    let pixel = |x: usize, y: usize| {
        let i = (y.min(source_height - 1) * source_width + x.min(source_width - 1)) * 4;
        (f32::from(bgra[i + 2]) / 255.0, f32::from(bgra[i + 1]) / 255.0, f32::from(bgra[i]) / 255.0)
    };

    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let band = chroma_rows.div_ceil(threads).max(1);
    let (y_stride, u_stride, v_stride) = (y_plane.stride, u_plane.stride, v_plane.stride);
    let bands = y_plane
        .data
        .chunks_mut(y_stride * per_chroma * band)
        .zip(u_plane.data.chunks_mut(u_stride * band))
        .zip(v_plane.data.chunks_mut(v_stride * band))
        .enumerate();
    std::thread::scope(|scope| {
        for (index, ((y_band, u_band), v_band)) in bands {
            scope.spawn(move || {
                let first = index * band;
                for chroma_row in first..(first + band).min(chroma_rows) {
                    let local = chroma_row - first;
                    let (u_row, v_row) = (&mut u_band[local * u_stride..], &mut v_band[local * v_stride..]);
                    for chroma_x in 0..out_width.div_ceil(2) {
                        // Average the block's colors, then convert that to chroma.
                        let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
                        let mut count = 0.0;
                        for dy in 0..per_chroma {
                            let y = chroma_row * per_chroma + dy;
                            if y >= out_height {
                                continue;
                            }
                            let y_row = &mut y_band[(local * per_chroma + dy) * y_stride..];
                            for dx in 0..2 {
                                let x = chroma_x * 2 + dx;
                                if x >= out_width {
                                    continue;
                                }
                                let (pr, pg, pb) = pixel(x, y);
                                put(y_row, x, luma_code(to_ycbcr(pr, pg, pb).0, ten_bit), ten_bit);
                                (r, g, b, count) = (r + pr, g + pg, b + pb, count + 1.0);
                            }
                        }
                        let (_, cb, cr) = to_ycbcr(r / count, g / count, b / count);
                        put(u_row, chroma_x, chroma_code(cb, ten_bit), ten_bit);
                        put(v_row, chroma_x, chroma_code(cr, ten_bit), ten_bit);
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts a solid-color image and returns its first Y, Cb and Cr values.
    fn solid(bgra: [u8; 4], layout: YuvLayout) -> (u16, u16, u16) {
        let (width, height) = (4, 4);
        let image: Vec<u8> = bgra.iter().copied().cycle().take(width * height * 4).collect();
        let bytes = if layout.ten_bit() { 2 } else { 1 };
        let chroma_height = height / layout.rows_per_chroma();
        let mut y = vec![0; width * bytes * height];
        let mut u = vec![0; width / 2 * bytes * chroma_height];
        let mut v = u.clone();
        let planes = [
            Plane { data: &mut y, stride: width * bytes },
            Plane { data: &mut u, stride: width / 2 * bytes },
            Plane { data: &mut v, stride: width / 2 * bytes },
        ];
        bgra_to_yuv(&image, (width, height), (width, height), layout, planes);
        let read = |plane: &[u8]| if bytes == 2 { u16::from_le_bytes([plane[0], plane[1]]) } else { u16::from(plane[0]) };
        (read(&y), read(&u), read(&v))
    }

    #[test]
    fn matches_the_bt709_reference_values() {
        // Reference values from ITU-R BT.709 (8-bit limited range).
        assert_eq!(solid([255, 255, 255, 255], YuvLayout::Yuv420p8), (235, 128, 128));
        assert_eq!(solid([0, 0, 0, 255], YuvLayout::Yuv420p8), (16, 128, 128));
        assert_eq!(solid([0, 0, 255, 255], YuvLayout::Yuv420p8), (63, 102, 240), "red");
        assert_eq!(solid([0, 255, 0, 255], YuvLayout::Yuv420p8), (173, 42, 26), "green");
        assert_eq!(solid([255, 0, 0, 255], YuvLayout::Yuv420p8), (32, 240, 118), "blue");
    }

    #[test]
    fn ten_bit_uses_the_wider_code_range() {
        assert_eq!(solid([255, 255, 255, 255], YuvLayout::Yuv422p10), (940, 512, 512));
        assert_eq!(solid([0, 0, 0, 255], YuvLayout::Yuv422p10), (64, 512, 512));
    }

    #[test]
    fn odd_sources_fill_even_frames_by_repeating_the_edge() {
        // A 3×3 white image into a 4×4 frame: every luma sample is white.
        let image = vec![255; 3 * 3 * 4];
        let (mut y, mut u, mut v) = (vec![0; 16], vec![0; 4], vec![0; 4]);
        let planes = [Plane { data: &mut y, stride: 4 }, Plane { data: &mut u, stride: 2 }, Plane { data: &mut v, stride: 2 }];
        bgra_to_yuv(&image, (3, 3), (4, 4), YuvLayout::Yuv420p8, planes);
        assert!(y.iter().all(|&value| value == 235));
    }
}
