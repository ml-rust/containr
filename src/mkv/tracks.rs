//! The `Tracks` element: one [`Track`] per `TrackEntry`.

use crate::edit::EditList;
use crate::error::{Error, Result};
use crate::matroska::{ContentEncoding, MatroskaTrack};
use crate::mkv::ebml::{Element, child_path, indexed_path};
use crate::mkv::ids;
use crate::mkv::track_media::{parse_audio, parse_video};
use crate::track::{Language, Timebase, Track, TrackKind};

/// The identity matrix in `tkhd` order, for a Matroska track.
const IDENTITY: [i32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

/// Parse the `Tracks` element body. Every track gets `timebase`.
pub(crate) fn parse_tracks(tracks: &Element<'_>, timebase: Timebase) -> Result<Vec<Track>> {
    let path = "Segment/Tracks";
    let mut out: Vec<Track> = Vec::new();
    let mut entries = 0usize;
    for child in tracks.children(path) {
        let child = child?;
        if child.id != ids::TRACK_ENTRY {
            continue;
        }
        entries += 1;
        let entry_path = indexed_path(path, ids::TRACK_ENTRY, entries);
        let track = parse_entry(&child, &entry_path, timebase)?;
        if out.iter().any(|other| other.id == track.id) {
            return Err(Error::mkv_malformed(
                format!("{entry_path}/TrackNumber"),
                child.offset,
                format!("track number {} repeats an earlier TrackEntry", track.id),
            ));
        }
        out.push(track);
    }
    Ok(out)
}

fn parse_entry(entry: &Element<'_>, path: &str, timebase: Timebase) -> Result<Track> {
    let mut number: Option<u64> = None;
    let mut mkv = MatroskaTrack {
        uid: None,
        track_type: 0,
        codec_id: String::new(),
        codec_private: None,
        codec_name: None,
        name: None,
        language: "eng".to_owned(),
        language_bcp47: None,
        default_duration: None,
        codec_delay: 0,
        seek_pre_roll: 0,
        enabled: true,
        default: true,
        forced: false,
        lacing: true,
        content_encodings: Vec::new(),
        video: None,
        audio: None,
    };
    let mut codec_id: Option<String> = None;
    let mut track_type: Option<u64> = None;
    for child in entry.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::TRACK_NUMBER => number = Some(child.uint(&at)?),
            ids::TRACK_UID => mkv.uid = Some(child.uint(&at)?),
            ids::TRACK_TYPE => track_type = Some(child.uint(&at)?),
            ids::FLAG_ENABLED => mkv.enabled = child.uint(&at)? != 0,
            ids::FLAG_DEFAULT => mkv.default = child.uint(&at)? != 0,
            ids::FLAG_FORCED => mkv.forced = child.uint(&at)? != 0,
            ids::FLAG_LACING => mkv.lacing = child.uint(&at)? != 0,
            ids::DEFAULT_DURATION => mkv.default_duration = Some(child.uint(&at)?),
            ids::NAME => mkv.name = Some(child.string(&at)?),
            ids::LANGUAGE => mkv.language = child.string(&at)?,
            ids::LANGUAGE_BCP47 => mkv.language_bcp47 = Some(child.string(&at)?),
            ids::CODEC_ID => codec_id = Some(child.string(&at)?),
            ids::CODEC_PRIVATE => mkv.codec_private = Some(child.binary()),
            ids::CODEC_NAME => mkv.codec_name = Some(child.string(&at)?),
            ids::CODEC_DELAY => mkv.codec_delay = child.uint(&at)?,
            ids::SEEK_PRE_ROLL => mkv.seek_pre_roll = child.uint(&at)?,
            ids::VIDEO => mkv.video = Some(parse_video(&child, &at)?),
            ids::AUDIO => mkv.audio = Some(parse_audio(&child, &at)?),
            ids::CONTENT_ENCODINGS => mkv.content_encodings = parse_encodings(&child, &at)?,
            _ => {}
        }
    }
    let number = number.ok_or_else(|| Error::mkv_missing(format!("{path}/TrackNumber")))?;
    let id = u32::try_from(number)
        .ok()
        .filter(|&id| id != 0)
        .ok_or_else(|| {
            Error::mkv_malformed(
                format!("{path}/TrackNumber"),
                entry.offset,
                format!("track number {number} is outside 1 to {}", u32::MAX),
            )
        })?;
    mkv.track_type = track_type.ok_or_else(|| Error::mkv_missing(format!("{path}/TrackType")))?;
    mkv.codec_id = codec_id.ok_or_else(|| Error::mkv_missing(format!("{path}/CodecID")))?;
    Ok(Track {
        id,
        kind: TrackKind::from_matroska_type(mkv.track_type),
        handler: None,
        timebase,
        duration: None,
        language: language(&mkv.language),
        matrix: IDENTITY,
        display_width: 0,
        display_height: 0,
        sample_entries: Vec::new(),
        edit_list: EditList::default(),
        references: Vec::new(),
        chapter_track: false,
        metadata: Vec::new(),
        sample_count: 0,
        matroska: Some(mkv),
    })
}

/// The packed language of a Matroska `Language` value: three lowercase
/// letters, else `und`.
fn language(value: &str) -> Language {
    match value.as_bytes() {
        &[a, b, c] if [a, b, c].iter().all(u8::is_ascii_lowercase) => Language::Iso639([a, b, c]),
        _ => Language::Iso639(*b"und"),
    }
}

fn parse_encodings(encodings: &Element<'_>, path: &str) -> Result<Vec<ContentEncoding>> {
    let mut out = Vec::new();
    for child in encodings.children(path) {
        let child = child?;
        if child.id != ids::CONTENT_ENCODING {
            continue;
        }
        let at = indexed_path(path, ids::CONTENT_ENCODING, out.len() + 1);
        let mut encoding = ContentEncoding {
            order: 0,
            scope: 1,
            encoding_type: 0,
            compression_algorithm: None,
            compression_settings: None,
            encryption_algorithm: None,
        };
        for field in child.children(&at) {
            let field = field?;
            let field_path = child_path(&at, field.id);
            match field.id {
                ids::CONTENT_ENCODING_ORDER => encoding.order = field.uint(&field_path)?,
                ids::CONTENT_ENCODING_SCOPE => encoding.scope = field.uint(&field_path)?,
                ids::CONTENT_ENCODING_TYPE => encoding.encoding_type = field.uint(&field_path)?,
                ids::CONTENT_COMPRESSION => {
                    // ContentCompAlgo defaults to 0, zlib.
                    encoding.compression_algorithm = Some(0);
                    for item in field.children(&field_path) {
                        let item = item?;
                        let item_path = child_path(&field_path, item.id);
                        match item.id {
                            ids::CONTENT_COMP_ALGO => {
                                encoding.compression_algorithm = Some(item.uint(&item_path)?);
                            }
                            ids::CONTENT_COMP_SETTINGS => {
                                encoding.compression_settings = Some(item.binary());
                            }
                            _ => {}
                        }
                    }
                }
                ids::CONTENT_ENCRYPTION => {
                    encoding.encryption_algorithm = Some(0);
                    for item in field.children(&field_path) {
                        let item = item?;
                        if item.id == ids::CONTENT_ENC_ALGO {
                            let item_path = child_path(&field_path, item.id);
                            encoding.encryption_algorithm = Some(item.uint(&item_path)?);
                        }
                    }
                }
                _ => {}
            }
        }
        out.push(encoding);
    }
    Ok(out)
}
