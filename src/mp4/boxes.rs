//! Box walking over an in-memory buffer, and big-endian field reads.
//!
//! Every size comes from the input, so every slice is checked against the
//! bytes the parent holds before it is taken.

use mp4_atom::{Decode, Header};

use crate::error::{Error, Result};
use crate::track::FourCc;

/// One box inside a parent buffer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawBox<'a> {
    /// The box type.
    pub kind: FourCc,
    /// The byte offset of the box header in the input.
    pub offset: u64,
    /// The whole box, header included.
    pub full: &'a [u8],
    /// The box body, after the header.
    pub body: &'a [u8],
    /// The byte offset of the body in the input.
    pub body_offset: u64,
}

impl<'a> RawBox<'a> {
    /// Parse the one box that fills `data` exactly.
    pub fn whole(data: &'a [u8], offset: u64, path: &str) -> Result<Self> {
        let mut children = Children::new(data, offset, path);
        match children.next() {
            Some(Ok(raw)) if raw.full.len() == data.len() => Ok(raw),
            Some(Ok(raw)) => Err(Error::malformed(
                path,
                offset,
                format!(
                    "the box spans {} bytes but {} bytes were read for it",
                    raw.full.len(),
                    data.len()
                ),
            )),
            Some(Err(err)) => Err(err),
            None => Err(Error::malformed(
                path,
                offset,
                format!("{} bytes are too few for a box header", data.len()),
            )),
        }
    }

    /// Iterate the child boxes that start `skip` bytes into the body.
    pub fn children<'p>(&self, skip: usize, path: &'p str) -> Result<Children<'a, 'p>> {
        let body = self.body.get(skip..).ok_or_else(|| {
            Error::malformed(
                path,
                self.offset,
                format!(
                    "the body holds {} bytes, fewer than the {skip} fixed bytes before its children",
                    self.body.len()
                ),
            )
        })?;
        Ok(Children::new(body, self.body_offset + skip as u64, path))
    }

    /// Decode the box with an mp4-atom type.
    pub fn decode<T: Decode>(&self, path: &str) -> Result<T> {
        let mut bytes = self.full;
        T::decode(&mut bytes).map_err(|err| Error::malformed(path, self.offset, err.to_string()))
    }

    /// A field reader over the body.
    pub fn reader<'p>(&self, path: &'p str) -> FieldReader<'a, 'p> {
        FieldReader {
            data: self.body,
            pos: 0,
            offset: self.body_offset,
            path,
        }
    }
}

/// An iterator over the boxes in a buffer.
///
/// A remainder shorter than a box header ends the walk without an error:
/// QuickTime writers pad containers with a 4-byte zero terminator. After an
/// error the iterator yields nothing more.
pub(crate) struct Children<'a, 'p> {
    data: &'a [u8],
    pos: usize,
    base: u64,
    path: &'p str,
}

impl<'a, 'p> Children<'a, 'p> {
    pub fn new(data: &'a [u8], base: u64, path: &'p str) -> Self {
        Self {
            data,
            pos: 0,
            base,
            path,
        }
    }

    fn fail(&mut self, offset: u64, reason: String) -> Option<Result<RawBox<'a>>> {
        self.pos = self.data.len();
        Some(Err(Error::malformed(self.path, offset, reason)))
    }
}

impl<'a> Iterator for Children<'a, '_> {
    type Item = Result<RawBox<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.data.get(self.pos..)?;
        if rest.len() < 8 {
            self.pos = self.data.len();
            return None;
        }
        let offset = self.base + self.pos as u64;
        let mut cursor = rest;
        let header = match Header::decode(&mut cursor) {
            Ok(header) => header,
            Err(err) => {
                return self.fail(offset, format!("a child box header does not decode: {err}"));
            }
        };
        let header_len = rest.len() - cursor.len();
        let body_len = header.size.unwrap_or(cursor.len());
        let kind = FourCc(header.kind.into());
        if body_len > cursor.len() {
            return self.fail(
                offset,
                format!(
                    "child box `{kind}` declares {body_len} body bytes, but only {} remain in its parent",
                    cursor.len()
                ),
            );
        }
        let full_len = header_len + body_len;
        let full = &rest[..full_len];
        self.pos += full_len;
        Some(Ok(RawBox {
            kind,
            offset,
            full,
            body: &full[header_len..],
            body_offset: offset + header_len as u64,
        }))
    }
}

/// Collect the children of a box, failing on the first malformed one.
pub(crate) fn collect_children<'a>(
    parent: &RawBox<'a>,
    skip: usize,
    path: &str,
) -> Result<Vec<RawBox<'a>>> {
    parent.children(skip, path)?.collect()
}

/// Big-endian field reads that fail with an error naming the box.
pub(crate) struct FieldReader<'a, 'p> {
    data: &'a [u8],
    pos: usize,
    offset: u64,
    path: &'p str,
}

impl<'a> FieldReader<'a, '_> {
    /// The bytes not read yet.
    pub fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or_default()
    }

    /// The read position within the body.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Read `len` bytes.
    pub fn bytes(&mut self, len: usize, field: &str) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len());
        match end {
            Some(end) => {
                let out = &self.data[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => Err(Error::malformed(
                self.path,
                self.offset,
                format!(
                    "field `{field}` needs {len} bytes at body offset {}, but the body holds {}",
                    self.pos,
                    self.data.len()
                ),
            )),
        }
    }

    /// Skip `len` bytes.
    pub fn skip(&mut self, len: usize, field: &str) -> Result<()> {
        self.bytes(len, field).map(|_| ())
    }

    fn array<const N: usize>(&mut self, field: &str) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(N, field)?);
        Ok(out)
    }

    pub fn u8(&mut self, field: &str) -> Result<u8> {
        Ok(self.array::<1>(field)?[0])
    }

    pub fn u16(&mut self, field: &str) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array(field)?))
    }

    pub fn u32(&mut self, field: &str) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array(field)?))
    }

    pub fn u64(&mut self, field: &str) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array(field)?))
    }

    pub fn fourcc(&mut self, field: &str) -> Result<FourCc> {
        Ok(FourCc(self.array(field)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn children_walks_boxes_and_ignores_short_padding() {
        let mut data = boxed(b"free", &[1, 2]);
        data.extend(boxed(b"skip", &[]));
        data.extend([0, 0, 0, 0]);
        let kinds: Vec<FourCc> = Children::new(&data, 100, "test")
            .map(|raw| raw.map(|raw| raw.kind))
            .collect::<Result<_>>()
            .expect("the boxes are well formed");
        assert_eq!(kinds, vec![FourCc(*b"free"), FourCc(*b"skip")]);
    }

    #[test]
    fn children_rejects_a_box_past_its_parent() {
        let mut data = boxed(b"free", &[1, 2]);
        data[3] = 40;
        let first = Children::new(&data, 0, "test").next();
        assert!(matches!(first, Some(Err(Error::Malformed { .. }))));
    }

    #[test]
    fn children_rejects_a_size_below_the_header() {
        let data = [0, 0, 0, 3, b'f', b'r', b'e', b'e'];
        let first = Children::new(&data, 0, "test").next();
        assert!(matches!(first, Some(Err(Error::Malformed { .. }))));
    }

    #[test]
    fn field_reader_reports_short_bodies() {
        let data = boxed(b"test", &[0, 1]);
        let raw = RawBox::whole(&data, 0, "test").expect("the box is well formed");
        let mut reader = raw.reader("test");
        assert_eq!(reader.u16("a").expect("two bytes remain"), 1);
        assert!(reader.u32("b").is_err());
    }
}
