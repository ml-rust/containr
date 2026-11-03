//! Positioned reads from the input: element headers, short prefixes, and
//! bounded whole bodies.

use std::io::{Read, Seek, SeekFrom};

use crate::error::{Error, Result};
use crate::mkv::ebml::{self, Header};

/// The longest element header: a 4-byte ID and an 8-byte size.
const MAX_HEADER_LEN: usize = ebml::MAX_ID_LEN + ebml::MAX_SIZE_LEN;

/// A reader with its length, for reads at absolute byte offsets.
#[derive(Debug)]
pub(crate) struct Source<'r, R> {
    reader: &'r mut R,
    /// The input length in bytes.
    pub input_len: u64,
}

impl<'r, R: Read + Seek> Source<'r, R> {
    pub fn new(reader: &'r mut R, input_len: u64) -> Self {
        Self { reader, input_len }
    }

    /// Read up to `buf.len()` bytes at `pos`, fewer when the input ends
    /// first. Returns the bytes read.
    pub fn peek<'b>(&mut self, pos: u64, buf: &'b mut [u8], what: &str) -> Result<&'b [u8]> {
        let available = self.input_len.saturating_sub(pos);
        let take = (buf.len() as u64).min(available) as usize;
        let out = buf.get_mut(..take).unwrap_or_default();
        self.reader
            .seek(SeekFrom::Start(pos))
            .map_err(|err| Error::io(what, pos, err))?;
        self.reader
            .read_exact(out)
            .map_err(|err| Error::io(what, pos, err))?;
        Ok(out)
    }

    /// Read the element header at `pos`. Returns `None` when the input ends
    /// before the header does.
    pub fn header_at(&mut self, pos: u64, path: &str) -> Result<Option<Header>> {
        let mut buf = [0u8; MAX_HEADER_LEN];
        let bytes = self.peek(pos, &mut buf, "an element header")?;
        ebml::header(bytes, pos, path)
    }

    /// Read `len` bytes at `pos`. `len` must not exceed `limit`, and the
    /// bytes must lie inside the input.
    pub fn read_body(&mut self, pos: u64, len: u64, path: &str, limit: u64) -> Result<Vec<u8>> {
        if len > limit {
            return Err(Error::mkv_too_large(path, len, limit));
        }
        let fits = pos
            .checked_add(len)
            .is_some_and(|end| end <= self.input_len);
        if !fits {
            return Err(Error::mkv_malformed(
                path,
                pos,
                format!(
                    "the body declares {len} bytes, past the input end at {}",
                    self.input_len
                ),
            ));
        }
        let mut bytes = vec![0u8; len as usize];
        self.reader
            .seek(SeekFrom::Start(pos))
            .and_then(|_| self.reader.read_exact(&mut bytes))
            .map_err(|err| Error::io(format!("element `{path}`"), pos, err))?;
        Ok(bytes)
    }
}
