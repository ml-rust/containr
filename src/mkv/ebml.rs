//! EBML primitives: variable-size integers, element headers, the children
//! of an element held in memory, and leaf values.
//!
//! Every read checks its length against the bytes it holds. Nothing here
//! allocates from a size the input declares: a body is a slice of bytes
//! already read.

use crate::error::{Error, Result};
use crate::mkv::ids;

/// The longest element ID this reader accepts, in bytes (`EBMLMaxIDLength`).
pub(crate) const MAX_ID_LEN: usize = 4;
/// The longest element size this reader accepts, in bytes
/// (`EBMLMaxSizeLength`).
pub(crate) const MAX_SIZE_LEN: usize = 8;

/// An element header: the ID with its marker bits, and the body size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    /// The element ID, marker bits included.
    pub id: u32,
    /// The body size in bytes. `None` for the unknown-size marker.
    pub size: Option<u64>,
    /// The header length in bytes.
    pub len: u64,
}

impl Header {
    /// The body size. An unknown size is an error, since only Segment and
    /// Cluster can have one. `path` and `offset` locate the element.
    pub fn known_size(&self, path: &str, offset: u64) -> Result<u64> {
        self.size.ok_or_else(|| {
            Error::mkv_malformed(
                path,
                offset,
                "only Segment and Cluster can have an unknown size",
            )
        })
    }
}

/// The outcome of decoding a variable-size integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Vint {
    /// The value without its marker bits, and the encoded length.
    Value(u64, usize),
    /// The bytes end before the integer does.
    Short,
    /// The first byte is 0, which starts no integer of 8 bytes or fewer.
    Invalid,
}

/// Decode a variable-size integer: an unsigned value with its length
/// marker removed.
pub(crate) fn vint(bytes: &[u8]) -> Vint {
    let Some(&first) = bytes.first() else {
        return Vint::Short;
    };
    if first == 0 {
        return Vint::Invalid;
    }
    let len = first.leading_zeros() as usize + 1;
    let Some(all) = bytes.get(..len) else {
        return Vint::Short;
    };
    let mask = (0xFFu32 >> len) as u8;
    let value = all
        .iter()
        .skip(1)
        .fold(u64::from(first & mask), |acc, &byte| {
            (acc << 8) | u64::from(byte)
        });
    Vint::Value(value, len)
}

/// The value of an all-ones variable-size integer of `len` bytes: the
/// unknown-size marker.
fn all_ones(len: usize) -> u64 {
    (1u64 << (7 * len)) - 1
}

/// Decode the element header at the start of `bytes`. Returns `None` when
/// the bytes end before the header does.
pub(crate) fn header(bytes: &[u8], offset: u64, path: &str) -> Result<Option<Header>> {
    let Some(&first) = bytes.first() else {
        return Ok(None);
    };
    let id_len = first.leading_zeros() as usize + 1;
    if first == 0 || id_len > MAX_ID_LEN {
        return Err(Error::mkv_malformed(
            path,
            offset,
            format!(
                "an element ID starts with byte {first:#04x}; an ID holds 1 to {MAX_ID_LEN} bytes"
            ),
        ));
    }
    let Some(id_bytes) = bytes.get(..id_len) else {
        return Ok(None);
    };
    let id = id_bytes
        .iter()
        .fold(0u32, |acc, &byte| (acc << 8) | u32::from(byte));
    let rest = bytes.get(id_len..).unwrap_or_default();
    let (size, size_len) = match vint(rest) {
        Vint::Short => return Ok(None),
        Vint::Invalid => {
            return Err(Error::mkv_malformed(
                format!("{path}/{}", ids::name(id)),
                offset,
                format!(
                    "the element size starts with byte 0x00; a size holds 1 to {MAX_SIZE_LEN} bytes"
                ),
            ));
        }
        Vint::Value(value, len) => (value, len),
    };
    Ok(Some(Header {
        id,
        size: (size != all_ones(size_len)).then_some(size),
        len: (id_len + size_len) as u64,
    }))
}

/// One child element whose body is in memory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Element<'a> {
    /// The element ID.
    pub id: u32,
    /// The byte offset of the element header in the input.
    pub offset: u64,
    /// The byte offset of the body in the input.
    pub body_offset: u64,
    /// The body.
    pub body: &'a [u8],
}

impl<'a> Element<'a> {
    fn malformed(&self, path: &str, reason: impl Into<String>) -> Error {
        Error::mkv_malformed(path, self.offset, reason)
    }

    /// The body as an unsigned integer. An empty body is 0.
    pub fn uint(&self, path: &str) -> Result<u64> {
        if self.body.len() > 8 {
            return Err(self.malformed(
                path,
                format!(
                    "an unsigned integer holds at most 8 bytes, this one {}",
                    self.body.len()
                ),
            ));
        }
        Ok(self
            .body
            .iter()
            .fold(0u64, |acc, &byte| (acc << 8) | u64::from(byte)))
    }

    /// The body as a signed integer. An empty body is 0.
    pub fn int(&self, path: &str) -> Result<i64> {
        let raw = self.uint(path)?;
        let bits = 8 * self.body.len() as u32;
        if bits == 0 || bits == 64 {
            return Ok(raw as i64);
        }
        let shift = 64 - bits;
        Ok(((raw << shift) as i64) >> shift)
    }

    /// The body as a float: 0, 4 or 8 bytes.
    pub fn float(&self, path: &str) -> Result<f64> {
        match self.body.len() {
            0 => Ok(0.0),
            4 => Ok(f64::from(f32::from_bits(self.uint(path)? as u32))),
            8 => Ok(f64::from_bits(self.uint(path)?)),
            len => Err(self.malformed(
                path,
                format!("a float holds 0, 4 or 8 bytes, this one {len}"),
            )),
        }
    }

    /// The body as UTF-8 text, trailing NUL padding removed.
    pub fn string(&self, path: &str) -> Result<String> {
        let end = self
            .body
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |last| last + 1);
        let text = self.body.get(..end).unwrap_or_default();
        std::str::from_utf8(text)
            .map(str::to_owned)
            .map_err(|err| self.malformed(path, format!("the text is not UTF-8: {err}")))
    }

    /// The body bytes.
    pub fn binary(&self) -> Vec<u8> {
        self.body.to_vec()
    }

    /// The children of this element.
    pub fn children(&self, path: &'a str) -> Children<'a> {
        Children::new(self.body, self.body_offset, path)
    }
}

/// The children of an element held in memory, in file order. `Void` and
/// `CRC-32` children are skipped. After an error the iterator yields
/// nothing more.
#[derive(Debug)]
pub(crate) struct Children<'a> {
    bytes: &'a [u8],
    base: u64,
    pos: usize,
    path: &'a str,
    done: bool,
}

impl<'a> Children<'a> {
    /// The children of the body `bytes`, which starts at byte offset `base`.
    /// `path` names the parent in errors.
    pub fn new(bytes: &'a [u8], base: u64, path: &'a str) -> Self {
        Self {
            bytes,
            base,
            pos: 0,
            path,
            done: false,
        }
    }

    fn step(&mut self) -> Result<Option<Element<'a>>> {
        let rest = self.bytes.get(self.pos..).unwrap_or_default();
        if rest.is_empty() {
            return Ok(None);
        }
        let offset = self.base + self.pos as u64;
        let header = header(rest, offset, self.path)?.ok_or_else(|| {
            Error::mkv_malformed(
                self.path,
                offset,
                "an element header runs past the end of its parent",
            )
        })?;
        let child = format!("{}/{}", self.path, ids::name(header.id));
        let size = header.known_size(&child, offset)?;
        let remaining = rest.len() as u64 - header.len;
        if size > remaining {
            return Err(Error::mkv_malformed(
                &child,
                offset,
                format!("the element declares {size} bytes, but its parent holds {remaining} more"),
            ));
        }
        let start = header.len as usize;
        let end = start + size as usize;
        self.pos += end;
        Ok(Some(Element {
            id: header.id,
            offset,
            body_offset: offset + header.len,
            body: rest.get(start..end).unwrap_or_default(),
        }))
    }
}

impl<'a> Iterator for Children<'a> {
    type Item = Result<Element<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            match self.step() {
                Ok(Some(element)) if matches!(element.id, ids::VOID | ids::CRC32) => continue,
                Ok(Some(element)) => return Some(Ok(element)),
                Ok(None) => self.done = true,
                Err(err) => {
                    self.done = true;
                    return Some(Err(err));
                }
            }
        }
        None
    }
}

/// The path of child `id` of `parent`.
pub(crate) fn child_path(parent: &str, id: u32) -> String {
    format!("{parent}/{}", ids::name(id))
}

/// The path of the `n`th (from 1) repeated child `id` of `parent`.
pub(crate) fn indexed_path(parent: &str, id: u32, n: usize) -> String {
    format!("{parent}/{}[{n}]", ids::name(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_decodes_each_length() {
        assert_eq!(vint(&[0x81]), Vint::Value(1, 1));
        assert_eq!(vint(&[0x40, 0x02]), Vint::Value(2, 2));
        assert_eq!(vint(&[0x01, 0, 0, 0, 0, 0, 0, 3]), Vint::Value(3, 8));
        assert_eq!(vint(&[0x40]), Vint::Short);
        assert_eq!(vint(&[0x00]), Vint::Invalid);
    }

    #[test]
    fn header_reads_unknown_sizes_of_every_length() {
        let one = header(&[0x1F, 0x43, 0xB6, 0x75, 0xFF], 0, "t")
            .ok()
            .flatten();
        assert_eq!(one.map(|h| h.size), Some(None));
        let eight = [
            0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let eight = header(&eight, 0, "t").ok().flatten();
        assert_eq!(
            eight.map(|h| (h.id, h.size, h.len)),
            Some((ids::CLUSTER, None, 12))
        );
    }

    #[test]
    fn header_refuses_long_ids() {
        assert!(header(&[0x08, 0, 0, 0, 0, 0x80], 0, "t").is_err());
    }

    #[test]
    fn leaves_decode_signed_and_float_values() {
        fn element(body: &[u8]) -> Element<'_> {
            Element {
                id: 0,
                offset: 0,
                body_offset: 0,
                body,
            }
        }
        assert_eq!(element(&[0xFF, 0xFE]).int("t").ok(), Some(-2));
        assert_eq!(
            element(&[0x40, 0xE7, 0x70, 0, 0, 0, 0, 0]).float("t").ok(),
            Some(48_000.0)
        );
        assert_eq!(element(b"und\0\0").string("t").ok().as_deref(), Some("und"));
        assert!(element(&[0; 9]).uint("t").is_err());
    }

    #[test]
    fn children_skip_void_and_refuse_overruns() {
        let body = [0xEC, 0x81, 0x00, 0xD7, 0x81, 0x05];
        let ids: Vec<u32> = Children::new(&body, 0, "t")
            .filter_map(|child| child.ok().map(|c| c.id))
            .collect();
        assert_eq!(ids, vec![ids::TRACK_NUMBER]);
        let overrun = [0xD7, 0x85, 0x05];
        assert!(Children::new(&overrun, 0, "t").any(|child| child.is_err()));
    }
}
