//! The start of the file, and the patches a seekable finish applies: the EBML
//! header, the Segment header, the `SeekHead` and `Info`.

use crate::error::Result;
use crate::mkv::ebml_write::{ElementBuf, UNKNOWN_SIZE, fixed_size, push_id, void};
use crate::mkv::ids;
use crate::mkv::mux::bytes::invalid;
use crate::mkv::mux::options::{APP_NAME, MkvOptions};

/// `DocTypeVersion`: the features the file uses come from version 4.
const DOC_TYPE_VERSION: u64 = 4;
/// `DocTypeReadVersion`: a reader of version 2 can play the file.
const DOC_TYPE_READ_VERSION: u64 = 2;
/// Bytes reserved for the `SeekHead`: its header and the six entries it can
/// hold, of 21 bytes each.
const SEEK_RESERVED: usize = 131;
/// The most `SeekHead` entries: `Info`, `Tracks`, `Cues`, `Chapters`, `Tags`
/// and `Attachments`.
const MAX_SEEK_ENTRIES: usize = 6;

/// Where the start of the file holds what the finish patches.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Head {
    /// The offset of the 8-byte Segment size.
    pub segment_size_at: u64,
    /// The offset of the Segment body.
    pub segment_data: u64,
    /// The offset of the reserved `SeekHead`, on a seekable output.
    pub seek_head_at: Option<u64>,
    /// The offset of the `Info` element.
    pub info_at: u64,
    /// The offset of the 8-byte `Duration` body, on a seekable output.
    pub duration_at: Option<u64>,
}

/// The bytes from the file start through `Info`, and where they hold what the
/// finish patches.
pub(crate) fn build_head(options: &MkvOptions) -> Result<(Vec<u8>, Head)> {
    let mut ebml = ElementBuf::new();
    ebml.uint(ids::EBML_VERSION, 1)?;
    ebml.uint(ids::EBML_READ_VERSION, 1)?;
    ebml.uint(ids::EBML_MAX_ID_LENGTH, 4)?;
    ebml.uint(ids::EBML_MAX_SIZE_LENGTH, 8)?;
    ebml.text(ids::DOC_TYPE, options.doc_type.value())?;
    ebml.uint(ids::DOC_TYPE_VERSION, DOC_TYPE_VERSION)?;
    ebml.uint(ids::DOC_TYPE_READ_VERSION, DOC_TYPE_READ_VERSION)?;
    let mut top = ElementBuf::new();
    top.master(ids::EBML, &ebml)?;
    let mut bytes = top.into_bytes();

    push_id(&mut bytes, ids::SEGMENT);
    let segment_size_at = bytes.len() as u64;
    bytes.extend_from_slice(&UNKNOWN_SIZE);
    let segment_data = bytes.len() as u64;
    let seek_head_at = options.seekable.then_some(segment_data);
    if options.seekable {
        bytes.extend_from_slice(&void(SEEK_RESERVED));
    }

    let info_at = bytes.len() as u64;
    let mut info = ElementBuf::new();
    info.uint(ids::TIMESTAMP_SCALE, options.timestamp_scale)?;
    info.text(ids::MUXING_APP, APP_NAME)?;
    info.text(ids::WRITING_APP, &options.writing_app)?;
    if let Some(title) = &options.title {
        info.text(ids::TITLE, title)?;
    }
    if options.seekable {
        // `Duration` comes last, so its body ends the element.
        info.float(ids::DURATION, 0.0)?;
    }
    let mut outer = ElementBuf::new();
    outer.master(ids::INFO, &info)?;
    bytes.extend_from_slice(outer.as_bytes());
    let duration_at = options.seekable.then_some(bytes.len() as u64 - 8);
    Ok((
        bytes,
        Head {
            segment_size_at,
            segment_data,
            seek_head_at,
            info_at,
            duration_at,
        },
    ))
}

/// The bytes that replace the reserved `SeekHead`: the element for `entries`
/// of (ID, offset from the Segment body), then a `Void` for the rest.
pub(crate) fn seek_head(entries: &[(u32, u64)]) -> Result<Vec<u8>> {
    if entries.len() > MAX_SEEK_ENTRIES {
        return Err(invalid(format!(
            "the `SeekHead` needs {} entries, above the {MAX_SEEK_ENTRIES} reserved. Write one \
             element of each kind",
            entries.len()
        )));
    }
    let mut body = ElementBuf::new();
    for &(id, position) in entries {
        let mut seek = ElementBuf::new();
        seek.binary(ids::SEEK_ID, &id.to_be_bytes())?;
        seek.fixed_uint(ids::SEEK_POSITION, position)?;
        body.master(ids::SEEK, &seek)?;
    }
    let mut top = ElementBuf::new();
    top.master(ids::SEEK_HEAD, &body)?;
    let mut bytes = top.into_bytes();
    let gap = SEEK_RESERVED.saturating_sub(bytes.len());
    bytes.extend_from_slice(&void(gap));
    Ok(bytes)
}

/// The Segment size field for a body of `size` bytes.
pub(crate) fn segment_size(size: u64) -> Result<[u8; 8]> {
    fixed_size(ids::SEGMENT, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_head_fills_the_reservation_at_every_entry_count() {
        for count in 0..=MAX_SEEK_ENTRIES {
            let entries = vec![(ids::INFO, 1u64 << 40); count];
            let bytes = seek_head(&entries).expect("the entries fit");
            assert_eq!(bytes.len(), SEEK_RESERVED, "{count} entries");
        }
        assert!(seek_head(&[(ids::INFO, 0); 7]).is_err());
    }

    #[test]
    fn head_marks_where_the_finish_patches() {
        let (bytes, head) = build_head(&MkvOptions::default()).expect("the options fit");
        let at = head.segment_size_at as usize;
        assert_eq!(&bytes[at..at + 8], &UNKNOWN_SIZE);
        assert_eq!(head.seek_head_at, Some(head.segment_data));
        assert_eq!(head.duration_at, Some(bytes.len() as u64 - 8));
        let live = MkvOptions {
            seekable: false,
            ..MkvOptions::default()
        };
        let (bytes, head) = build_head(&live).expect("the options fit");
        assert_eq!(head.seek_head_at, None);
        assert_eq!(head.duration_at, None);
        assert_eq!(head.info_at, head.segment_data);
        assert!(bytes.len() as u64 > head.info_at);
    }
}
