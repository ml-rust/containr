//! EBML element encoders.

/// The ID bytes of `id`, marker bits included.
pub(super) fn id_bytes(id: u32) -> Vec<u8> {
    let bytes = id.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count();
    bytes[skip..].to_vec()
}

/// The shortest size encoding of `size` that is not the unknown marker.
pub(super) fn size_bytes(size: usize) -> Vec<u8> {
    let size = size as u64;
    let len = (1..=8)
        .find(|&n| size < (1u64 << (7 * n)) - 1)
        .expect("size fits");
    let marked = size | (1u64 << (7 * len));
    marked.to_be_bytes()[8 - len..].to_vec()
}

pub(super) fn el(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = id_bytes(id);
    out.extend(size_bytes(body.len()));
    out.extend_from_slice(body);
    out
}

pub(super) fn uint(id: u32, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count().min(7);
    el(id, &bytes[skip..])
}

/// An unsigned integer in 8 bytes, so its size never depends on the value.
pub(super) fn uint8(id: u32, value: u64) -> Vec<u8> {
    el(id, &value.to_be_bytes())
}

pub(super) fn text(id: u32, value: &str) -> Vec<u8> {
    el(id, value.as_bytes())
}

pub(super) fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

/// The frame byte pattern: track id, frame index, then filler.
pub fn frame_bytes(track: u32, index: u64, size: u32) -> Vec<u8> {
    (0..size)
        .map(|i| (u64::from(track) * 64 + index * 8 + u64::from(i)) as u8)
        .collect()
}
