//! Packet byte reads, shared by the demuxers.

use std::io::{Read, Seek, SeekFrom};

use crate::error::{Error, Result};
use crate::packet::Packet;

/// Read the bytes of `packet` into `buf`, after checking them against the
/// input length.
pub(crate) fn read_range<R: Read + Seek>(
    reader: &mut R,
    input_len: u64,
    packet: &Packet,
    buf: &mut Vec<u8>,
) -> Result<()> {
    let out_of_bounds = Error::PacketOutOfBounds {
        track_id: packet.track_id,
        index: packet.index,
        offset: packet.offset,
        size: packet.size,
        input_len,
    };
    match packet.offset.checked_add(u64::from(packet.size)) {
        Some(end) if end <= input_len => {}
        _ => return Err(out_of_bounds),
    }
    buf.clear();
    buf.resize(packet.size as usize, 0);
    reader
        .seek(SeekFrom::Start(packet.offset))
        .and_then(|_| reader.read_exact(buf))
        .map_err(|err| {
            Error::io(
                format!("packet {} of track {}", packet.index, packet.track_id),
                packet.offset,
                err,
            )
        })
}
