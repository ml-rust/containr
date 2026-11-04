//! The Matroska/WebM muxer: tracks, packets, tags and the final layout.
//!
//! The layout: EBML header, Segment, `SeekHead`, `Info`, `Tracks`, Clusters,
//! `Cues`, `Chapters`, `Tags`, `Attachments`.

use std::io::{Seek, Write};

use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mkv::ebml_write::{ElementBuf, push_id, push_size};
use crate::mkv::ids;
use crate::mkv::mux::attachments::{AttachedFile, AttachmentIn};
use crate::mkv::mux::bytes::{derive_uid, invalid};
use crate::mkv::mux::chapters::{ChapterPlan, plan_chapters};
use crate::mkv::mux::cluster::OpenCluster;
use crate::mkv::mux::cues::{Cue, MAX_CUE_POINTS};
use crate::mkv::mux::head::{Head, build_head};
use crate::mkv::mux::options::MkvOptions;
use crate::mkv::mux::spec::{PacketIn, TrackId, TrackSpec};
use crate::mkv::mux::spec_check::{check_spec, retarget};
use crate::mkv::mux::tags::check_items;
use crate::mkv::mux::track::{TrackState, earliest};
use crate::mkv::mux::track_entry::track_entry;
use crate::mkv::segment::MAX_ELEMENT_SIZE;

/// The most tracks in one output.
pub const MAX_TRACKS: usize = 1024;

/// A Matroska or WebM muxer.
///
/// [`Muxer::new`] writes the start of the file. Each track's newest packet
/// waits for the next one, since only that tells whether it is the last.
/// Blocks of all tracks interleave by timestamp, with a bounded buffer.
/// [`Muxer::finish`] writes the rest.
///
/// The target must start empty, and the muxer does not truncate it. A target
/// with no seek works through [`NoSeek`](crate::mkv::NoSeek) with
/// [`MkvOptions::seekable`] set to false.
///
/// The last block of each track always carries `BlockDuration`, so the
/// duration of the last frame reads back exact. A block whose duration differs
/// from what a reader takes from `DefaultDuration` also carries it.
#[derive(Debug)]
pub struct Muxer<W> {
    pub(super) writer: W,
    pub(super) options: MkvOptions,
    pub(super) head: Head,
    pub(super) tracks: Vec<TrackState>,
    pub(super) metadata: Vec<MetadataItem>,
    pub(super) chapters: ChapterPlan,
    pub(super) attachments: Vec<AttachedFile>,
    cluster: Option<OpenCluster>,
    pub(super) cues: Vec<Cue>,
    /// `SeekHead` entries: element ID and offset from the Segment body.
    pub(super) seek: Vec<(u32, u64)>,
    /// The offset of the next byte to write.
    pub(super) pos: u64,
    /// Packet bytes held in track queues.
    buffered: usize,
    /// The `Tracks` element has gone out.
    started: bool,
    pub(super) broken: bool,
}

impl<W: Write + Seek> Muxer<W> {
    /// Start a file on `writer`.
    pub fn new(mut writer: W, options: MkvOptions) -> Result<Self> {
        options.check()?;
        let (bytes, head) = build_head(&options)?;
        writer
            .write_all(&bytes)
            .map_err(|err| Error::io("the output: writing the file start", 0, err))?;
        let seek = vec![(ids::INFO, head.info_at - head.segment_data)];
        Ok(Self {
            writer,
            options,
            head,
            tracks: Vec::new(),
            metadata: Vec::new(),
            chapters: ChapterPlan::default(),
            attachments: Vec::new(),
            cluster: None,
            cues: Vec::new(),
            seek,
            pos: bytes.len() as u64,
            buffered: 0,
            started: false,
            broken: false,
        })
    }

    pub(super) fn check_usable(&self) -> Result<()> {
        if self.broken {
            return Err(invalid(
                "an earlier write to the target failed, so the output is incomplete. Start \
                 again on a new target"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Add a track. Every track comes before the first packet.
    pub fn add_track(&mut self, mut spec: TrackSpec) -> Result<TrackId> {
        self.check_usable()?;
        if self.started {
            return Err(invalid(
                "`add_track` came after the first packet. Add every track first".to_owned(),
            ));
        }
        if self.tracks.len() >= MAX_TRACKS {
            return Err(invalid(format!(
                "the output holds {MAX_TRACKS} tracks, the most this muxer accepts. Mux fewer \
                 tracks"
            )));
        }
        check_spec(&spec, self.options.doc_type)?;
        check_items(&spec.metadata)?;
        let number = self.tracks.len() as u32 + 1;
        let taken: Vec<u64> = self.tracks.iter().map(|track| track.uid).collect();
        let uid = match spec.matroska.uid {
            Some(0) => {
                return Err(invalid(format!(
                    "track {number} has `TrackUID` 0. Use a non-zero UID, or `None`"
                )));
            }
            Some(uid) if taken.contains(&uid) => {
                return Err(invalid(format!(
                    "track {number} repeats `TrackUID` {uid}. Give each track its own UID"
                )));
            }
            Some(uid) => uid,
            None => derive_uid(
                spec.matroska.codec_id.as_bytes(),
                u64::from(number),
                |uid| taken.contains(&uid),
            ),
        };
        spec.matroska.uid = Some(uid);
        retarget(&mut spec.metadata, uid)?;
        self.tracks.push(TrackState::new(
            number,
            uid,
            spec,
            self.options.timestamp_scale,
        ));
        Ok(TrackId(number))
    }

    /// Set the Segment-level tags. Each item needs `MetadataScope::Tag` and a
    /// named key. [`tags_from_mp4`](crate::mkv::tags_from_mp4) converts MP4
    /// items.
    pub fn set_metadata(&mut self, items: Vec<MetadataItem>) -> Result<()> {
        check_items(&items)?;
        self.metadata = items;
        Ok(())
    }

    /// Set the chapters: flat, nested ones after their parent, depth first.
    /// A chapter keeps its Matroska UIDs when it has them.
    pub fn set_chapters(&mut self, chapters: Vec<Chapter>) -> Result<()> {
        self.chapters = plan_chapters(&chapters)?;
        Ok(())
    }

    /// Attach a file with a derived UID. Returns the UID. WebM refuses it.
    pub fn add_attachment(&mut self, name: &str, mime: &str, data: &[u8]) -> Result<u64> {
        self.attach(&AttachmentIn {
            name,
            mime,
            description: None,
            uid: None,
            data,
        })
    }

    /// Attach a file, with its description and UID. Returns the UID. WebM
    /// refuses it.
    pub fn attach(&mut self, file: &AttachmentIn<'_>) -> Result<u64> {
        let taken: Vec<u64> = self.attachments.iter().map(|file| file.uid).collect();
        let attached = AttachedFile::new(file, self.options.doc_type, &taken)?;
        let uid = attached.uid;
        self.attachments.push(attached);
        Ok(uid)
    }

    /// Write one packet of `track`, in decode order within the track. A
    /// refused packet leaves the muxer as it was.
    pub fn write_packet(&mut self, track: TrackId, packet: &PacketIn<'_>) -> Result<()> {
        self.check_usable()?;
        let slot = (track.0 as usize)
            .checked_sub(1)
            .filter(|&slot| slot < self.tracks.len())
            .ok_or_else(|| Error::TrackNotFound {
                track_id: track.0,
                available: self.tracks.iter().map(|t| t.number).collect(),
            })?;
        let Some(state) = self.tracks.get(slot) else {
            return Ok(());
        };
        let block = state.accept(packet)?;
        self.start()?;
        if let Some(state) = self.tracks.get_mut(slot) {
            self.buffered = self.buffered.saturating_add(state.push(block));
        }
        self.drain()
    }

    /// Write `bytes` at the end of the output.
    pub(super) fn put(&mut self, bytes: &[u8], what: &str) -> Result<()> {
        if let Err(err) = self.writer.write_all(bytes) {
            self.broken = true;
            return Err(Error::io(
                format!("the output: writing {what}"),
                self.pos,
                err,
            ));
        }
        self.pos += bytes.len() as u64;
        Ok(())
    }

    /// Write a Segment child that a demuxer reads whole, and index it.
    pub(super) fn put_master(&mut self, id: u32, body: &ElementBuf) -> Result<()> {
        let size = body.as_bytes().len() as u64;
        if size > MAX_ELEMENT_SIZE {
            return Err(invalid(format!(
                "element `{}` holds {size} bytes, above the {MAX_ELEMENT_SIZE} a demuxer reads \
                 whole. Write fewer tracks, chapters or tags",
                ids::name(id)
            )));
        }
        self.seek.push((id, self.pos - self.head.segment_data));
        let mut header = Vec::with_capacity(12);
        push_id(&mut header, id);
        push_size(&mut header, id, size)?;
        let name = ids::name(id);
        self.put(&header, &name)?;
        self.put(body.as_bytes(), &name)
    }

    /// Write `Tracks`, once.
    pub(super) fn start(&mut self) -> Result<()> {
        if self.started {
            return Ok(());
        }
        let mut body = ElementBuf::new();
        for track in &self.tracks {
            let entry = track_entry(track.number, track.uid, &track.spec.matroska)?;
            body.master(ids::TRACK_ENTRY, &entry)?;
        }
        self.put_master(ids::TRACKS, &body)?;
        self.started = true;
        Ok(())
    }

    /// Write queued blocks while the interleave rule allows it: every track
    /// holds a block past its newest, so the earliest is known. Past
    /// `max_buffer`, the earliest block leaves regardless.
    fn drain(&mut self) -> Result<()> {
        loop {
            let over = self.buffered > self.options.max_buffer;
            if !over && self.tracks.iter().any(|track| track.queue.len() < 2) {
                return Ok(());
            }
            let Some(slot) = earliest(&self.tracks) else {
                return Ok(());
            };
            self.release(slot)?;
        }
    }

    /// Write the first queued block of the track at `slot`. The newest block
    /// of a track can be its last, so it carries its duration when known.
    pub(super) fn release(&mut self, slot: usize) -> Result<()> {
        let Some(track) = self.tracks.get_mut(slot) else {
            return Ok(());
        };
        let explicit = track.queue.len() == 1;
        let Some(block) = track.queue.pop_front() else {
            return Ok(());
        };
        let duration = track.written_duration(&block, explicit);
        let reference = track.reference(&block);
        track.previous = Some(block.ticks);
        let number = track.number;
        let video = track.is_video();
        self.buffered = self.buffered.saturating_sub(block.data.len());

        let max_ticks = self.options.cluster_ticks();
        let max_bytes = self.options.cluster_bytes;
        let fits = self
            .cluster
            .as_ref()
            .is_some_and(|cluster| cluster.accepts(block.ticks, max_ticks, max_bytes));
        if !fits {
            self.close_cluster()?;
        }
        if self.cluster.is_none() {
            self.cluster = Some(OpenCluster::new(self.pos, block.ticks)?);
        }
        let Some(cluster) = self.cluster.as_mut() else {
            return Ok(());
        };
        let relative = cluster.append(number, &block, duration, reference)?;
        let wanted = block.sync
            && self.options.cues
            && self.cues.len() < MAX_CUE_POINTS
            && (video
                || (!self.tracks.iter().any(TrackState::is_video)
                    && !cluster.cued.contains(&number)));
        if wanted {
            cluster.cued.push(number);
            self.cues.push(Cue {
                time: block.ticks.unsigned_abs(),
                track: number,
                cluster: cluster.start - self.head.segment_data,
                relative,
            });
        }
        Ok(())
    }

    /// Write the open Cluster.
    pub(super) fn close_cluster(&mut self) -> Result<()> {
        let Some(cluster) = self.cluster.take() else {
            return Ok(());
        };
        let header = cluster.header()?;
        self.put(&header, "a Cluster")?;
        self.put(&cluster.body, "a Cluster")
    }
}
