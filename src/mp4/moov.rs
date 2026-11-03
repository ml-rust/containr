//! The `moov` box: movie header, tracks, metadata and `chpl` chapters.

use std::collections::HashSet;
use std::sync::Arc;

use mp4_atom::Mvhd;

use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mp4::boxes::{RawBox, collect_children};
use crate::mp4::metadata::{parse_meta, parse_udta};
use crate::mp4::tables::SampleTables;
use crate::mp4::trak::parse_trak;
use crate::track::Track;

/// Everything `moov` declares.
#[derive(Debug)]
pub(crate) struct Movie {
    pub timescale: u32,
    pub duration: u64,
    pub tracks: Vec<Track>,
    /// The sample tables of `tracks`, at the same positions.
    pub tables: Vec<Arc<SampleTables>>,
    pub metadata: Vec<MetadataItem>,
    pub chapters: Vec<Chapter>,
}

/// Parse a whole `moov` box read from `offset` in the input.
pub(crate) fn parse_moov(bytes: &[u8], offset: u64) -> Result<Movie> {
    let moov = RawBox::whole(bytes, offset, "moov")?;
    let children = collect_children(&moov, 0, "moov")?;

    let mut mvhd = None;
    let mut traks = Vec::new();
    let mut metadata = Vec::new();
    let mut chapters = Vec::new();
    for child in &children {
        match &child.kind.0 {
            b"mvhd" => {
                if mvhd.is_some() {
                    return Err(Error::mp4_malformed(
                        "moov/mvhd",
                        child.offset,
                        "the box appears twice in moov",
                    ));
                }
                mvhd = Some(child.decode::<Mvhd>("moov/mvhd")?);
            }
            b"trak" => traks.push(*child),
            b"mvex" => {
                return Err(Error::Fragmented {
                    path: "moov/mvex".to_owned(),
                    offset: child.offset,
                });
            }
            b"udta" => {
                let udta = parse_udta(child, "moov/udta")?;
                metadata.extend(udta.items);
                chapters.extend(udta.chapters);
            }
            b"meta" => metadata.extend(parse_meta(child, "moov/meta")?),
            _ => {}
        }
    }
    let mvhd = mvhd.ok_or_else(|| Error::mp4_missing("moov/mvhd"))?;

    let mut tracks = Vec::with_capacity(traks.len());
    let mut tables = Vec::with_capacity(traks.len());
    let mut ids = HashSet::new();
    for (n, trak) in traks.iter().enumerate() {
        let path = format!("moov/trak[{}]", n + 1);
        let (track, table) = parse_trak(trak, &path, mvhd.timescale)?;
        if !ids.insert(track.id) {
            return Err(Error::mp4_malformed(
                format!("{path}/tkhd"),
                trak.offset,
                format!("track id {} is already in use by an earlier trak", track.id),
            ));
        }
        tracks.push(track);
        tables.push(Arc::new(table));
    }
    mark_chapter_tracks(&mut tracks);

    Ok(Movie {
        timescale: mvhd.timescale,
        duration: mvhd.duration,
        tracks,
        tables,
        metadata,
        chapters,
    })
}

/// Flag every track that a `chap` reference names. A reference to an absent
/// track id names nothing.
fn mark_chapter_tracks(tracks: &mut [Track]) {
    let named: HashSet<u32> = tracks
        .iter()
        .flat_map(|track| track.references.iter())
        .filter(|reference| &reference.kind.0 == b"chap")
        .flat_map(|reference| reference.track_ids.iter().copied())
        .collect();
    for track in tracks.iter_mut() {
        track.chapter_track = named.contains(&track.id);
    }
}
