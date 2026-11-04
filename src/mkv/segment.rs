//! The top-level scan: the EBML header, then every Segment child in file
//! order.
//!
//! The scan stops at the first element that runs past the input end, and
//! keeps what it read before it. An unknown-size Segment runs to the input
//! end. An unknown-size Cluster ends at the next Segment child.

use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mkv::attachments::{Attachment, scan_attachments};
use crate::mkv::chapters::parse_chapters;
use crate::mkv::cluster::{ClusterEnd, RawFrame, scan_cluster};
use crate::mkv::ebml::{Element, Header};
use crate::mkv::ids;
use crate::mkv::index::{CuePoint, SeekEntry, parse_cues, parse_seek_head};
use crate::mkv::info::{EbmlHeader, SegmentInfo, parse_ebml_header, parse_info};
use crate::mkv::source::Source;
use crate::mkv::tags::parse_tags;
use crate::mkv::tracks::parse_tracks;
use crate::track::{Timebase, Track};

/// The largest element read whole, in bytes: the EBML header, `SeekHead`,
/// `Info`, `Tracks`, `Cues`, `Chapters` and `Tags`.
pub const MAX_ELEMENT_SIZE: u64 = 256 << 20;

/// Everything the scan finds.
#[derive(Debug)]
pub(crate) struct Scan {
    pub ebml: EbmlHeader,
    pub segment_data: u64,
    pub info: SegmentInfo,
    /// The `Tracks` element, kept raw until `Info` is known.
    pub tracks: Option<(u64, Vec<u8>)>,
    pub frames: Vec<RawFrame>,
    pub seek_head: Vec<SeekEntry>,
    pub cues: Vec<CuePoint>,
    pub chapters: Vec<Chapter>,
    pub tags: Vec<MetadataItem>,
    pub attachments: Vec<Attachment>,
    pub truncated: bool,
}

/// Counts of repeated Segment children, for error paths.
#[derive(Debug, Default)]
struct Counts {
    clusters: u32,
    seek_heads: usize,
    tags: usize,
    attachments: usize,
}

/// Read a whole element whose header sits at `pos`, bounded by
/// [`MAX_ELEMENT_SIZE`]. Returns the body offset and bytes.
pub(crate) fn read_whole<R: Read + Seek>(
    src: &mut Source<'_, R>,
    pos: u64,
    header: Header,
    path: &str,
) -> Result<(u64, Vec<u8>)> {
    let body = pos + header.len;
    let size = header.known_size(path, pos)?;
    Ok((body, src.read_body(body, size, path, MAX_ELEMENT_SIZE)?))
}

pub(crate) fn element(id: u32, offset: u64, body_offset: u64, body: &[u8]) -> Element<'_> {
    Element {
        id,
        offset,
        body_offset,
        body,
    }
}

/// The EBML header of the input, and where its first Segment body lies.
#[derive(Debug)]
pub(crate) struct SegmentStart {
    pub ebml: EbmlHeader,
    /// The byte offset of the Segment body.
    pub data: u64,
    /// The offset the Segment size names. `u64::MAX` for an unknown size.
    pub declared_end: u64,
}

/// Read the EBML header, and find the first Segment after it.
pub(crate) fn open_segment<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<SegmentStart> {
    let first = src
        .header_at(0, "EBML")?
        .ok_or_else(|| Error::mkv_missing("EBML"))?;
    if first.id != ids::EBML {
        return Err(Error::mkv_malformed(
            "EBML",
            0,
            format!(
                "the input starts with element {:#X}, not the EBML header {:#X}",
                first.id,
                ids::EBML
            ),
        ));
    }
    let (body, bytes) = read_whole(src, 0, first, "EBML")?;
    let ebml = parse_ebml_header(&element(first.id, 0, body, &bytes))?;
    let mut pos = body + bytes.len() as u64;

    // Skip Void and CRC-32 up to the Segment.
    let segment = loop {
        let header = src
            .header_at(pos, "Segment")?
            .ok_or_else(|| Error::mkv_missing("Segment"))?;
        match header.id {
            ids::SEGMENT => break header,
            ids::VOID | ids::CRC32 => {
                let size = header.size.ok_or_else(|| Error::mkv_missing("Segment"))?;
                pos = (pos + header.len).saturating_add(size);
            }
            other => {
                return Err(Error::mkv_malformed(
                    "Segment",
                    pos,
                    format!("element {other:#X} follows the EBML header, not a Segment"),
                ));
            }
        }
    };
    let data = pos + segment.len;
    let declared_end = segment
        .size
        .map_or(u64::MAX, |size| data.saturating_add(size));
    Ok(SegmentStart {
        ebml,
        data,
        declared_end,
    })
}

/// Scan the input.
pub(crate) fn scan<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<Scan> {
    let SegmentStart {
        ebml,
        data: segment_data,
        declared_end,
    } = open_segment(src)?;
    let end = declared_end.min(src.input_len);

    let mut scan = Scan {
        ebml,
        segment_data,
        info: SegmentInfo::default(),
        tracks: None,
        frames: Vec::new(),
        seek_head: Vec::new(),
        cues: Vec::new(),
        chapters: Vec::new(),
        tags: Vec::new(),
        attachments: Vec::new(),
        truncated: declared_end != u64::MAX && declared_end > src.input_len,
    };
    let mut seen_info = false;
    let mut counts = Counts::default();
    let mut at = segment_data;
    while at < end {
        let Some(header) = src.header_at(at, "Segment")? else {
            scan.truncated = true;
            break;
        };
        if matches!(header.id, ids::EBML | ids::SEGMENT) {
            // A chained Segment follows. Only the first is read.
            break;
        }
        if header.id == ids::CLUSTER {
            counts.clusters = counts.clusters.saturating_add(1);
            match scan_cluster(src, at, header, counts.clusters, &mut scan.frames)? {
                ClusterEnd::At(next) => {
                    at = next;
                    continue;
                }
                ClusterEnd::Truncated => {
                    scan.truncated = true;
                    break;
                }
            }
        }
        let path = format!("Segment/{}", ids::name(header.id));
        let size = header.known_size(&path, at)?;
        let next = (at + header.len).saturating_add(size);
        if next > src.input_len {
            scan.truncated = true;
            break;
        }
        visit(src, at, header, &mut scan, &mut seen_info, &mut counts)?;
        at = next;
    }
    if !seen_info {
        return Err(Error::mkv_missing("Segment/Info"));
    }
    Ok(scan)
}

/// Read one Segment child that is not a Cluster.
fn visit<R: Read + Seek>(
    src: &mut Source<'_, R>,
    at: u64,
    header: Header,
    scan: &mut Scan,
    seen_info: &mut bool,
    counts: &mut Counts,
) -> Result<()> {
    let whole = |src: &mut Source<'_, R>, path: &str| read_whole(src, at, header, path);
    match header.id {
        ids::INFO => {
            if *seen_info {
                return Err(Error::mkv_malformed(
                    "Segment/Info",
                    at,
                    "a second Info follows the first",
                ));
            }
            let (body, bytes) = whole(src, "Segment/Info")?;
            scan.info = parse_info(&element(header.id, at, body, &bytes))?;
            *seen_info = true;
        }
        ids::TRACKS => {
            if scan.tracks.is_some() {
                return Err(Error::mkv_malformed(
                    "Segment/Tracks",
                    at,
                    "a second Tracks follows the first",
                ));
            }
            let (body, bytes) = whole(src, "Segment/Tracks")?;
            scan.tracks = Some((body, bytes));
        }
        ids::SEEK_HEAD => {
            counts.seek_heads += 1;
            let (body, bytes) = whole(src, "Segment/SeekHead")?;
            let entries = parse_seek_head(
                &element(header.id, at, body, &bytes),
                counts.seek_heads,
                scan.segment_data,
            )?;
            scan.seek_head.extend(entries);
        }
        ids::CUES => {
            let (body, bytes) = whole(src, "Segment/Cues")?;
            let cues = parse_cues(&element(header.id, at, body, &bytes), scan.segment_data)?;
            scan.cues.extend(cues);
        }
        ids::CHAPTERS => {
            let (body, bytes) = whole(src, "Segment/Chapters")?;
            let chapters = parse_chapters(&element(header.id, at, body, &bytes))?;
            scan.chapters.extend(chapters);
        }
        ids::TAGS => {
            counts.tags += 1;
            let (body, bytes) = whole(src, "Segment/Tags")?;
            let tags = parse_tags(&element(header.id, at, body, &bytes), counts.tags)?;
            scan.tags.extend(tags);
        }
        ids::ATTACHMENTS => {
            counts.attachments += 1;
            scan_attachments(src, at, header, counts.attachments, &mut scan.attachments)?;
        }
        _ => {}
    }
    Ok(())
}

/// Parse the `Tracks` element the scan kept, with the `Info` timebase.
pub(crate) fn tracks_of(scan: &Scan) -> Result<Vec<Track>> {
    let (body, bytes) = scan
        .tracks
        .as_ref()
        .ok_or_else(|| Error::mkv_missing("Segment/Tracks"))?;
    // `parse_info` bounds the scale to `u32`.
    let nanoseconds = u32::try_from(scan.info.timestamp_scale).unwrap_or(u32::MAX);
    let timebase = Timebase::from_nanoseconds(nanoseconds);
    parse_tracks(&element(ids::TRACKS, *body, *body, bytes), timebase)
}
