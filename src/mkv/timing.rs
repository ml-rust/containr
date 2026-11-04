//! One track's trim facts, read with no packet index.
//!
//! [`first_track_timing`] reads the EBML header, `Tracks`, and the blocks of
//! each Cluster. It keeps one Cluster's frames at a time.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

use crate::error::{Error, Result};
use crate::mkv::cluster::{ClusterEnd, RawFrame, scan_cluster};
use crate::mkv::ebml::{Element, child_path, indexed_path};
use crate::mkv::ids;
use crate::mkv::segment::{element, open_segment, read_whole};
use crate::mkv::source::Source;
use crate::track::TrackKind;

/// The trim facts of one track, in nanoseconds. The caller applies them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackTiming {
    /// The `TrackNumber`.
    pub track_id: u32,
    /// The `CodecDelay`: decoder delay to drop from the start.
    pub codec_delay: u64,
    /// The sum of every positive `DiscardPadding` of the track's blocks:
    /// padding to drop from the end. A negative value adds nothing.
    pub discard_padding: u64,
}

/// The trim facts of the first track of `kind` in the first Segment. `None`
/// when no track has that kind.
///
/// This reads less than [`Demuxer::open`](super::Demuxer::open). It reads no
/// `Info`, `SeekHead`, `Cues`, `Chapters`, `Tags` or `Attachments`, reads only
/// the fields above of a `TrackEntry`, and accepts a block of an undeclared
/// track. An error in those parts gives no error here.
///
/// On a truncated file, the sum covers the whole blocks before the cut.
///
/// Returns an error when the header or `Tracks` is missing, or when an element
/// it reads is malformed.
pub fn first_track_timing<R: Read + Seek>(
    mut reader: R,
    kind: TrackKind,
) -> Result<Option<TrackTiming>> {
    let input_len = reader
        .seek(SeekFrom::End(0))
        .map_err(|err| Error::io("the input length", 0, err))?;
    let mut src = Source::new(&mut reader, input_len);
    let start = open_segment(&mut src)?;
    let end = start.declared_end.min(input_len);

    let mut tracks = None;
    let mut padding = BTreeMap::new();
    let mut frames = Vec::new();
    let mut clusters = 0u32;
    let mut at = start.data;
    while at < end {
        let Some(header) = src.header_at(at, "Segment")? else {
            break;
        };
        if matches!(header.id, ids::EBML | ids::SEGMENT) {
            // A chained Segment follows. Only the first is read.
            break;
        }
        if header.id == ids::CLUSTER {
            clusters = clusters.saturating_add(1);
            let scanned = scan_cluster(&mut src, at, header, clusters, &mut frames)?;
            add_padding(&mut frames, &mut padding);
            match scanned {
                ClusterEnd::At(next) => {
                    at = next;
                    continue;
                }
                ClusterEnd::Truncated => break,
            }
        }
        let path = format!("Segment/{}", ids::name(header.id));
        let next = (at + header.len).saturating_add(header.known_size(&path, at)?);
        if next > input_len {
            break;
        }
        if header.id == ids::TRACKS && tracks.is_none() {
            let (body, bytes) = read_whole(&mut src, at, header, &path)?;
            tracks = Some(first_of_kind(&element(header.id, at, body, &bytes), kind)?);
        }
        at = next;
    }
    let tracks = tracks.ok_or_else(|| Error::mkv_missing("Segment/Tracks"))?;
    Ok(tracks.map(|(track_id, codec_delay)| TrackTiming {
        track_id,
        codec_delay,
        discard_padding: padding.get(&u64::from(track_id)).copied().unwrap_or(0),
    }))
}

/// Move `frames` out, adding each positive `DiscardPadding` to its track's
/// sum.
fn add_padding(frames: &mut Vec<RawFrame>, padding: &mut BTreeMap<u64, u64>) {
    for frame in frames.drain(..) {
        if let Some(value) = frame.discard_padding.filter(|&value| value > 0) {
            let sum = padding.entry(frame.track).or_default();
            *sum = sum.saturating_add(value.unsigned_abs());
        }
    }
}

/// The `TrackNumber` and `CodecDelay` of the first `TrackEntry` of `kind`.
fn first_of_kind(tracks: &Element<'_>, kind: TrackKind) -> Result<Option<(u32, u64)>> {
    let path = "Segment/Tracks";
    let mut entries = 0usize;
    for child in tracks.children(path) {
        let child = child?;
        if child.id != ids::TRACK_ENTRY {
            continue;
        }
        entries += 1;
        let entry_path = indexed_path(path, ids::TRACK_ENTRY, entries);
        let (mut number, mut track_type, mut codec_delay) = (None, None, 0);
        for field in child.children(&entry_path) {
            let field = field?;
            let at = child_path(&entry_path, field.id);
            match field.id {
                ids::TRACK_NUMBER => number = Some(field.uint(&at)?),
                ids::TRACK_TYPE => track_type = Some(field.uint(&at)?),
                ids::CODEC_DELAY => codec_delay = field.uint(&at)?,
                _ => {}
            }
        }
        if track_type.map(TrackKind::from_matroska_type) != Some(kind) {
            continue;
        }
        let at = format!("{entry_path}/TrackNumber");
        let number = number.ok_or_else(|| Error::mkv_missing(at.clone()))?;
        let id = u32::try_from(number)
            .ok()
            .filter(|&id| id != 0)
            .ok_or_else(|| {
                Error::mkv_malformed(
                    &at,
                    child.offset,
                    format!("track number {number} is outside 1 to {}", u32::MAX),
                )
            })?;
        return Ok(Some((id, codec_delay)));
    }
    Ok(None)
}
