//! EBML writing into memory: element IDs, sizes, leaf values and masters.
//!
//! Every size is checked against what an 8-byte EBML size can store. A
//! number takes its shortest form, and a size never takes the unknown-size
//! marker.

use crate::error::{Container, Error, Result};
use crate::mkv::ids;

/// The largest body an EBML size can state: all ones marks an unknown size.
pub(crate) const MAX_BODY: u64 = (1 << 56) - 2;
/// An 8-byte unknown-size marker.
pub(crate) const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// The error for a body no EBML size can state.
fn too_large(id: u32, size: u64) -> Error {
    Error::InvalidMux {
        container: Container::Matroska,
        reason: format!(
            "element `{}` holds {size} bytes, above the EBML limit of {MAX_BODY}. Write less data \
             into one output",
            ids::name(id)
        ),
    }
}

/// Append the bytes of `id`, marker bits included.
pub(crate) fn push_id(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let skip = (id.leading_zeros() / 8) as usize;
    out.extend_from_slice(bytes.get(skip..).unwrap_or_default());
}

/// Append the shortest size field for a body of `size` bytes in `id`.
pub(crate) fn push_size(out: &mut Vec<u8>, id: u32, size: u64) -> Result<()> {
    if size > MAX_BODY {
        return Err(too_large(id, size));
    }
    let mut len = 1usize;
    while len < 8 && size >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let bytes = (size | (1u64 << (7 * len))).to_be_bytes();
    out.extend_from_slice(bytes.get(8 - len..).unwrap_or_default());
    Ok(())
}

/// The 8-byte size field for a body of `size` bytes in `id`, to patch into a
/// reserved field.
pub(crate) fn fixed_size(id: u32, size: u64) -> Result<[u8; 8]> {
    if size > MAX_BODY {
        return Err(too_large(id, size));
    }
    Ok((size | (1u64 << 56)).to_be_bytes())
}

/// Append one element whose body is the concatenation of `parts`.
pub(crate) fn push_element(out: &mut Vec<u8>, id: u32, parts: &[&[u8]]) -> Result<()> {
    let size: u64 = parts.iter().map(|part| part.len() as u64).sum();
    push_id(out, id);
    push_size(out, id, size)?;
    for part in parts {
        out.extend_from_slice(part);
    }
    Ok(())
}

/// A `Void` element of exactly `total` bytes. `total` is 2 or more, and a
/// smaller value gives nothing, since no element fits.
pub(crate) fn void(total: usize) -> Vec<u8> {
    if total < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(total);
    out.push(ids::VOID as u8);
    if total - 2 <= 126 {
        out.push(0x80 | (total - 2) as u8);
    } else {
        let body = total - 3;
        out.push(0x40 | ((body >> 8) & 0x3F) as u8);
        out.push((body & 0xFF) as u8);
    }
    out.resize(total, 0);
    out
}

/// A buffer of whole elements.
#[derive(Debug, Default)]
pub(crate) struct ElementBuf {
    bytes: Vec<u8>,
}

impl ElementBuf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Append an element with `body`.
    pub fn binary(&mut self, id: u32, body: &[u8]) -> Result<()> {
        push_element(&mut self.bytes, id, &[body])
    }

    /// Append a UTF-8 text element.
    pub fn text(&mut self, id: u32, value: &str) -> Result<()> {
        self.binary(id, value.as_bytes())
    }

    /// Append an unsigned integer in its shortest form, one byte or more.
    pub fn uint(&mut self, id: u32, value: u64) -> Result<()> {
        let bytes = value.to_be_bytes();
        let skip = ((value.leading_zeros() / 8) as usize).min(7);
        self.binary(id, bytes.get(skip..).unwrap_or_default())
    }

    /// Append an unsigned integer in 8 bytes, so its size never depends on
    /// the value.
    pub fn fixed_uint(&mut self, id: u32, value: u64) -> Result<()> {
        self.binary(id, &value.to_be_bytes())
    }

    /// Append a signed integer in its shortest two's complement form.
    pub fn int(&mut self, id: u32, value: i64) -> Result<()> {
        let bytes = value.to_be_bytes();
        let redundant = if value < 0 {
            value.leading_ones()
        } else {
            value.leading_zeros()
        };
        // One sign bit stays, in the top bit of the first byte kept.
        let skip = ((redundant.saturating_sub(1) / 8) as usize).min(7);
        self.binary(id, bytes.get(skip..).unwrap_or_default())
    }

    /// Append a float in 8 bytes.
    pub fn float(&mut self, id: u32, value: f64) -> Result<()> {
        self.binary(id, &value.to_bits().to_be_bytes())
    }

    /// Append a master element whose children are in `children`.
    pub fn master(&mut self, id: u32, children: &ElementBuf) -> Result<()> {
        self.binary(id, children.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mkv::ebml::{Children, Vint, header, vint};

    #[test]
    fn sizes_take_the_shortest_form_and_skip_the_unknown_marker() {
        let encode = |size: u64| {
            let mut out = Vec::new();
            push_size(&mut out, ids::VOID, size).expect("the size fits");
            out
        };
        assert_eq!(encode(0), [0x80]);
        assert_eq!(encode(126), [0xFE]);
        assert_eq!(encode(127), [0x40, 0x7F]);
        assert_eq!(encode(16_382), [0x7F, 0xFE]);
        assert_eq!(encode(16_383), [0x20, 0x3F, 0xFF]);
        assert_eq!(vint(&encode(MAX_BODY)), Vint::Value(MAX_BODY, 8));
        let mut out = Vec::new();
        assert!(push_size(&mut out, ids::VOID, MAX_BODY + 1).is_err());
    }

    #[test]
    fn fixed_sizes_read_back() {
        let bytes = fixed_size(ids::SEGMENT, 300).expect("the size fits");
        assert_eq!(vint(&bytes), Vint::Value(300, 8));
        assert!(fixed_size(ids::SEGMENT, MAX_BODY + 1).is_err());
    }

    #[test]
    fn void_has_the_exact_length_at_both_size_forms() {
        for total in [2usize, 3, 128, 129, 131, 300] {
            let bytes = void(total);
            assert_eq!(bytes.len(), total);
            let parsed = header(&bytes, 0, "t").ok().flatten().expect("a header");
            assert_eq!(parsed.id, ids::VOID);
            assert_eq!(
                parsed.len + parsed.size.expect("a known size"),
                total as u64
            );
        }
        assert!(void(1).is_empty());
    }

    #[test]
    fn leaves_read_back_through_the_reader() {
        let mut buf = ElementBuf::new();
        for &value in &[0u64, 1, 255, 256, u64::MAX] {
            buf.uint(ids::TRACK_NUMBER, value).expect("the value fits");
        }
        for &value in &[0i64, 1, -1, 127, 128, -128, -129, i64::MIN, i64::MAX] {
            buf.int(ids::REFERENCE_BLOCK, value)
                .expect("the value fits");
        }
        buf.float(ids::DURATION, 1.5).expect("the value fits");
        buf.text(ids::TITLE, "t").expect("the value fits");
        let bytes = buf.into_bytes();
        let children: Vec<_> = Children::new(&bytes, 0, "t")
            .map(|child| child.expect("the element is whole"))
            .collect();
        let uints: Vec<u64> = children
            .iter()
            .filter(|child| child.id == ids::TRACK_NUMBER)
            .map(|child| child.uint("t").expect("an integer"))
            .collect();
        assert_eq!(uints, vec![0, 1, 255, 256, u64::MAX]);
        let ints: Vec<i64> = children
            .iter()
            .filter(|child| child.id == ids::REFERENCE_BLOCK)
            .map(|child| child.int("t").expect("an integer"))
            .collect();
        assert_eq!(
            ints,
            vec![0, 1, -1, 127, 128, -128, -129, i64::MIN, i64::MAX]
        );
        let lengths: Vec<usize> = children
            .iter()
            .filter(|child| child.id == ids::REFERENCE_BLOCK)
            .map(|child| child.body.len())
            .collect();
        assert_eq!(lengths, vec![1, 1, 1, 1, 2, 1, 2, 8, 8]);
    }
}
