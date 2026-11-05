//! A Matroska source whose last block stores no duration, copied to MP4. The
//! source is built in memory, so these tests hold with any FFmpeg or none.

use std::io::Cursor;

use containr::mp4::{MuxOptions, Muxer, StreamCopy};
use containr::{EditEntry, Error, Packet, Result};

use crate::common::mkv_synth::{BARE_TRACK, BareKind, bare_file};
use crate::mux::support::{open_mkv, open_mp4};

/// Plan, write and finish an MP4 copy of the one track.
fn copy(source: &[u8]) -> Result<Vec<u8>> {
    let mut input = open_mkv(source);
    let track = input.tracks()[0].clone();
    let packets: Vec<Packet> = input
        .track_packets(BARE_TRACK)?
        .collect::<Result<Vec<_>>>()?;
    let options = MuxOptions::default();
    let plan = StreamCopy::plan(&track, &packets, options.movie_timescale)?;
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options)?;
    let id = muxer.add_track(plan.spec.clone())?;
    for packet in &packets {
        let data = input.read_packet(packet)?;
        muxer.write_packet(id, &plan.packet(packet, &data)?)?;
    }
    Ok(muxer.finish()?.into_inner())
}

/// The sample durations of an MP4 copy of `source`.
fn sample_durations(source: &[u8]) -> Vec<Option<u64>> {
    let copied = copy(source).unwrap_or_else(|err| panic!("{err}"));
    let output = open_mp4(&copied);
    let id = output.tracks()[0].id;
    let packets = output
        .track_packets(id)
        .and_then(|packets| packets.collect::<Result<Vec<_>>>())
        .unwrap_or_else(|err| panic!("{err}"));
    packets.iter().map(|packet| packet.duration).collect()
}

#[test]
fn a_last_audio_sample_takes_the_gap_before_it() {
    let source = bare_file(BareKind::Opus, &[(0, None), (20, None), (40, None)]);
    assert_eq!(sample_durations(&source), [Some(20), Some(20), Some(20)]);
}

#[test]
fn discard_padding_comes_off_the_gap_and_ends_the_edit() {
    let source = bare_file(
        BareKind::Opus,
        &[(0, None), (20, None), (40, Some(13_500_000))],
    );
    assert_eq!(sample_durations(&source), [Some(20), Some(20), Some(7)]);
    let copied = copy(&source).unwrap_or_else(|err| panic!("{err}"));
    let output = open_mp4(&copied);
    let track = &output.tracks()[0];
    assert_eq!(track.timebase.den, 1000, "the track counts in milliseconds");
    let movie = u64::from(output.movie_timescale());
    assert_eq!(
        track.edit_list.entries,
        vec![EditEntry {
            segment_duration: 47 * movie / 1000,
            media_time: 0,
            media_rate: EditEntry::NORMAL_RATE,
        }],
        "the edit ends at the end of the last packet"
    );
}

#[test]
fn reordered_video_takes_the_gap_in_presentation_order() {
    let blocks = [(0, None), (120, None), (40, None), (80, None)];
    let source = bare_file(BareKind::Video, &blocks);
    assert_eq!(
        sample_durations(&source),
        [Some(40), Some(40), Some(40), Some(40)]
    );
}

#[test]
fn a_track_with_one_timestamp_stays_refused() {
    let source = bare_file(BareKind::Opus, &[(0, None)]);
    assert!(
        matches!(copy(&source), Err(Error::MissingDuration { index: 0, .. })),
        "the copy is not refused by name"
    );
}

#[test]
fn a_subtitle_track_stays_refused() {
    let source = bare_file(BareKind::Text, &[(0, None), (900, None)]);
    assert!(
        matches!(copy(&source), Err(Error::MissingDuration { .. })),
        "the copy is not refused by name"
    );
}
