//! A Matroska source whose last block stores no duration, built in memory, so
//! these tests hold with any FFmpeg or none.

use std::io::Cursor;

use containr::mkv::{MkvOptions, Muxer, StreamCopy};
use containr::{Error, Packet, Result};

use crate::common::mkv_synth::{BARE_TRACK, BareKind, bare_file};
use crate::mkvmux::support::open_mkv;
use crate::mkvmux::walk::walk;

fn packets_of(bytes: &[u8]) -> Vec<Packet> {
    open_mkv(bytes)
        .track_packets(BARE_TRACK)
        .and_then(|packets| packets.collect::<Result<_>>())
        .unwrap_or_else(|err| panic!("{err}"))
}

/// Plan, write and finish a copy of the one track.
fn copy(source: &[u8]) -> Result<Vec<u8>> {
    let mut input = open_mkv(source);
    let track = input.tracks()[0].clone();
    let packets = packets_of(source);
    let plan = StreamCopy::plan(&track, &packets)?;
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default())?;
    let id = muxer.add_track(plan.spec.clone())?;
    for packet in &packets {
        let data = input.read_packet(packet)?;
        if let Some(packet_in) = plan.packet(packet, &data)? {
            muxer.write_packet(id, &packet_in)?;
        }
    }
    Ok(muxer.finish()?.into_inner())
}

/// The durations a copy of `source` reads back with, after checking that the
/// source stores none and that the last block states its own.
fn copied_durations(source: &[u8]) -> Vec<Option<u64>> {
    let theirs = packets_of(source);
    assert!(
        theirs.iter().all(|packet| packet.duration.is_none()),
        "the source stores no duration"
    );
    let copied = copy(source).unwrap_or_else(|err| panic!("{err}"));
    let ours = packets_of(&copied);
    assert_eq!(ours.len(), theirs.len(), "packet count");
    for (our, their) in ours.iter().zip(&theirs) {
        assert_eq!(our.pts, their.pts, "pts");
        assert_eq!(our.discard_padding, their.discard_padding, "padding");
    }
    let durations: Vec<_> = ours.iter().map(|packet| packet.duration).collect();
    let stored = walk(&copied)
        .last_block(u64::from(BARE_TRACK))
        .expect("a block")
        .duration;
    assert_eq!(stored, durations.last().copied().flatten(), "BlockDuration");
    durations
}

fn assert_refused(source: &[u8], last_index: u64) {
    assert!(
        matches!(
            copy(source),
            Err(Error::MissingDuration { track_id, index })
                if track_id == BARE_TRACK && index == last_index
        ),
        "the copy is not refused by name"
    );
}

#[test]
fn a_last_audio_packet_takes_the_gap_before_it() {
    let source = bare_file(BareKind::Opus, &[(0, None), (20, None), (40, None)]);
    assert_eq!(copied_durations(&source), [None, None, Some(20)]);
}

#[test]
fn the_last_gap_counts_and_not_the_first() {
    let source = bare_file(BareKind::Opus, &[(0, None), (21, None), (43, None)]);
    assert_eq!(copied_durations(&source), [None, None, Some(22)]);
}

#[test]
fn discard_padding_comes_off_the_gap() {
    for (padding, want) in [(13_500_000, 7), (500_000, 20), (19_900_000, 1)] {
        let source = bare_file(
            BareKind::Opus,
            &[(0, None), (20, None), (40, Some(padding))],
        );
        assert_eq!(
            copied_durations(&source),
            [None, None, Some(want)],
            "padding {padding}"
        );
    }
}

#[test]
fn reordered_video_takes_the_gap_in_presentation_order() {
    let blocks = [(0, None), (120, None), (40, None), (80, None)];
    let source = bare_file(BareKind::Video, &blocks);
    assert_eq!(copied_durations(&source), [None, None, None, Some(40)]);
}

#[test]
fn a_track_with_one_timestamp_stays_refused() {
    assert_refused(&bare_file(BareKind::Opus, &[(0, None)]), 0);
    assert_refused(&bare_file(BareKind::Opus, &[(5, None), (5, None)]), 1);
}

#[test]
fn padding_that_covers_the_gap_stays_refused() {
    let source = bare_file(BareKind::Opus, &[(0, None), (20, Some(20_000_000))]);
    assert_refused(&source, 1);
}

#[test]
fn a_subtitle_track_stays_refused() {
    assert_refused(&bare_file(BareKind::Text, &[(0, None), (900, None)]), 1);
}
