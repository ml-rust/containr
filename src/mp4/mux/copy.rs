//! Stream copy: a track spec and packet timing from a demuxed track.
//!
//! An MP4 track copies as it is. A Matroska track stores no decode time, so
//! [`StreamCopy::plan`] derives it: the sorted presentation times, moved so
//! the first decode time is 0, in the order the packets arrive. Each `pts`
//! stays the stored value, so `ctts` holds what the sort moves, with signed
//! offsets when a frame presents before it decodes.
//!
//! Negative Matroska timestamps mark decoder delay (`CodecDelay`), and a
//! `DiscardPadding` on the last block trims the end. MP4 expresses both with
//! an edit list, so the plan adds one edit from time 0 to the end of the
//! last packet. `BlockDuration` already excludes the padding in the files
//! FFmpeg writes, and the edit list uses the stored duration.

use crate::edit::{EditEntry, EditList};
use crate::error::{Container, Error, Result};
use crate::matroska::MatroskaTrack;
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::matroska::{spec_from_matroska, timescale_of};
use crate::mp4::mux::metadata::rescale;
use crate::mp4::mux::spec::{PacketFormat, PacketIn, SampleEntrySpec, TrackSpec};
use crate::packet::Packet;
use crate::track::Track;

impl TrackSpec {
    /// A spec from a demuxed MP4 or Matroska track.
    ///
    /// An MP4 track keeps its raw sample entry, timescale, language, matrix,
    /// display size, edit list, metadata and references. `chap` references
    /// are dropped, since the muxer writes its own chapter track from
    /// [`Muxer::set_chapters`](crate::mp4::Muxer::set_chapters). A track with
    /// two or more sample descriptions gives
    /// [`Error::SampleDescriptionChange`].
    ///
    /// A Matroska track maps when its codec is H.264, HEVC, AV1, AAC, Opus
    /// or `S_TEXT/UTF8`. Any other codec gives [`Error::Unsupported`], which
    /// names Matroska. Use [`StreamCopy::plan`] for packet timing and the
    /// edit list a Matroska track needs.
    ///
    /// Two limits apply. The muxer does not copy MP4 `sgpd` and `sbgp`
    /// sample-group boxes. Matroska attachments have no MP4 form, so the
    /// muxer never sees them. A caller that copies a Matroska file to MP4
    /// must check `Demuxer::attachments()` on the Matroska demuxer and
    /// refuse the copy when it lists any, naming MKV as the container that
    /// holds attachments.
    pub fn copy_from(track: &Track) -> Result<Self> {
        match &track.matroska {
            Some(mkv) => spec_from_matroska(track, mkv),
            None => spec_from_mp4(track),
        }
    }
}

fn spec_from_mp4(track: &Track) -> Result<TrackSpec> {
    let entry = match track.sample_entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(invalid(format!(
                "track {} has no sample description",
                track.id
            )));
        }
        entries => {
            return Err(Error::SampleDescriptionChange {
                track_id: track.id,
                descriptions: entries.len() as u64,
            });
        }
    };
    let (timescale, factor) = timescale_of(track.timebase)?;
    if factor != 1 {
        return Err(invalid(format!(
            "track {} has timebase {}/{}. An MP4 track counts in 1/timescale seconds",
            track.id, track.timebase.num, track.timebase.den
        )));
    }
    let mut spec = TrackSpec::new(
        track.kind,
        timescale,
        SampleEntrySpec::Raw(entry.raw.clone()),
    );
    if let Some(handler) = track.handler {
        spec.handler = handler;
    }
    spec.language = track.language;
    spec.matrix = track.matrix;
    spec.display_width = track.display_width;
    spec.display_height = track.display_height;
    spec.edit_list = track.edit_list.clone();
    spec.references = track
        .references
        .iter()
        .filter(|reference| &reference.kind.0 != b"chap")
        .cloned()
        .collect();
    spec.metadata = track.metadata.clone();
    Ok(spec)
}

/// One packet's timing, in the output track's ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PacketTiming {
    /// Presentation timestamp.
    pub pts: i64,
    /// Decode timestamp.
    pub dts: i64,
    /// Duration. `None` leaves the last packet to
    /// [`TrackSpec::default_duration`].
    pub duration: Option<u64>,
}

/// A track spec and packet timing for the stream copy of one track.
///
/// The copy drops MP4 `sgpd` and `sbgp` sample-group boxes. Matroska
/// attachments have no MP4 form, so the copy never sees them. A caller that
/// copies a Matroska file to MP4 must check `Demuxer::attachments()` and
/// refuse the copy when it lists any, naming MKV as the container that
/// holds attachments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamCopy {
    /// The spec to pass to [`Muxer::add_track`](crate::mp4::Muxer::add_track).
    pub spec: TrackSpec,
    timing: Vec<PacketTiming>,
}

impl StreamCopy {
    /// Plan the copy of `track`. `packets` holds every packet of the track,
    /// in decode order, as the demuxer's `track_packets` yields them.
    /// `movie_timescale` is the output's [`MuxOptions::movie_timescale`].
    ///
    /// [`MuxOptions::movie_timescale`]: crate::mp4::MuxOptions::movie_timescale
    pub fn plan(track: &Track, packets: &[Packet], movie_timescale: u32) -> Result<Self> {
        for (n, packet) in packets.iter().enumerate() {
            if packet.track_id != track.id || packet.index != n as u64 {
                return Err(invalid(format!(
                    "`StreamCopy::plan` got packet {} of track {} at position {n} for track \
                     {}. Pass the track's packets in decode order",
                    packet.index, packet.track_id, track.id
                )));
            }
        }
        let mut spec = TrackSpec::copy_from(track)?;
        let timing = match &track.matroska {
            Some(mkv) => {
                let timing = matroska_timing(track, mkv, &spec, packets)?;
                spec.edit_list = trim_edit_list(&spec, packets, &timing, movie_timescale)?;
                timing
            }
            None => mp4_timing(track, packets)?,
        };
        Ok(Self { spec, timing })
    }

    /// The timing of every packet, in decode order.
    pub fn timing(&self) -> &[PacketTiming] {
        &self.timing
    }

    /// The packet to write for a demuxed `packet` and its bytes.
    pub fn packet<'a>(&self, packet: &Packet, data: &'a [u8]) -> Result<PacketIn<'a>> {
        let timing = usize::try_from(packet.index)
            .ok()
            .and_then(|index| self.timing.get(index))
            .ok_or_else(|| {
                invalid(format!(
                    "packet {} is past the {} packets planned for this track",
                    packet.index,
                    self.timing.len()
                ))
            })?;
        Ok(PacketIn {
            data,
            pts: timing.pts,
            dts: timing.dts,
            duration: timing.duration,
            sync: packet.sync,
        })
    }
}

fn mp4_timing(track: &Track, packets: &[Packet]) -> Result<Vec<PacketTiming>> {
    packets
        .iter()
        .map(|packet| {
            if packet.description_index != 1 {
                return Err(Error::SampleDescriptionChange {
                    track_id: track.id,
                    descriptions: u64::from(packet.description_index),
                });
            }
            let dts = packet.dts.ok_or_else(|| {
                invalid(format!(
                    "packet {} of MP4 track {} has no dts",
                    packet.index, track.id
                ))
            })?;
            Ok(PacketTiming {
                pts: packet.pts,
                dts,
                duration: packet.duration,
            })
        })
        .collect()
}

fn scale(value: i64, factor: u64, track_id: u32) -> Result<i64> {
    i64::try_from(factor)
        .ok()
        .and_then(|factor| value.checked_mul(factor))
        .ok_or_else(|| {
            invalid(format!(
                "track {track_id}: timestamp {value} times the tick factor {factor} overflows \
                 64 bits"
            ))
        })
}

fn matroska_timing(
    track: &Track,
    mkv: &MatroskaTrack,
    spec: &TrackSpec,
    packets: &[Packet],
) -> Result<Vec<PacketTiming>> {
    let (_, factor) = timescale_of(track.timebase)?;
    let last = packets.len().checked_sub(1);
    for (n, packet) in packets.iter().enumerate() {
        match packet.discard_padding {
            Some(padding) if padding < 0 => {
                return Err(unsupported_padding(track, mkv, packet.index, "negative"));
            }
            Some(_) if Some(n) != last => {
                return Err(unsupported_padding(track, mkv, packet.index, "mid-stream"));
            }
            _ => {}
        }
    }
    let durations: Vec<Option<u64>> = packets
        .iter()
        .map(|packet| {
            packet
                .duration
                .map(|duration| {
                    duration.checked_mul(factor).ok_or_else(|| {
                        invalid(format!(
                            "track {}: duration {duration} times the tick factor {factor} \
                             overflows 64 bits",
                            track.id
                        ))
                    })
                })
                .transpose()
        })
        .collect::<Result<_>>()?;
    let mut pts = Vec::with_capacity(packets.len());
    for packet in packets {
        pts.push(scale(packet.pts, factor, track.id)?);
    }
    if spec.packet_format == PacketFormat::PlainText {
        return Ok(pts
            .iter()
            .zip(&durations)
            .map(|(&pts, &duration)| PacketTiming {
                pts,
                dts: pts,
                duration,
            })
            .collect());
    }
    let mut decode = pts.clone();
    decode.sort_unstable();
    let origin = decode.first().copied().unwrap_or(0);
    let mut timing = Vec::with_capacity(pts.len());
    for (n, &presentation) in pts.iter().enumerate() {
        let dts = decode.get(n).copied().unwrap_or(0).saturating_sub(origin);
        let duration = match decode.get(n + 1) {
            Some(&next) => {
                let next = next.saturating_sub(origin);
                Some(next.saturating_sub(dts) as u64)
            }
            None => durations.get(n).copied().flatten(),
        };
        timing.push(PacketTiming {
            pts: presentation,
            dts,
            duration,
        });
    }
    Ok(timing)
}

fn unsupported_padding(track: &Track, mkv: &MatroskaTrack, index: u64, kind: &str) -> Error {
    Error::Unsupported {
        container: Container::Mp4,
        what: format!(
            "{kind} `DiscardPadding` on packet {index} of track {} (codec `{}`): MP4 trims \
             only the track start and end with an edit list",
            track.id, mkv.codec_id
        ),
        use_instead: Container::Matroska,
    }
}

/// The edit that drops negative timestamps and the padding of the last
/// packet. Empty when the track has neither.
fn trim_edit_list(
    spec: &TrackSpec,
    packets: &[Packet],
    timing: &[PacketTiming],
    movie_timescale: u32,
) -> Result<EditList> {
    let negative = timing.iter().any(|packet| packet.pts < 0);
    let padded = packets
        .iter()
        .any(|packet| packet.discard_padding.is_some());
    if !negative && !padded {
        return Ok(EditList::default());
    }
    let end = timing.iter().fold(0i64, |end, packet| {
        let duration = packet
            .duration
            .or_else(|| spec.default_duration.map(u64::from))
            .unwrap_or(0);
        let duration = i64::try_from(duration).unwrap_or(i64::MAX);
        end.max(packet.pts.saturating_add(duration))
    });
    if end <= 0 {
        return Ok(EditList::default());
    }
    if movie_timescale == 0 {
        return Err(invalid(
            "`movie_timescale` is 0. Pass the value in `MuxOptions`".to_owned(),
        ));
    }
    Ok(EditList {
        movie_timescale,
        entries: vec![EditEntry {
            segment_duration: rescale(end as u64, spec.timescale, movie_timescale),
            media_time: 0,
            media_rate: EditEntry::NORMAL_RATE,
        }],
    })
}
