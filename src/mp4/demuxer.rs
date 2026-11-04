//! The MP4/MOV demuxer: top-level box scan, `moov` parse, packet reads.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use mp4_atom::{Decode, Ftyp, Header};

use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mp4::boxes::RawBox;
use crate::mp4::chapters::read_chapter_track;
use crate::mp4::moov::{Movie, parse_moov};
use crate::mp4::packets::{Packets, TrackPackets};
use crate::mp4::tables::SampleCursor;
use crate::packet::Packet;
use crate::read::read_range;
use crate::track::{FourCc, Track};

/// The largest `moov` box read, in bytes. The whole box is held in memory.
pub const MAX_MOOV_SIZE: u64 = 1 << 30;
/// The largest `ftyp` box read, in bytes.
pub const MAX_FTYP_SIZE: u64 = 64 * 1024;

/// An MP4 or QuickTime MOV demuxer over a seekable reader.
///
/// [`Demuxer::open`] reads every top-level box header and the whole `moov`,
/// wherever it sits. Packet bytes stay in the input until
/// [`Demuxer::read_packet`] reads them.
#[derive(Debug)]
pub struct Demuxer<R> {
    reader: R,
    input_len: u64,
    major_brand: Option<FourCc>,
    compatible_brands: Vec<FourCc>,
    movie: Movie,
}

impl Demuxer<BufReader<File>> {
    /// Open the file at `path`.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let in_file = |source: Error| Error::InFile {
            file: path.to_path_buf(),
            source: Box::new(source),
        };
        let file = File::open(path).map_err(|err| in_file(Error::io("the file", 0, err)))?;
        Self::open(BufReader::new(file)).map_err(in_file)
    }
}

impl<R: Read + Seek> Demuxer<R> {
    /// Parse `ftyp` and `moov` from `reader`. `moov` can sit before or after
    /// `mdat`.
    pub fn open(mut reader: R) -> Result<Self> {
        let input_len = reader
            .seek(SeekFrom::End(0))
            .map_err(|err| Error::io("the input length", 0, err))?;
        let mut major_brand = None;
        let mut compatible_brands = Vec::new();
        let mut moov: Option<(u64, Vec<u8>)> = None;
        let mut pos = 0u64;
        while input_len - pos >= 8 {
            let (kind, box_len) = top_level_header(&mut reader, pos, input_len)?;
            match &kind.0 {
                b"ftyp" if major_brand.is_none() => {
                    let bytes = read_box(&mut reader, pos, box_len, "ftyp", MAX_FTYP_SIZE)?;
                    let ftyp = RawBox::whole(&bytes, pos, "ftyp")?.decode::<Ftyp>("ftyp")?;
                    major_brand = Some(FourCc(ftyp.major_brand.into()));
                    compatible_brands = ftyp
                        .compatible_brands
                        .into_iter()
                        .map(|brand| FourCc(brand.into()))
                        .collect();
                }
                b"moov" => {
                    if moov.is_some() {
                        return Err(Error::mp4_malformed(
                            "moov",
                            pos,
                            "a second moov box follows the first",
                        ));
                    }
                    moov = Some((
                        pos,
                        read_box(&mut reader, pos, box_len, "moov", MAX_MOOV_SIZE)?,
                    ));
                }
                b"moof" => {
                    return Err(Error::Fragmented {
                        path: "moof".to_owned(),
                        offset: pos,
                    });
                }
                _ => {}
            }
            pos += box_len;
        }
        let (moov_offset, moov_bytes) = moov.ok_or_else(|| Error::mp4_missing("moov"))?;
        let mut movie = parse_moov(&moov_bytes, moov_offset)?;
        drop(moov_bytes);
        for (track, tables) in movie.tracks.iter().zip(&movie.tables) {
            if track.chapter_track {
                let chapters = read_chapter_track(&mut reader, input_len, track, tables)?;
                movie.chapters.extend(chapters);
            }
        }
        Ok(Self {
            reader,
            input_len,
            major_brand,
            compatible_brands,
            movie,
        })
    }

    /// Read the bytes of `packet` into `buf`, which it replaces.
    pub fn read_packet_into(&mut self, packet: &Packet, buf: &mut Vec<u8>) -> Result<()> {
        read_range(&mut self.reader, self.input_len, packet, buf)
    }

    /// Read the bytes of `packet`.
    pub fn read_packet(&mut self, packet: &Packet) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.read_packet_into(packet, &mut buf)?;
        Ok(buf)
    }
}

impl<R> Demuxer<R> {
    /// The tracks in file order.
    pub fn tracks(&self) -> &[Track] {
        &self.movie.tracks
    }

    /// The track with `track_id`.
    pub fn track(&self, track_id: u32) -> Option<&Track> {
        self.movie.tracks.iter().find(|track| track.id == track_id)
    }

    /// The `ftyp` major brand. QuickTime files can omit `ftyp`.
    pub fn major_brand(&self) -> Option<FourCc> {
        self.major_brand
    }

    /// The `ftyp` compatible brands.
    pub fn compatible_brands(&self) -> &[FourCc] {
        &self.compatible_brands
    }

    /// Ticks per second of the movie timescale (`mvhd`). Edit list segment
    /// durations count in it.
    pub fn movie_timescale(&self) -> u32 {
        self.movie.timescale
    }

    /// The movie duration in movie timescale ticks (`mvhd`).
    pub fn movie_duration(&self) -> u64 {
        self.movie.duration
    }

    /// Movie-level metadata from `moov/udta` and `moov/meta`.
    pub fn metadata(&self) -> &[MetadataItem] {
        &self.movie.metadata
    }

    /// Chapters from `chpl` and from QuickTime chapter tracks, in that order.
    pub fn chapters(&self) -> &[Chapter] {
        &self.movie.chapters
    }

    /// The input length in bytes.
    pub fn input_len(&self) -> u64 {
        self.input_len
    }

    /// Every packet of every track, in file order.
    pub fn packets(&self) -> Packets {
        Packets::new(
            self.movie
                .tables
                .iter()
                .cloned()
                .map(SampleCursor::new)
                .collect(),
        )
    }

    /// The packets of one track, in decode order.
    pub fn track_packets(&self, track_id: u32) -> Result<TrackPackets> {
        self.movie
            .tracks
            .iter()
            .zip(&self.movie.tables)
            .find(|(track, _)| track.id == track_id)
            .map(|(_, tables)| TrackPackets::new(SampleCursor::new(tables.clone())))
            .ok_or_else(|| Error::TrackNotFound {
                track_id,
                available: self.movie.tracks.iter().map(|track| track.id).collect(),
            })
    }

    /// The reader, given back.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

/// Read a top-level box header at `pos`. Returns the type and the whole box
/// length. A box of size 0 runs to the input end.
pub(crate) fn top_level_header<R: Read + Seek>(
    reader: &mut R,
    pos: u64,
    input_len: u64,
) -> Result<(FourCc, u64)> {
    let remaining = input_len - pos;
    let mut bytes = [0u8; 16];
    let take = remaining.min(16) as usize;
    reader
        .seek(SeekFrom::Start(pos))
        .and_then(|_| reader.read_exact(&mut bytes[..take]))
        .map_err(|err| Error::io("a top-level box header", pos, err))?;
    let mut cursor = &bytes[..take];
    let header = Header::decode(&mut cursor).map_err(|err| {
        Error::mp4_malformed(
            "top-level box",
            pos,
            format!("the box header does not decode: {err}"),
        )
    })?;
    let kind = FourCc(header.kind.into());
    let header_len = (take - cursor.len()) as u64;
    let box_len = match header.size {
        None => remaining,
        Some(body) => header_len.saturating_add(body as u64),
    };
    if box_len > remaining {
        // A truncated `mdat` keeps the samples before the cut readable.
        if &kind.0 == b"mdat" {
            return Ok((kind, remaining));
        }
        return Err(Error::mp4_malformed(
            kind.to_string(),
            pos,
            format!("the box declares {box_len} bytes, but {remaining} remain in the input"),
        ));
    }
    Ok((kind, box_len))
}

/// Read one whole top-level box, bounded by `limit`.
pub(crate) fn read_box<R: Read + Seek>(
    reader: &mut R,
    pos: u64,
    box_len: u64,
    path: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    if box_len > limit {
        return Err(Error::mp4_too_large(path, box_len, limit));
    }
    let mut bytes = vec![0u8; box_len as usize];
    reader
        .seek(SeekFrom::Start(pos))
        .and_then(|_| reader.read_exact(&mut bytes))
        .map_err(|err| Error::io(format!("box `{path}`"), pos, err))?;
    Ok(bytes)
}
