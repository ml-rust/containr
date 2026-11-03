//! Decode a `stbl` box and check its tables against each other.

use mp4_atom::{Co64, Ctts, Stco, Stsc, Stss, Stsz, Stts};

use crate::error::{Error, Result};
use crate::mp4::boxes::{RawBox, collect_children};
use crate::mp4::sample_entry::parse_stsd;
use crate::mp4::tables::chunk::ChunkMap;
use crate::mp4::tables::size::SampleSizes;
use crate::mp4::tables::sync::SyncSamples;
use crate::mp4::tables::time::{Run, duration_runs, offset_runs, run_total};
use crate::track::{SampleEntry, TrackKind};

/// The checked sample tables of one track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SampleTables {
    /// The `stbl` box path, for errors.
    pub path: String,
    /// The `stsc` box path, for errors.
    pub stsc_path: String,
    pub track_id: u32,
    pub sample_count: u64,
    pub durations: Vec<Run<u32>>,
    /// `None` when the track has no `ctts`.
    pub offsets: Option<Vec<Run<i64>>>,
    pub chunks: ChunkMap,
    pub sizes: SampleSizes,
    pub sync: SyncSamples,
}

/// The `stbl` children this demuxer reads, each at most once.
#[derive(Default)]
struct Found<'a> {
    stsd: Option<RawBox<'a>>,
    stts: Option<RawBox<'a>>,
    ctts: Option<RawBox<'a>>,
    stsc: Option<RawBox<'a>>,
    stsz: Option<RawBox<'a>>,
    stz2: Option<RawBox<'a>>,
    stco: Option<RawBox<'a>>,
    co64: Option<RawBox<'a>>,
    stss: Option<RawBox<'a>>,
}

impl<'a> Found<'a> {
    fn collect(stbl: &RawBox<'a>, path: &str) -> Result<Self> {
        let mut found = Self::default();
        for child in collect_children(stbl, 0, path)? {
            let slot = match &child.kind.0 {
                b"stsd" => &mut found.stsd,
                b"stts" => &mut found.stts,
                b"ctts" => &mut found.ctts,
                b"stsc" => &mut found.stsc,
                b"stsz" => &mut found.stsz,
                b"stz2" => &mut found.stz2,
                b"stco" => &mut found.stco,
                b"co64" => &mut found.co64,
                b"stss" => &mut found.stss,
                _ => continue,
            };
            if slot.is_some() {
                return Err(Error::mp4_malformed(
                    format!("{path}/{}", child.kind),
                    child.offset,
                    "the box appears twice in one stbl",
                ));
            }
            *slot = Some(child);
        }
        Ok(found)
    }
}

fn required<'a>(raw: Option<RawBox<'a>>, path: &str, name: &str) -> Result<RawBox<'a>> {
    raw.ok_or_else(|| Error::mp4_missing(format!("{path}/{name}")))
}

/// One of two alternative boxes, which must not both appear.
fn either<'a>(
    first: Option<RawBox<'a>>,
    second: Option<RawBox<'a>>,
    path: &str,
    names: (&str, &str),
) -> Result<RawBox<'a>> {
    match (first, second) {
        (Some(raw), None) | (None, Some(raw)) => Ok(raw),
        (Some(_), Some(raw)) => Err(Error::mp4_malformed(
            format!("{path}/{}", names.1),
            raw.offset,
            format!(
                "the stbl holds both {} and {}. Remove one",
                names.0, names.1
            ),
        )),
        (None, None) => Err(Error::mp4_missing(format!("{path}/{}", names.0))),
    }
}

/// Decode `stbl` and check that its tables describe one sample count.
pub(crate) fn parse_stbl(
    stbl: &RawBox<'_>,
    path: &str,
    kind: TrackKind,
    track_id: u32,
) -> Result<(Vec<SampleEntry>, SampleTables)> {
    let found = Found::collect(stbl, path)?;
    let stsd_path = format!("{path}/stsd");
    let entries = parse_stsd(&required(found.stsd, path, "stsd")?, &stsd_path, kind)?;

    let size_box = either(found.stsz, found.stz2, path, ("stsz", "stz2"))?;
    let size_path = format!("{path}/{}", size_box.kind);
    let sizes = if &size_box.kind.0 == b"stz2" {
        SampleSizes::parse_stz2(&size_box, &size_path)?
    } else {
        SampleSizes::from_stsz(size_box.decode::<Stsz>(&size_path)?)
    };
    let sample_count = sizes.count();

    let stts_path = format!("{path}/stts");
    let durations = duration_runs(required(found.stts, path, "stts")?.decode::<Stts>(&stts_path)?);
    let covered = run_total(&durations);
    if covered != sample_count {
        return Err(Error::mp4_inconsistent(
            stts_path,
            format!(
                "stts covers {covered} samples, but {} holds {sample_count}",
                size_box.kind
            ),
        ));
    }

    let offsets = match found.ctts {
        Some(raw) => {
            let ctts_path = format!("{path}/ctts");
            let runs = offset_runs(raw.decode::<Ctts>(&ctts_path)?);
            let covered = run_total(&runs);
            if covered != sample_count {
                return Err(Error::mp4_inconsistent(
                    ctts_path,
                    format!(
                        "ctts covers {covered} samples, but {} holds {sample_count}",
                        size_box.kind
                    ),
                ));
            }
            Some(runs)
        }
        None => None,
    };

    let offset_box = either(found.stco, found.co64, path, ("stco", "co64"))?;
    let offset_path = format!("{path}/{}", offset_box.kind);
    let chunk_offsets = if &offset_box.kind.0 == b"co64" {
        offset_box.decode::<Co64>(&offset_path)?.entries
    } else {
        let stco = offset_box.decode::<Stco>(&offset_path)?;
        stco.entries.into_iter().map(u64::from).collect()
    };

    let stsc_path = format!("{path}/stsc");
    let stsc = required(found.stsc, path, "stsc")?.decode::<Stsc>(&stsc_path)?;
    let chunks = ChunkMap::new(stsc, chunk_offsets, entries.len(), &stsc_path)?;
    let mapped = chunks.sample_count(&stsc_path)?;
    if mapped != sample_count {
        return Err(Error::mp4_inconsistent(
            stsc_path,
            format!(
                "stsc places {mapped} samples in the chunks of {}, but {} holds {sample_count}",
                offset_box.kind, size_box.kind
            ),
        ));
    }

    let stss_path = format!("{path}/stss");
    let stss = match found.stss {
        Some(raw) => Some(raw.decode::<Stss>(&stss_path)?),
        None => None,
    };
    let sync = SyncSamples::new(stss, sample_count, &stss_path)?;

    Ok((
        entries,
        SampleTables {
            path: path.to_owned(),
            stsc_path,
            track_id,
            sample_count,
            durations,
            offsets,
            chunks,
            sizes,
            sync,
        },
    ))
}
