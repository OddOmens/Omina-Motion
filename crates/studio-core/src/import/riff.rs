//! Reading RIFX files, the big-endian chunk format After Effects projects are saved in: each
//! chunk is a four-letter id, a 32-bit length and that many bytes (padded to an even length).
//! `LIST` chunks hold a four-letter type and more chunks. Every length is checked against what
//! is actually there, so a damaged file reads as fewer chunks, never out of bounds.

/// One chunk, borrowing its bytes from the file.
#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    pub id: [u8; 4],
    /// For a `LIST`, its type; otherwise the id again.
    pub kind: [u8; 4],
    /// The chunk's contents (for a `LIST`, what follows its type).
    pub body: &'a [u8],
}

impl<'a> Chunk<'a> {
    pub fn is_list(&self, kind: &[u8; 4]) -> bool {
        &self.id == b"LIST" && &self.kind == kind
    }

    /// The chunks inside a `LIST`.
    pub fn children(&self) -> Chunks<'a> {
        Chunks { data: if &self.id == b"LIST" { self.body } else { &[] } }
    }

    /// The first child chunk with this id (not a list).
    pub fn find(&self, id: &[u8; 4]) -> Option<&'a [u8]> {
        self.children().find(|chunk| &chunk.id == id && &chunk.id != b"LIST").map(|chunk| chunk.body)
    }

    /// The first child list of this type.
    pub fn list(&self, kind: &[u8; 4]) -> Option<Chunk<'a>> {
        self.children().find(|chunk| chunk.is_list(kind))
    }
}

/// The chunks in a run of bytes, in order.
#[derive(Clone, Copy, Debug)]
pub struct Chunks<'a> {
    pub data: &'a [u8],
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Chunk<'a>> {
        if self.data.len() < 8 {
            return None;
        }
        let id: [u8; 4] = self.data[0..4].try_into().ok()?;
        let length = u32::from_be_bytes(self.data[4..8].try_into().ok()?) as usize;
        let end = 8usize.checked_add(length).filter(|&end| end <= self.data.len())?;
        let body = &self.data[8..end];
        let padded = (end + (length & 1)).min(self.data.len());
        self.data = &self.data[padded..];
        if &id == b"LIST" {
            let kind: [u8; 4] = body.get(0..4)?.try_into().ok()?;
            Some(Chunk { id, kind, body: &body[4..] })
        } else {
            Some(Chunk { id, kind: id, body })
        }
    }
}

/// Big-endian numbers at an offset, or `None` past the end.
pub fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

pub fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?)).filter(|value| value.is_finite())
}

pub fn f64_at(bytes: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?)).filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_and_lists_read_and_damage_stops_cleanly() {
        // A list holding "abcd" (3 bytes, padded) and "efgh" (2 bytes).
        let mut file = b"LIST\x00\x00\x00\x1aTtypabcd\x00\x00\x00\x03xyz\x00efgh\x00\x00\x00\x02hi".to_vec();
        let list = Chunks { data: &file }.next().unwrap();
        assert!(list.is_list(b"Ttyp"));
        assert_eq!(list.find(b"abcd"), Some(&b"xyz"[..]));
        assert_eq!(list.find(b"efgh"), Some(&b"hi"[..]));
        // A length that runs past the end reads as nothing, not a crash.
        file[7] = 0xff;
        assert!(Chunks { data: &file }.next().is_none());
        assert_eq!(u32_at(&[1, 2], 0), None);
    }
}
