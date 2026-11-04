//! The MP4/MOV muxer: tracks, packets, metadata and the final layout.

use std::io::{Read, Seek, Write};

use crate::edit::{EditEntry, EditList};
use crate::error::{Error, Result};
use crate::metadata::{Chapter, MetadataItem};
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::chapters::chapter_track;
use crate::mp4::mux::entry::build_entry;
use crate::mp4::mux::interleave::{earliest_open, next_chunk};
use crate::mp4::mux::layout::{patch_mdat, settle_moov, shift_up, write_at, write_head};
use crate::mp4::mux::metadata::{Level, check_chapters, check_items};
use crate::mp4::mux::moov::{Movie, build_moov, track_duration};
use crate::mp4::mux::options::MuxOptions;
use crate::mp4::mux::spec::{PacketFormat, PacketIn, TrackId, TrackSpec};
use crate::mp4::mux::track::TrackState;

/// The most tracks in one movie.
pub const MAX_TRACKS: usize = 1024;

/// An MP4 or QuickTime MOV muxer over a seekable writer.
///
/// [`Muxer::new`] writes `ftyp` and the start of `mdat`. Packets collect in
/// per-track chunks of bounded duration, and chunks reach `mdat` in decode
/// time order across tracks, with a bounded buffer. [`Muxer::finish`] writes
/// `moov`.
///
/// The target must start empty: chunk offsets count from its start, and the
/// muxer does not truncate it. Packet timestamps are stored as given, never
/// rescaled.
#[derive(Debug)]
pub struct Muxer<W> {
    writer: W,
    options: MuxOptions,
    tracks: Vec<TrackState>,
    metadata: Vec<MetadataItem>,
    chapters: Vec<Chapter>,
    /// The offset of the 16 reserved header bytes before the sample data.
    reserved: u64,
    /// The offset of the next sample data byte.
    pos: u64,
    /// Packet bytes held in chunks.
    buffered: usize,
    started: bool,
    broken: bool,
}

impl<W> Muxer<W> {
    /// The packet bytes held in chunks not yet written. After each
    /// [`Muxer::write_packet`] this is at most [`MuxOptions::max_buffer`].
    pub fn buffered_bytes(&self) -> usize {
        self.buffered
    }
}

impl<W: Write + Seek> Muxer<W> {
    /// Start a movie on `writer`.
    pub fn new(mut writer: W, options: MuxOptions) -> Result<Self> {
        options.check()?;
        let (reserved, pos) = write_head(&mut writer, options.brand)?;
        Ok(Self {
            writer,
            options,
            tracks: Vec::new(),
            metadata: Vec::new(),
            chapters: Vec::new(),
            reserved,
            pos,
            buffered: 0,
            started: false,
            broken: false,
        })
    }

    fn check_usable(&self) -> Result<()> {
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
                "the movie holds {MAX_TRACKS} tracks, the most this muxer accepts. Mux fewer tracks"
            )));
        }
        if spec.timescale == 0 {
            return Err(invalid(
                "`TrackSpec::timescale` is 0. Use a positive tick rate".to_owned(),
            ));
        }
        check_items(&spec.metadata, Level::Track)?;
        spec.edit_list = checked_edit_list(&spec.edit_list, self.options.movie_timescale)?;
        let (fourcc, entry) = build_entry(&spec.sample_entry, spec.pixel_aspect, spec.colour)?;
        if spec.packet_format == PacketFormat::PlainText && &fourcc.0 != b"tx3g" {
            return Err(invalid(format!(
                "`PacketFormat::PlainText` writes `tx3g` samples, but the sample entry is \
                 `{fourcc}`. Use `SampleEntrySpec::Tx3g`"
            )));
        }
        let id = self.tracks.len() as u32 + 1;
        self.tracks.push(TrackState::new(
            id,
            spec,
            entry,
            self.options.chunk_duration_ms,
        ));
        Ok(TrackId(id))
    }

    /// Set the movie-level metadata, written to `moov/udta` and `moov/meta`.
    pub fn set_metadata(&mut self, items: Vec<MetadataItem>) -> Result<()> {
        check_items(&items, Level::Movie)?;
        self.metadata = items;
        Ok(())
    }

    /// Set the chapters, in start order, each at one distinct start. They
    /// are written as `chpl` and as a QuickTime chapter track.
    pub fn set_chapters(&mut self, chapters: Vec<Chapter>) -> Result<()> {
        check_chapters(&chapters)?;
        self.chapters = chapters;
        Ok(())
    }

    /// Write one packet of `track`, in decode order within the track.
    /// A refused packet leaves the muxer as it was.
    pub fn write_packet(&mut self, track: TrackId, packet: &PacketIn<'_>) -> Result<()> {
        self.check_usable()?;
        let slot = (track.0 as usize)
            .checked_sub(1)
            .filter(|&slot| slot < self.tracks.len())
            .ok_or_else(|| Error::TrackNotFound {
                track_id: track.0,
                available: self.tracks.iter().map(|t| t.id).collect(),
            })?;
        let signed = self.options.brand.signed_offsets();
        let Some(state) = self.tracks.get_mut(slot) else {
            return Ok(());
        };
        self.buffered += state.push(packet, signed)?;
        self.started = true;
        self.drain(false)
    }

    /// Replace the edit list of `track`, at any point before
    /// [`Muxer::finish`]. A single-pass writer learns a track's duration only
    /// at its end, so it sets the edit that needs it last. Segment durations
    /// convert to the movie timescale when the conversion is exact.
    pub fn set_edit_list(&mut self, track: TrackId, list: &EditList) -> Result<()> {
        self.check_usable()?;
        let list = checked_edit_list(list, self.options.movie_timescale)?;
        let slot = (track.0 as usize)
            .checked_sub(1)
            .filter(|&slot| slot < self.tracks.len())
            .ok_or_else(|| Error::TrackNotFound {
                track_id: track.0,
                available: self.tracks.iter().map(|t| t.id).collect(),
            })?;
        if let Some(state) = self.tracks.get_mut(slot) {
            state.spec.edit_list = list;
        }
        Ok(())
    }

    /// Write queued chunks while the interleave rule allows it. With
    /// `force`, write every queued chunk.
    fn drain(&mut self, force: bool) -> Result<()> {
        loop {
            let over = self.buffered > self.options.max_buffer;
            if let Some(slot) = next_chunk(&self.tracks, force || over) {
                self.write_chunk(slot)?;
                continue;
            }
            if over && let Some(slot) = earliest_open(&self.tracks) {
                if let Some(track) = self.tracks.get_mut(slot) {
                    track.close_open();
                }
                continue;
            }
            return Ok(());
        }
    }

    fn write_chunk(&mut self, slot: usize) -> Result<()> {
        let Some(track) = self.tracks.get_mut(slot) else {
            return Ok(());
        };
        let Some(chunk) = track.closed.pop_front() else {
            return Ok(());
        };
        track.table.push_chunk(self.pos, chunk.samples);
        if let Err(err) = self.writer.write_all(&chunk.data) {
            self.broken = true;
            return Err(Error::io("the output: writing mdat", self.pos, err));
        }
        self.pos += chunk.data.len() as u64;
        self.buffered = self.buffered.saturating_sub(chunk.data.len());
        Ok(())
    }

    /// Check that every `tref` entry names a track of this movie.
    fn check_references(&self) -> Result<()> {
        for track in &self.tracks {
            for reference in &track.spec.references {
                for &id in &reference.track_ids {
                    if id == 0 || id as usize > self.tracks.len() {
                        return Err(invalid(format!(
                            "track {} has a `{}` reference to track {id}, but the movie \
                             holds tracks 1 to {}. Reference output track ids",
                            track.id,
                            reference.kind,
                            self.tracks.len()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Add the chapter track when the movie has chapters and tracks. Returns
    /// its id.
    fn add_chapter_track(&mut self) -> Result<Option<u32>> {
        if self.chapters.is_empty() || self.tracks.is_empty() {
            return Ok(None);
        }
        let timescale = self.options.movie_timescale;
        let duration = self
            .tracks
            .iter()
            .map(|track| track_duration(track, timescale))
            .max()
            .unwrap_or(0);
        let id = self.tracks.len() as u32 + 1;
        let track = chapter_track(
            id,
            &self.chapters,
            timescale,
            duration,
            self.options.chunk_duration_ms,
        )?;
        self.tracks.push(track);
        Ok(Some(id))
    }
}

impl<W: Read + Write + Seek> Muxer<W> {
    /// End the movie: set each last sample's duration, write the remaining
    /// chunks and `moov`, and return the writer.
    ///
    /// The target must also implement [`Read`], since faststart reads `mdat`
    /// back to shift it. Files and `Cursor<Vec<u8>>` do.
    pub fn finish(mut self) -> Result<W> {
        self.check_usable()?;
        self.check_references()?;
        for track in &mut self.tracks {
            track.finish()?;
        }
        self.drain(true)?;
        let chapter_id = self.add_chapter_track()?;
        self.drain(true)?;
        patch_mdat(&mut self.writer, self.reserved, self.pos)?;
        let movie = Movie {
            options: &self.options,
            tracks: &self.tracks,
            chapter_track: chapter_id,
            metadata: &self.metadata,
            chapters: &self.chapters,
        };
        if self.options.faststart {
            let moov = settle_moov(|shift| build_moov(&movie, shift))?;
            shift_up(&mut self.writer, self.reserved, self.pos, moov.len() as u64)?;
            write_at(&mut self.writer, self.reserved, &moov)?;
        } else {
            let moov = build_moov(&movie, 0)?;
            write_at(&mut self.writer, self.pos, &moov)?;
        }
        self.writer
            .flush()
            .map_err(|err| Error::io("the output: flushing", self.pos, err))?;
        Ok(self.writer)
    }
}

/// Check an edit list and convert its segment durations to the movie
/// timescale. A conversion that is not exact is refused.
fn checked_edit_list(list: &EditList, movie_timescale: u32) -> Result<EditList> {
    if list.entries.is_empty() {
        return Ok(EditList {
            movie_timescale,
            entries: Vec::new(),
        });
    }
    if list.movie_timescale == 0 {
        return Err(invalid(
            "`EditList::movie_timescale` is 0 on a list with edits. Set the timescale its \
             segment durations count in"
                .to_owned(),
        ));
    }
    let mut entries = Vec::with_capacity(list.entries.len());
    for edit in &list.entries {
        if edit.media_time < EditEntry::EMPTY_MEDIA_TIME {
            return Err(invalid(format!(
                "an edit has media_time {}. Use -1 for an empty edit, or a value of 0 or more",
                edit.media_time
            )));
        }
        let scaled = u128::from(edit.segment_duration) * u128::from(movie_timescale);
        let divisor = u128::from(list.movie_timescale);
        let segment_duration = u64::try_from(scaled / divisor)
            .ok()
            .filter(|_| scaled.is_multiple_of(divisor))
            .ok_or_else(|| {
                invalid(format!(
                    "an edit of {} ticks at {} ticks per second has no exact value at the \
                     movie timescale {movie_timescale}. Set `MuxOptions::movie_timescale` \
                     to {}",
                    edit.segment_duration, list.movie_timescale, list.movie_timescale
                ))
            })?;
        entries.push(EditEntry {
            segment_duration,
            ..*edit
        });
    }
    Ok(EditList {
        movie_timescale,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(segment_duration: u64, media_time: i64) -> EditEntry {
        EditEntry {
            segment_duration,
            media_time,
            media_rate: EditEntry::NORMAL_RATE,
        }
    }

    #[test]
    fn edit_lists_convert_only_when_exact() {
        let list = EditList {
            movie_timescale: 1000,
            entries: vec![entry(500, 0)],
        };
        let converted = checked_edit_list(&list, 48_000).expect("the conversion is exact");
        assert_eq!(converted.entries[0].segment_duration, 24_000);
        assert_eq!(converted.movie_timescale, 48_000);
        let inexact = EditList {
            movie_timescale: 1000,
            entries: vec![entry(1, 0)],
        };
        assert!(checked_edit_list(&inexact, 1).is_err());
    }

    #[test]
    fn edit_lists_refuse_a_negative_media_time() {
        let list = EditList {
            movie_timescale: 1000,
            entries: vec![entry(500, -2)],
        };
        assert!(checked_edit_list(&list, 1000).is_err());
    }
}
