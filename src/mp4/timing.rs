//! One track's timing facts, read from `moov` with no sample tables.
//!
//! [`first_track_timing`] reads `mvhd`, then each track's `hdlr`. It stops at
//! the first track of the wanted kind and reads that track's `mdhd` and
//! `elst`. A fragmented file stores its `moov` as any other file does, so
//! `moof` and `mvex` need no refusal here.

use std::io::{Read, Seek, SeekFrom};

use crate::edit::EditList;
use crate::error::{Error, Result};
use crate::mp4::boxes::{RawBox, collect_children};
use crate::mp4::demuxer::{MAX_MOOV_SIZE, read_box, top_level_header};
use crate::mp4::trak::{edits, require};
use crate::track::TrackKind;

/// The timing facts of one track. The caller applies them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackTiming {
    /// Ticks per second of the track (`mdhd`), raw. A malformed file can
    /// store 0.
    pub timescale: u32,
    /// The raw edit list, with the `mvhd` timescale. A malformed file can
    /// store a timescale of 0.
    pub edit_list: EditList,
}

/// The timing facts of the first track of `kind` in the first `moov`. `None`
/// when no track has that kind.
///
/// This reads less than [`Demuxer::open`](super::Demuxer::open). It holds the
/// whole `moov` in memory and builds no sample tables. It ignores every box
/// after `moov`, and every box of a track that does not hold the facts above,
/// so an error in those boxes gives no error here.
///
/// Returns an error when `moov` is missing or larger than [`MAX_MOOV_SIZE`],
/// or when a box it reads is malformed.
pub fn first_track_timing<R: Read + Seek>(
    mut reader: R,
    kind: TrackKind,
) -> Result<Option<TrackTiming>> {
    let input_len = reader
        .seek(SeekFrom::End(0))
        .map_err(|err| Error::io("the input length", 0, err))?;
    let mut pos = 0u64;
    while input_len - pos >= 8 {
        let (box_kind, box_len) = top_level_header(&mut reader, pos, input_len)?;
        if &box_kind.0 == b"moov" {
            let bytes = read_box(&mut reader, pos, box_len, "moov", MAX_MOOV_SIZE)?;
            return parse(&bytes, pos, kind);
        }
        pos += box_len;
    }
    Err(Error::mp4_missing("moov"))
}

/// Find the first track of `kind` in a whole `moov` read from `offset`.
fn parse(bytes: &[u8], offset: u64, kind: TrackKind) -> Result<Option<TrackTiming>> {
    let moov = RawBox::whole(bytes, offset, "moov")?;
    let children = collect_children(&moov, 0, "moov")?;
    let movie_timescale = timescale(&require(&children, b"mvhd", "moov")?, "moov/mvhd")?;
    let traks = children.iter().filter(|child| &child.kind.0 == b"trak");
    for (n, trak) in traks.enumerate() {
        let path = format!("moov/trak[{}]", n + 1);
        let trak_children = collect_children(trak, 0, &path)?;
        let mdia_path = format!("{path}/mdia");
        let mdia = require(&trak_children, b"mdia", &path)?;
        let mdia_children = collect_children(&mdia, 0, &mdia_path)?;
        let hdlr_path = format!("{mdia_path}/hdlr");
        let hdlr = require(&mdia_children, b"hdlr", &mdia_path)?;
        let mut hdlr_reader = hdlr.reader(&hdlr_path);
        hdlr_reader.skip(8, "version, flags and pre_defined")?;
        if TrackKind::from_handler(hdlr_reader.fourcc("handler_type")?) != kind {
            continue;
        }
        let mdhd = require(&mdia_children, b"mdhd", &mdia_path)?;
        return Ok(Some(TrackTiming {
            timescale: timescale(&mdhd, &format!("{mdia_path}/mdhd"))?,
            edit_list: EditList {
                movie_timescale,
                entries: edits(&trak_children, &path)?,
            },
        }));
    }
    Ok(None)
}

/// The timescale field of an `mvhd` or `mdhd` box. Version 1 stores 64-bit
/// creation and modification times before it.
fn timescale(raw: &RawBox<'_>, path: &str) -> Result<u32> {
    let mut reader = raw.reader(path);
    let version = reader.u8("version")?;
    reader.skip(3, "flags")?;
    let times = if version == 1 { 16 } else { 8 };
    reader.skip(times, "creation and modification times")?;
    reader.u32("timescale")
}
