//! Small encoders the formats need: base64 (for pictures embedded as data URLs) and a zip
//! archive (a dotLottie file is a zip). Both are short enough to write here; the compression
//! inside the zip is miniz_oxide's deflate.

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` in standard base64, padded with `=`.
pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let bits = (u32::from(triple[0]) << 16) | (u32::from(triple[1]) << 8) | u32::from(triple[2]);
        let sextets = [(bits >> 18) & 63, (bits >> 12) & 63, (bits >> 6) & 63, bits & 63];
        // One input byte makes two output characters, two make three, three make four.
        for (index, sextet) in sextets.iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(BASE64_ALPHABET[*sextet as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A data URL for an embedded file.
pub(crate) fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64(bytes))
}

/// The CRC-32 checksum zip files use (the IEEE polynomial, bit-reversed).
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                crc ^= 0xedb8_8320;
            }
        }
    }
    !crc
}

/// A zip archive holding `files` (name, contents), each deflated. Names are marked as UTF-8.
pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    // The zip format's date for 1 January 1980, its earliest, so the file is the same every time.
    const DOS_DATE: u16 = (1 << 5) | 1;
    const UTF8_NAMES: u16 = 1 << 11;
    const DEFLATED: u16 = 8;
    let mut out = Vec::new();
    let mut directory = Vec::new();
    let le16 = |buffer: &mut Vec<u8>, value: u16| buffer.extend_from_slice(&value.to_le_bytes());
    let le32 = |buffer: &mut Vec<u8>, value: u32| buffer.extend_from_slice(&value.to_le_bytes());
    for (name, contents) in files {
        let offset = out.len() as u32;
        let packed = miniz_oxide::deflate::compress_to_vec(contents, 9);
        let (crc, packed_size, size, name_length) = (crc32(contents), packed.len() as u32, contents.len() as u32, name.len() as u16);
        // The local header, then the file itself.
        le32(&mut out, 0x0403_4b50);
        for value in [20, UTF8_NAMES, DEFLATED, 0, DOS_DATE] {
            le16(&mut out, value);
        }
        for value in [crc, packed_size, size] {
            le32(&mut out, value);
        }
        le16(&mut out, name_length);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        // Its entry in the central directory, written after all the files.
        le32(&mut directory, 0x0201_4b50);
        for value in [20, 20, UTF8_NAMES, DEFLATED, 0, DOS_DATE] {
            le16(&mut directory, value);
        }
        for value in [crc, packed_size, size] {
            le32(&mut directory, value);
        }
        for value in [name_length, 0, 0, 0, 0] {
            le16(&mut directory, value);
        }
        le32(&mut directory, 0);
        le32(&mut directory, offset);
        directory.extend_from_slice(name.as_bytes());
    }
    let (directory_offset, directory_size, count) = (out.len() as u32, directory.len() as u32, files.len() as u16);
    out.extend_from_slice(&directory);
    // The end-of-directory record, which readers look for first.
    le32(&mut out, 0x0605_4b50);
    for value in [0, 0, count, count] {
        le16(&mut out, value);
    }
    le32(&mut out, directory_size);
    le32(&mut out, directory_offset);
    le16(&mut out, 0);
    out
}

/// Reads back the files of a zip written by `zip`, for the tests.
#[cfg(test)]
pub(crate) fn unzip(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let read16 = |at: usize| u16::from_le_bytes([archive[at], archive[at + 1]]) as usize;
    let read32 = |at: usize| u32::from_le_bytes([archive[at], archive[at + 1], archive[at + 2], archive[at + 3]]) as usize;
    let end = archive.len() - 22;
    assert_eq!(read32(end), 0x0605_4b50, "end-of-directory record");
    let (count, mut at) = (read16(end + 10), read32(end + 16));
    let mut files = Vec::new();
    for _ in 0..count {
        assert_eq!(read32(at), 0x0201_4b50, "directory entry");
        let (crc, packed_size, size, name_length, offset) = (read32(at + 16), read32(at + 20), read32(at + 24), read16(at + 28), read32(at + 42));
        let name = String::from_utf8(archive[at + 46..at + 46 + name_length].to_vec()).unwrap();
        let data_at = offset + 30 + read16(offset + 26) + read16(offset + 28);
        let contents = miniz_oxide::inflate::decompress_to_vec(&archive[data_at..data_at + packed_size]).expect("deflated");
        assert_eq!(contents.len(), size, "size of {name}");
        assert_eq!(crc32(&contents) as usize, crc, "checksum of {name}");
        files.push((name, contents));
        at += 46 + name_length;
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_examples() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(data_url("image/png", &[0xff, 0x00]), "data:image/png;base64,/wA=");
    }

    #[test]
    fn checksums_match_the_zip_standard() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_zip_reads_back_as_the_files_put_in() {
        let archive = zip(&[("manifest.json", b"{}"), ("animations/a.json", "{\"nm\":\"é\"}".as_bytes())]);
        let files = unzip(&archive);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], ("manifest.json".to_owned(), b"{}".to_vec()));
        assert_eq!(files[1].0, "animations/a.json");
    }
}
