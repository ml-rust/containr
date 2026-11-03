//! Box writing into memory: headers with patched sizes, big-endian fields,
//! and mp4-atom encoders.

use mp4_atom::Encode;

use crate::error::{Container, Error, Result};

/// An error for a box that does not encode.
pub(crate) fn encode_error(path: &str, reason: impl std::fmt::Display) -> Error {
    Error::InvalidMux {
        container: Container::Mp4,
        reason: format!("box `{path}` does not encode: {reason}"),
    }
}

/// An error for an invalid muxer call, option, spec or item.
pub(crate) fn invalid(reason: String) -> Error {
    Error::InvalidMux {
        container: Container::Mp4,
        reason,
    }
}

/// A buffer of whole boxes.
#[derive(Debug, Default)]
pub(crate) struct BoxBuf {
    bytes: Vec<u8>,
}

impl BoxBuf {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes, given back.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Open a box of `kind`. Pass the returned start to [`BoxBuf::close`].
    pub fn open(&mut self, kind: &[u8; 4]) -> usize {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
        self.bytes.extend_from_slice(kind);
        start
    }

    /// Open a full box: a box with a version and 24 bits of flags.
    pub fn open_full(&mut self, kind: &[u8; 4], version: u8, flags: u32) -> usize {
        let start = self.open(kind);
        let mut head = flags.to_be_bytes();
        head[0] = version;
        self.bytes.extend_from_slice(&head);
        start
    }

    /// Close the box opened at `start`: write its size.
    pub fn close(&mut self, start: usize, path: &str) -> Result<()> {
        let size = u32::try_from(self.bytes.len() - start)
            .map_err(|_| encode_error(path, "the box spans 4 GiB or more"))?;
        if let Some(field) = self.bytes.get_mut(start..start + 4) {
            field.copy_from_slice(&size.to_be_bytes());
        }
        Ok(())
    }

    /// Append a whole box of `kind` with `body`.
    pub fn boxed(&mut self, kind: &[u8; 4], body: &[u8], path: &str) -> Result<()> {
        let start = self.open(kind);
        self.bytes.extend_from_slice(body);
        self.close(start, path)
    }

    /// Append a box with an mp4-atom encoder.
    pub fn encode<T: Encode>(&mut self, value: &T, path: &str) -> Result<()> {
        value
            .encode(&mut self.bytes)
            .map_err(|err| encode_error(path, err))
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn zeros(&mut self, count: usize) {
        self.bytes.resize(self.bytes.len() + count, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_writes_the_box_size() {
        let mut buf = BoxBuf::new();
        let outer = buf.open(b"moov");
        let inner = buf.open_full(b"mvhd", 1, 3);
        buf.u32(7);
        buf.close(inner, "moov/mvhd").expect("the box is small");
        buf.close(outer, "moov").expect("the box is small");
        let bytes = buf.into_bytes();
        assert_eq!(
            bytes,
            [
                0, 0, 0, 24, b'm', b'o', b'o', b'v', 0, 0, 0, 16, b'm', b'v', b'h', b'd', 1, 0, 0,
                3, 0, 0, 0, 7
            ]
        );
    }
}
