//! One `trak` box: header, media, edits, references and sample tables.

use mp4_atom::{Elst, FixedPoint, Mdhd, Tkhd, Tref};

use crate::edit::{EditEntry, EditList};
use crate::error::{Error, Result};
use crate::mp4::boxes::{RawBox, collect_children};
use crate::mp4::metadata::{parse_meta, parse_udta};
use crate::mp4::tables::{SampleTables, parse_stbl};
use crate::track::{FourCc, Language, Timebase, Track, TrackKind, TrackReference};

/// Find the one child of `kind`. A second one is an error.
fn single<'a>(children: &[RawBox<'a>], kind: &[u8; 4], path: &str) -> Result<Option<RawBox<'a>>> {
    let mut found = None;
    for child in children.iter().filter(|child| &child.kind.0 == kind) {
        if found.is_some() {
            return Err(Error::mp4_malformed(
                format!("{path}/{}", child.kind),
                child.offset,
                "the box appears twice in its parent",
            ));
        }
        found = Some(*child);
    }
    Ok(found)
}

pub(crate) fn require<'a>(
    children: &[RawBox<'a>],
    kind: &[u8; 4],
    path: &str,
) -> Result<RawBox<'a>> {
    single(children, kind, path)?
        .ok_or_else(|| Error::mp4_missing(format!("{path}/{}", FourCc(*kind))))
}

/// A 16.16 fixed-point value as its raw 32 bits.
fn fixed_u32(value: &FixedPoint<u16>) -> u32 {
    (u32::from(value.integer()) << 16) | u32::from(value.decimal())
}

/// A signed 16.16 fixed-point value as its raw 32 bits.
fn fixed_i32(value: &FixedPoint<i16>) -> i32 {
    (((value.integer() as u16 as u32) << 16) | (value.decimal() as u16 as u32)) as i32
}

/// Parse a `trak`. `path` names it, such as `moov/trak[2]`.
pub(crate) fn parse_trak(
    trak: &RawBox<'_>,
    path: &str,
    movie_timescale: u32,
) -> Result<(Track, SampleTables)> {
    let children = collect_children(trak, 0, path)?;
    let tkhd_path = format!("{path}/tkhd");
    let tkhd = require(&children, b"tkhd", path)?.decode::<Tkhd>(&tkhd_path)?;
    let m = &tkhd.matrix;

    let mdia_path = format!("{path}/mdia");
    let mdia = require(&children, b"mdia", path)?;
    let mdia_children = collect_children(&mdia, 0, &mdia_path)?;
    let mdhd_path = format!("{mdia_path}/mdhd");
    let mdhd_raw = require(&mdia_children, b"mdhd", &mdia_path)?;
    let mdhd = mdhd_raw.decode::<Mdhd>(&mdhd_path)?;
    if mdhd.timescale == 0 {
        return Err(Error::mp4_malformed(
            mdhd_path,
            mdhd_raw.offset,
            "timescale is 0",
        ));
    }
    let language = mdhd_language(&mdhd_raw, &mdhd_path)?;

    let hdlr_path = format!("{mdia_path}/hdlr");
    let hdlr = require(&mdia_children, b"hdlr", &mdia_path)?;
    let mut hdlr_reader = hdlr.reader(&hdlr_path);
    hdlr_reader.skip(8, "version, flags and pre_defined")?;
    let handler = hdlr_reader.fourcc("handler_type")?;
    let kind = TrackKind::from_handler(handler);

    let minf_path = format!("{mdia_path}/minf");
    let minf = require(&mdia_children, b"minf", &mdia_path)?;
    let minf_children = collect_children(&minf, 0, &minf_path)?;
    let stbl_path = format!("{minf_path}/stbl");
    let stbl = require(&minf_children, b"stbl", &minf_path)?;
    let (sample_entries, tables) = parse_stbl(&stbl, &stbl_path, kind, tkhd.track_id)?;

    let edit_list = EditList {
        movie_timescale,
        entries: edits(&children, path)?,
    };

    let references = match single(&children, b"tref", path)? {
        Some(raw) => raw
            .decode::<Tref>(&format!("{path}/tref"))?
            .track_reference_type_boxes
            .into_iter()
            .map(|reference| TrackReference {
                kind: FourCc(reference.reference_type.into()),
                track_ids: reference.track_ids,
            })
            .collect(),
        None => Vec::new(),
    };

    let mut metadata = Vec::new();
    if let Some(udta) = single(&children, b"udta", path)? {
        metadata.extend(parse_udta(&udta, &format!("{path}/udta"))?.items);
    }
    if let Some(meta) = single(&children, b"meta", path)? {
        metadata.extend(parse_meta(&meta, &format!("{path}/meta"))?);
    }

    let track = Track {
        id: tkhd.track_id,
        kind,
        handler: Some(handler),
        timebase: Timebase::from_timescale(mdhd.timescale),
        duration: Some(mdhd.duration),
        language,
        matrix: [m.a, m.b, m.u, m.c, m.d, m.v, m.x, m.y, m.w],
        display_width: fixed_u32(&tkhd.width),
        display_height: fixed_u32(&tkhd.height),
        sample_entries,
        edit_list,
        references,
        chapter_track: false,
        metadata,
        sample_count: tables.sample_count,
        matroska: None,
    };
    Ok((track, tables))
}

/// The raw `mdhd` language field: the 16 bits after the duration.
fn mdhd_language(mdhd: &RawBox<'_>, path: &str) -> Result<Language> {
    let mut reader = mdhd.reader(path);
    let version = reader.u8("version")?;
    reader.skip(3, "flags")?;
    let times = if version == 1 { 28 } else { 16 };
    reader.skip(times, "times, timescale and duration")?;
    Ok(Language::from_code(reader.u16("language")?))
}

/// The `edts/elst` entries, raw.
pub(crate) fn edits(children: &[RawBox<'_>], path: &str) -> Result<Vec<EditEntry>> {
    let Some(edts) = single(children, b"edts", path)? else {
        return Ok(Vec::new());
    };
    let edts_path = format!("{path}/edts");
    let edts_children = collect_children(&edts, 0, &edts_path)?;
    let Some(elst) = single(&edts_children, b"elst", &edts_path)? else {
        return Ok(Vec::new());
    };
    let elst_path = format!("{edts_path}/elst");
    let decoded = elst.decode::<Elst>(&elst_path)?;
    decoded
        .entries
        .into_iter()
        .map(|entry| -> Result<EditEntry> {
            let media_time = match entry.media_time {
                None => EditEntry::EMPTY_MEDIA_TIME,
                Some(time) => i64::try_from(time).map_err(|_| {
                    Error::mp4_malformed(
                        &elst_path,
                        elst.offset,
                        format!("media_time {time} exceeds 2^63"),
                    )
                })?,
            };
            Ok(EditEntry {
                segment_duration: entry.segment_duration,
                media_time,
                media_rate: fixed_i32(&entry.media_rate),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_keeps_raw_bits() {
        assert_eq!(fixed_u32(&FixedPoint::new(96, 0)), 96 << 16);
        assert_eq!(fixed_i32(&FixedPoint::new(1, 0)), EditEntry::NORMAL_RATE);
        assert_eq!(fixed_i32(&FixedPoint::new(-1, 0)), -65536);
        assert_eq!(fixed_i32(&FixedPoint::new(0, 0)), 0);
    }
}
