//! The end of the file: `Cues`, `Chapters`, `Tags`, `Attachments`, and the
//! patches a seekable output takes.

use std::io::{Seek, SeekFrom, Write};

use crate::error::{Error, Result};
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;
use crate::mkv::mux::attachments::attachments_header;
use crate::mkv::mux::chapters::chapters_body;
use crate::mkv::mux::cues::cues_body;
use crate::mkv::mux::head::{seek_head, segment_size};
use crate::mkv::mux::muxer::Muxer;
use crate::mkv::mux::tags::{Targets, append_tags, check_targets};
use crate::mkv::mux::track::earliest;

impl<W: Write + Seek> Muxer<W> {
    /// Write `Cues`, `Chapters`, `Tags` and `Attachments`.
    fn write_tail(&mut self) -> Result<()> {
        if !self.cues.is_empty() {
            let mut cues = std::mem::take(&mut self.cues);
            let body = cues_body(&mut cues)?;
            self.put_master(ids::CUES, &body)?;
        }
        if !self.chapters.editions.is_empty() {
            let body = chapters_body(&self.chapters)?;
            self.put_master(ids::CHAPTERS, &body)?;
        }
        let mut tags = ElementBuf::new();
        append_tags(&mut tags, &self.metadata)?;
        for track in &self.tracks {
            append_tags(&mut tags, &track.spec.metadata)?;
        }
        if !tags.is_empty() {
            self.put_master(ids::TAGS, &tags)?;
        }
        if !self.attachments.is_empty() {
            self.seek
                .push((ids::ATTACHMENTS, self.pos - self.head.segment_data));
            let header = attachments_header(&self.attachments)?;
            self.put(&header, "Attachments")?;
            let files = std::mem::take(&mut self.attachments);
            for file in &files {
                for part in file.parts() {
                    self.put(part, "an AttachedFile")?;
                }
            }
        }
        Ok(())
    }

    /// Check every tag target against what the output holds.
    fn check_tag_targets(&self) -> Result<()> {
        let tracks: Vec<u64> = self.tracks.iter().map(|track| track.uid).collect();
        let editions: Vec<u64> = self
            .chapters
            .editions
            .iter()
            .filter_map(|edition| edition.uid)
            .collect();
        let attachments: Vec<u64> = self.attachments.iter().map(|file| file.uid).collect();
        let targets = Targets {
            tracks: &tracks,
            editions: &editions,
            chapters: &self.chapters.uids,
            attachments: &attachments,
        };
        check_targets(&self.metadata, &targets)
    }

    /// Replace bytes written earlier.
    fn patch(&mut self, at: u64, bytes: &[u8], what: &str) -> Result<()> {
        let result = self
            .writer
            .seek(SeekFrom::Start(at))
            .and_then(|_| self.writer.write_all(bytes));
        result.map_err(|err| {
            self.broken = true;
            Error::io(format!("the output: patching {what}"), at, err)
        })
    }

    /// Patch `SeekHead`, the Segment size and `Duration`, and leave the target
    /// at its end.
    fn patch_head(&mut self) -> Result<()> {
        if let Some(at) = self.head.seek_head_at {
            let bytes = seek_head(&self.seek)?;
            self.patch(at, &bytes, "SeekHead")?;
        }
        if let Some(at) = self.head.duration_at {
            let end = self.tracks.iter().map(|track| track.end).max().unwrap_or(0);
            // `Duration` is a float in the format. Nothing reads timing back
            // from it, and every block keeps its integer timestamp.
            let bits = (end as f64).to_bits().to_be_bytes();
            self.patch(at, &bits, "Duration")?;
        }
        if self.options.seekable {
            let size = segment_size(self.pos - self.head.segment_data)?;
            self.patch(self.head.segment_size_at, &size, "the Segment size")?;
            self.writer
                .seek(SeekFrom::Start(self.pos))
                .map_err(|err| Error::io("the output: seeking to its end", self.pos, err))?;
        }
        Ok(())
    }

    /// End the file and return the writer. A track whose last packet has no
    /// duration and no `DefaultDuration` gives [`Error::MissingDuration`].
    pub fn finish(mut self) -> Result<W> {
        self.check_usable()?;
        for track in &self.tracks {
            track.check_last()?;
        }
        self.check_tag_targets()?;
        self.start()?;
        while let Some(slot) = earliest(&self.tracks) {
            self.release(slot)?;
        }
        self.close_cluster()?;
        self.write_tail()?;
        self.patch_head()?;
        self.writer
            .flush()
            .map_err(|err| Error::io("the output: flushing", self.pos, err))?;
        Ok(self.writer)
    }
}
