//! The Matroska/WebM demuxer: the top-level scan, track and packet
//! assembly, packet and attachment reads.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem, MetadataScope, TagScope, TagTarget};
use crate::mkv::attachments::Attachment;
use crate::mkv::frames::build_packets;
use crate::mkv::index::{CuePoint, SeekEntry};
use crate::mkv::info::{EbmlHeader, SegmentInfo};
use crate::mkv::packets::{Packets, TrackPackets};
use crate::mkv::segment::{scan, tracks_of};
use crate::mkv::source::Source;
use crate::packet::Packet;
use crate::read::read_range;
use crate::track::Track;

/// A Matroska or WebM demuxer over a seekable reader.
///
/// [`Demuxer::open`] reads the EBML header and every Segment child, and
/// indexes every frame of every Cluster. Packet and attachment bytes stay
/// in the input until [`Demuxer::read_packet`] or
/// [`Demuxer::read_attachment`] reads them.
#[derive(Debug)]
pub struct Demuxer<R> {
    reader: R,
    input_len: u64,
    ebml: EbmlHeader,
    info: SegmentInfo,
    segment_data: u64,
    tracks: Vec<Track>,
    packets: Arc<[Packet]>,
    metadata: Vec<MetadataItem>,
    chapters: Vec<Chapter>,
    attachments: Vec<Attachment>,
    cues: Vec<CuePoint>,
    seek_head: Vec<SeekEntry>,
    truncated: bool,
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
    /// Read the EBML header and the first Segment from `reader`.
    pub fn open(mut reader: R) -> Result<Self> {
        let input_len = reader
            .seek(SeekFrom::End(0))
            .map_err(|err| Error::io("the input length", 0, err))?;
        let found = scan(&mut Source::new(&mut reader, input_len))?;
        let mut tracks = tracks_of(&found)?;
        let packets = build_packets(&found.frames, &mut tracks, found.info.timestamp_scale)?;
        let metadata = place_tags(found.tags, &mut tracks);
        Ok(Self {
            reader,
            input_len,
            ebml: found.ebml,
            info: found.info,
            segment_data: found.segment_data,
            tracks,
            packets: packets.into(),
            metadata,
            chapters: found.chapters,
            attachments: found.attachments,
            cues: found.cues,
            seek_head: found.seek_head,
            truncated: found.truncated,
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

    /// Read the `FileData` bytes of `attachment`.
    pub fn read_attachment(&mut self, attachment: &Attachment) -> Result<Vec<u8>> {
        let path = "Segment/Attachments/AttachedFile/FileData";
        let mut src = Source::new(&mut self.reader, self.input_len);
        src.read_body(attachment.offset, attachment.size, path, self.input_len)
    }
}

impl<R> Demuxer<R> {
    /// The tracks in file order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track with `track_id`, its `TrackNumber`.
    pub fn track(&self, track_id: u32) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    /// Every packet of every track, in file order.
    pub fn packets(&self) -> Packets {
        Packets::new(Arc::clone(&self.packets))
    }

    /// The packets of one track, in decode order.
    pub fn track_packets(&self, track_id: u32) -> Result<TrackPackets> {
        if self.track(track_id).is_none() {
            return Err(Error::TrackNotFound {
                track_id,
                available: self.tracks.iter().map(|track| track.id).collect(),
            });
        }
        Ok(TrackPackets::new(Arc::clone(&self.packets), track_id))
    }

    /// Segment-level metadata: every `SimpleTag` whose target is not one of
    /// the tracks. A track's tags are in [`Track::metadata`].
    pub fn metadata(&self) -> &[MetadataItem] {
        &self.metadata
    }

    /// The chapter atoms of every edition, in file order.
    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }

    /// The attached files.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    /// The `Cues` points. Empty when the file has no `Cues`.
    pub fn cues(&self) -> &[CuePoint] {
        &self.cues
    }

    /// The entries of every `SeekHead`.
    pub fn seek_head(&self) -> &[SeekEntry] {
        &self.seek_head
    }

    /// The EBML header facts, `DocType` included.
    pub fn ebml_header(&self) -> &EbmlHeader {
        &self.ebml
    }

    /// The Segment `Info` facts.
    pub fn info(&self) -> &SegmentInfo {
        &self.info
    }

    /// The byte offset of the Segment body. `SeekPosition` and
    /// `CueClusterPosition` count from it.
    pub fn segment_data_offset(&self) -> u64 {
        self.segment_data
    }

    /// True when the input ends inside the Segment. The packets before the
    /// cut are indexed. Elements after it are absent.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The input length in bytes.
    pub fn input_len(&self) -> u64 {
        self.input_len
    }

    /// The reader, given back.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

/// Move each tag that targets a track's `TrackUID` into that track.
/// Returns the rest.
fn place_tags(tags: Vec<MetadataItem>, tracks: &mut [Track]) -> Vec<MetadataItem> {
    let mut rest = Vec::new();
    for item in tags {
        let uid = match item.scope {
            MetadataScope::Tag(TagScope {
                target: TagTarget::Track(uid),
                ..
            }) => Some(uid),
            _ => None,
        };
        let owner = match uid {
            Some(uid) => tracks.iter_mut().find(|track| {
                track
                    .matroska
                    .as_ref()
                    .is_some_and(|mkv| mkv.uid == Some(uid))
            }),
            None => None,
        };
        match owner {
            Some(track) => track.metadata.push(item),
            None => rest.push(item),
        }
    }
    rest
}
