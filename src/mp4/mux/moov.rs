//! The `moov` box: `mvhd`, one `trak` per track, movie metadata and `chpl`.

use mp4_atom::{
    Dinf, Dref, DrefEntry, Elst, ElstEntry, FixedPoint, Matrix, Mvhd, Smhd, Tkhd,
    TrackReferenceTypeBox, Tref, Url, Vmhd,
};

use crate::error::{Container, Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mp4::demuxer::MAX_MOOV_SIZE;
use crate::mp4::mux::bytes::BoxBuf;
use crate::mp4::mux::entry::TEXT_GMHD;
use crate::mp4::mux::metadata::write_metadata;
use crate::mp4::mux::options::MuxOptions;
use crate::mp4::mux::track::TrackState;
use crate::track::{Language, TrackKind};

/// Everything `moov` describes.
pub(crate) struct Movie<'a> {
    pub options: &'a MuxOptions,
    /// Every track, the chapter track last when there is one.
    pub tracks: &'a [TrackState],
    /// The id of the chapter track.
    pub chapter_track: Option<u32>,
    pub metadata: &'a [MetadataItem],
    pub chapters: &'a [Chapter],
}

/// The `tkhd` duration in movie ticks: the edit list's total, else the media
/// duration rounded up.
pub(crate) fn track_duration(track: &TrackState, movie_timescale: u32) -> u64 {
    let edits = &track.spec.edit_list.entries;
    if !edits.is_empty() {
        return edits.iter().fold(0u64, |total, edit| {
            total.saturating_add(edit.segment_duration)
        });
    }
    let scaled = (u128::from(track.table.media_duration) * u128::from(movie_timescale))
        .div_ceil(u128::from(track.spec.timescale.max(1)));
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// The packed 16-bit `mdhd` language field.
fn language_code(language: Language) -> u16 {
    match language {
        Language::Iso639(letters) => letters.iter().fold(0u16, |code, &letter| {
            (code << 5) | u16::from(letter.wrapping_sub(0x60) & 0x1F)
        }),
        Language::Other(code) => code,
    }
}

/// A 16.16 value from its raw 32 bits.
fn fixed_u16(raw: u32) -> FixedPoint<u16> {
    FixedPoint::new((raw >> 16) as u16, (raw & 0xFFFF) as u16)
}

/// Write `hdlr`: the ISO form, or the QuickTime form with a component type
/// and a Pascal-string name.
fn hdlr(
    buf: &mut BoxBuf,
    component: &[u8; 4],
    handler: &[u8; 4],
    name: &str,
    quicktime: bool,
    path: &str,
) -> Result<()> {
    let start = buf.open_full(b"hdlr", 0, 0);
    if quicktime {
        buf.bytes(component);
        buf.bytes(handler);
        buf.zeros(12);
        let name = name.as_bytes();
        let len = u8::try_from(name.len()).map_err(|_| Error::InvalidMux {
            container: Container::Mp4,
            reason: format!(
                "the handler name holds {} bytes. A QuickTime handler name holds at most 255",
                name.len()
            ),
        })?;
        buf.u8(len);
        buf.bytes(name);
    } else {
        buf.u32(0);
        buf.bytes(handler);
        buf.zeros(12);
        buf.bytes(name.as_bytes());
        buf.u8(0);
    }
    buf.close(start, path)
}

fn mdhd(buf: &mut BoxBuf, track: &TrackState, path: &str) -> Result<()> {
    let duration = track.table.media_duration;
    let language = language_code(track.spec.language);
    match u32::try_from(duration) {
        Ok(short) => {
            let start = buf.open_full(b"mdhd", 0, 0);
            buf.u32(0);
            buf.u32(0);
            buf.u32(track.spec.timescale);
            buf.u32(short);
            buf.u16(language);
            buf.u16(0);
            buf.close(start, path)
        }
        Err(_) => {
            let start = buf.open_full(b"mdhd", 1, 0);
            buf.u64(0);
            buf.u64(0);
            buf.u32(track.spec.timescale);
            buf.u64(duration);
            buf.u16(language);
            buf.u16(0);
            buf.close(start, path)
        }
    }
}

/// Write the media header that matches the handler.
fn media_header(buf: &mut BoxBuf, track: &TrackState, path: &str) -> Result<()> {
    match &track.spec.handler.0 {
        b"vide" => buf.encode(&Vmhd::default(), &format!("{path}/vmhd")),
        b"soun" => buf.encode(&Smhd::default(), &format!("{path}/smhd")),
        b"text" => {
            buf.bytes(&TEXT_GMHD);
            Ok(())
        }
        b"subt" => buf.boxed(b"sthd", &[0; 4], &format!("{path}/sthd")),
        _ => buf.boxed(b"nmhd", &[0; 4], &format!("{path}/nmhd")),
    }
}

fn edit_list(buf: &mut BoxBuf, track: &TrackState, path: &str) -> Result<()> {
    let entries = &track.spec.edit_list.entries;
    if entries.is_empty() {
        return Ok(());
    }
    let elst = Elst {
        entries: entries
            .iter()
            .map(|edit| ElstEntry {
                segment_duration: edit.segment_duration,
                // add_track checks that media_time is -1 or above.
                media_time: u64::try_from(edit.media_time).ok(),
                media_rate: FixedPoint::new(
                    (edit.media_rate >> 16) as i16,
                    (edit.media_rate & 0xFFFF) as u16 as i16,
                ),
            })
            .collect(),
    };
    let edts = buf.open(b"edts");
    buf.encode(&elst, &format!("{path}/edts/elst"))?;
    buf.close(edts, &format!("{path}/edts"))
}

fn references(buf: &mut BoxBuf, track: &TrackState, chap: Option<u32>, path: &str) -> Result<()> {
    let mut boxes: Vec<TrackReferenceTypeBox> = track
        .spec
        .references
        .iter()
        .map(|reference| TrackReferenceTypeBox {
            reference_type: reference.kind.0.into(),
            track_ids: reference.track_ids.clone(),
        })
        .collect();
    if let Some(chapter_id) = chap {
        let kind = mp4_atom::FourCC::new(b"chap");
        match boxes.iter_mut().find(|b| b.reference_type == kind) {
            Some(existing) => existing.track_ids.push(chapter_id),
            None => boxes.push(TrackReferenceTypeBox {
                reference_type: kind,
                track_ids: vec![chapter_id],
            }),
        }
    }
    if boxes.is_empty() {
        return Ok(());
    }
    let tref = Tref {
        track_reference_type_boxes: boxes,
    };
    buf.encode(&tref, &format!("{path}/tref"))
}

fn trak(
    buf: &mut BoxBuf,
    movie: &Movie<'_>,
    track: &TrackState,
    chap: Option<u32>,
    shift: u64,
    path: &str,
) -> Result<()> {
    let quicktime = movie.options.brand.quicktime();
    let spec = &track.spec;
    let start = buf.open(b"trak");
    let m = &spec.matrix;
    let tkhd = Tkhd {
        creation_time: 0,
        modification_time: 0,
        track_id: track.id,
        duration: track_duration(track, movie.options.movie_timescale),
        layer: 0,
        alternate_group: match spec.kind {
            TrackKind::Audio => 1,
            TrackKind::Subtitle if Some(track.id) != movie.chapter_track => 2,
            _ => 0,
        },
        enabled: spec.enabled,
        in_movie: true,
        size_is_aspect_ratio: false,
        volume: if spec.kind == TrackKind::Audio {
            FixedPoint::new(1, 0)
        } else {
            FixedPoint::new(0, 0)
        },
        matrix: Matrix {
            a: m[0],
            b: m[1],
            u: m[2],
            c: m[3],
            d: m[4],
            v: m[5],
            x: m[6],
            y: m[7],
            w: m[8],
        },
        width: fixed_u16(spec.display_width),
        height: fixed_u16(spec.display_height),
    };
    buf.encode(&tkhd, &format!("{path}/tkhd"))?;
    edit_list(buf, track, path)?;
    references(buf, track, chap, path)?;

    let mdia_path = format!("{path}/mdia");
    let mdia = buf.open(b"mdia");
    mdhd(buf, track, &format!("{mdia_path}/mdhd"))?;
    hdlr(
        buf,
        b"mhlr",
        &spec.handler.0,
        &spec.handler_name,
        quicktime,
        &format!("{mdia_path}/hdlr"),
    )?;
    let minf_path = format!("{mdia_path}/minf");
    let minf = buf.open(b"minf");
    media_header(buf, track, &minf_path)?;
    if quicktime {
        hdlr(
            buf,
            b"dhlr",
            b"url ",
            "DataHandler",
            true,
            &format!("{minf_path}/hdlr"),
        )?;
    }
    let dinf = Dinf {
        dref: Dref {
            entries: vec![DrefEntry::Url(Url {
                location: String::new(),
            })],
        },
    };
    buf.encode(&dinf, &format!("{minf_path}/dinf"))?;
    track
        .table
        .encode(buf, &track.entry, shift, &format!("{minf_path}/stbl"))?;
    buf.close(minf, &minf_path)?;
    buf.close(mdia, &mdia_path)?;
    write_metadata(buf, &spec.metadata, &[], path)?;
    buf.close(start, path)
}

/// The tracks that carry the `chap` reference: every video and audio track,
/// else the first track.
fn chapter_holders(movie: &Movie<'_>) -> Vec<bool> {
    let regular = |track: &TrackState| Some(track.id) != movie.chapter_track;
    let mut holders: Vec<bool> = movie
        .tracks
        .iter()
        .map(|track| {
            movie.chapter_track.is_some()
                && regular(track)
                && matches!(track.spec.kind, TrackKind::Video | TrackKind::Audio)
        })
        .collect();
    if movie.chapter_track.is_some()
        && !holders.contains(&true)
        && let Some(first) = movie.tracks.iter().position(regular)
        && let Some(slot) = holders.get_mut(first)
    {
        *slot = true;
    }
    holders
}

/// Build `moov` with every chunk offset moved up by `shift`.
pub(crate) fn build_moov(movie: &Movie<'_>, shift: u64) -> Result<Vec<u8>> {
    let mut buf = BoxBuf::new();
    let moov = buf.open(b"moov");
    let timescale = movie.options.movie_timescale;
    let duration = movie
        .tracks
        .iter()
        .map(|track| track_duration(track, timescale))
        .max()
        .unwrap_or(0);
    let next_track_id = movie
        .tracks
        .iter()
        .map(|track| track.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mvhd = Mvhd {
        creation_time: 0,
        modification_time: 0,
        timescale,
        duration,
        rate: FixedPoint::new(1, 0),
        volume: FixedPoint::new(1, 0),
        matrix: Matrix::default(),
        next_track_id,
    };
    buf.encode(&mvhd, "moov/mvhd")?;
    let holders = chapter_holders(movie);
    for (n, (track, holds)) in movie.tracks.iter().zip(holders).enumerate() {
        let chap = if holds { movie.chapter_track } else { None };
        trak(
            &mut buf,
            movie,
            track,
            chap,
            shift,
            &format!("moov/trak[{}]", n + 1),
        )?;
    }
    write_metadata(&mut buf, movie.metadata, movie.chapters, "moov")?;
    buf.close(moov, "moov")?;
    let bytes = buf.into_bytes();
    if bytes.len() as u64 > MAX_MOOV_SIZE {
        return Err(Error::InvalidMux {
            container: Container::Mp4,
            reason: format!(
                "the moov box would span {} bytes, above the {MAX_MOOV_SIZE} a reader \
                 accepts. Split the movie into shorter files",
                bytes.len()
            ),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_packs_three_letters() {
        assert_eq!(language_code(Language::Iso639(*b"und")), 0x55C4);
        assert_eq!(language_code(Language::Iso639(*b"eng")), 0x15C7);
        assert_eq!(language_code(Language::Other(0x7FFF)), 0x7FFF);
    }

    #[test]
    fn fixed_point_keeps_raw_bits() {
        let value = fixed_u16(0x0060_8000);
        assert_eq!((value.integer(), value.decimal()), (0x60, 0x8000));
    }
}
