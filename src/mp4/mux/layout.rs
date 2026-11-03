//! Top-level layout: `ftyp`, the `mdat` header, `moov` placement and the
//! faststart rewrite.
//!
//! The muxer writes `ftyp`, 16 reserved bytes and the sample data. The
//! reserved bytes become a `free` box and an 8-byte `mdat` header, or one
//! 16-byte `mdat` header when the data reaches 4 GiB.
//!
//! Without faststart, `moov` follows `mdat`. With faststart, the muxer
//! computes `moov` with every chunk offset moved up by the `moov` size,
//! shifts `free` and `mdat` up by that size in place, and writes `moov` in
//! the gap. The copy runs from the end in blocks of [`MOVE_BLOCK`] bytes,
//! so no byte is overwritten before it is read, and memory stays bounded.
//! A `moov` that grows when an offset crosses 4 GiB (`stco` to `co64`)
//! changes the shift, so the size is recomputed until it holds.

use std::io::{Read, Seek, SeekFrom, Write};

use mp4_atom::{FourCC, Ftyp};

use crate::error::{Container, Error, Result};
use crate::mp4::mux::bytes::BoxBuf;
use crate::mp4::mux::options::Brand;

/// The block size of the faststart copy, in bytes.
pub const MOVE_BLOCK: usize = 1 << 20;
/// The reserved bytes before the sample data: `free` and `mdat` headers.
const RESERVED: u64 = 16;
/// The most `moov` size computations faststart runs before it gives up.
const MAX_PASSES: usize = 4;

fn io(what: &str, offset: u64, err: std::io::Error) -> Error {
    Error::io(format!("the output: {what}"), offset, err)
}

/// Write `ftyp` and the reserved header bytes. Returns the offset of the
/// reserved bytes and the offset of the sample data.
pub(crate) fn write_head<W: Write + Seek>(writer: &mut W, brand: Brand) -> Result<(u64, u64)> {
    let (major, minor, compatible) = brand.ftyp();
    let ftyp = Ftyp {
        major_brand: FourCC::from(major.0),
        minor_version: minor,
        compatible_brands: compatible.iter().map(|b| FourCC::from(b.0)).collect(),
    };
    let mut buf = BoxBuf::new();
    buf.encode(&ftyp, "ftyp")?;
    buf.boxed(b"free", &[], "free")?;
    buf.boxed(b"mdat", &[], "mdat")?;
    let head = buf.into_bytes();
    writer
        .seek(SeekFrom::Start(0))
        .and_then(|_| writer.write_all(&head))
        .map_err(|err| io("writing ftyp", 0, err))?;
    let data = head.len() as u64;
    Ok((data - RESERVED, data))
}

/// Write the `mdat` header into the reserved bytes at `reserved`, for data
/// that ends at `end`.
pub(crate) fn patch_mdat<W: Write + Seek>(writer: &mut W, reserved: u64, end: u64) -> Result<()> {
    let data = end - (reserved + RESERVED);
    let mut header = Vec::with_capacity(RESERVED as usize);
    match u32::try_from(data + 8) {
        Ok(size) => {
            header.extend_from_slice(&8u32.to_be_bytes());
            header.extend_from_slice(b"free");
            header.extend_from_slice(&size.to_be_bytes());
            header.extend_from_slice(b"mdat");
        }
        Err(_) => {
            header.extend_from_slice(&1u32.to_be_bytes());
            header.extend_from_slice(b"mdat");
            header.extend_from_slice(&(data + RESERVED).to_be_bytes());
        }
    }
    writer
        .seek(SeekFrom::Start(reserved))
        .and_then(|_| writer.write_all(&header))
        .map_err(|err| io("writing the mdat header", reserved, err))
}

/// Write `bytes` at `offset`.
pub(crate) fn write_at<W: Write + Seek>(writer: &mut W, offset: u64, bytes: &[u8]) -> Result<()> {
    writer
        .seek(SeekFrom::Start(offset))
        .and_then(|_| writer.write_all(bytes))
        .map_err(|err| io("writing moov", offset, err))
}

/// Move the bytes in `start..end` up by `by`, from the end down.
pub(crate) fn shift_up<W: Read + Write + Seek>(
    writer: &mut W,
    start: u64,
    end: u64,
    by: u64,
) -> Result<()> {
    let mut block = vec![0u8; MOVE_BLOCK];
    let mut top = end;
    while top > start {
        let len = (top - start).min(MOVE_BLOCK as u64);
        let from = top - len;
        let Some(chunk) = block.get_mut(..len as usize) else {
            break;
        };
        writer
            .seek(SeekFrom::Start(from))
            .and_then(|_| writer.read_exact(chunk))
            .map_err(|err| io("reading mdat back for faststart", from, err))?;
        writer
            .seek(SeekFrom::Start(from + by))
            .and_then(|_| writer.write_all(chunk))
            .map_err(|err| io("moving mdat for faststart", from + by, err))?;
        top = from;
    }
    Ok(())
}

/// Find the `moov` size that holds when every chunk offset moves up by it.
/// `build` makes `moov` for a shift.
pub(crate) fn settle_moov(mut build: impl FnMut(u64) -> Result<Vec<u8>>) -> Result<Vec<u8>> {
    let mut moov = build(0)?;
    for _ in 0..MAX_PASSES {
        let shift = moov.len() as u64;
        let next = build(shift)?;
        if next.len() as u64 == shift {
            return Ok(next);
        }
        moov = next;
    }
    Err(Error::InvalidMux {
        container: Container::Mp4,
        reason: format!(
            "the moov size does not settle after {MAX_PASSES} passes. Write the file \
             without faststart"
        ),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn shift_up_moves_bytes_across_blocks() {
        let data: Vec<u8> = (0..(MOVE_BLOCK as u32 * 2 + 77)).map(|n| n as u8).collect();
        let mut cursor = Cursor::new(data.clone());
        shift_up(&mut cursor, 5, data.len() as u64, 1000).expect("the move succeeds");
        let out = cursor.into_inner();
        assert_eq!(&out[1005..], &data[5..]);
        assert_eq!(&out[..5], &data[..5]);
    }

    #[test]
    fn mdat_header_turns_large_past_4_gib() {
        let mut cursor = Cursor::new(Vec::new());
        patch_mdat(&mut cursor, 0, 16 + 100).expect("the header writes");
        let small = cursor.into_inner();
        assert_eq!(&small[..8], &[0, 0, 0, 8, b'f', b'r', b'e', b'e']);
        assert_eq!(&small[8..16], &[0, 0, 0, 108, b'm', b'd', b'a', b't']);
        let mut cursor = Cursor::new(Vec::new());
        patch_mdat(&mut cursor, 0, 16 + (5 << 30)).expect("the header writes");
        let large = cursor.into_inner();
        assert_eq!(&large[..8], &[0, 0, 0, 1, b'm', b'd', b'a', b't']);
        let size = u64::from_be_bytes(large[8..16].try_into().expect("8 bytes"));
        assert_eq!(size, 16 + (5 << 30));
    }

    #[test]
    fn settle_moov_follows_a_growing_size() {
        let moov = settle_moov(|shift| Ok(vec![0; if shift > 10 { 30 } else { 12 }]))
            .expect("the size settles");
        assert_eq!(moov.len(), 30);
    }
}
