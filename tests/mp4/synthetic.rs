//! Demux of the in-memory synthetic movie, whose tables are known exactly.

use std::io::Cursor;

use containr::mp4::Demuxer;
use containr::{
    ChapterSource, CodecConfig, Error, FourCc, MetadataKey, MetadataScope, PixelAspect, TrackKind,
};

use crate::common::synth::{self, AUDIO_ID, VIDEO_ID};

fn open(moov_first: bool) -> (Demuxer<Cursor<Vec<u8>>>, Vec<containr::Packet>) {
    let (bytes, expected) = synth::movie(moov_first);
    let demuxer = Demuxer::open(Cursor::new(bytes)).expect("the synthetic movie is well formed");
    (demuxer, expected)
}

#[test]
fn synthetic_tracks_carry_their_header_facts() {
    for moov_first in [true, false] {
        let (demuxer, _) = open(moov_first);
        assert_eq!(demuxer.major_brand(), Some(FourCc(*b"isom")));
        assert_eq!(
            demuxer.compatible_brands(),
            &[FourCc(*b"isom"), FourCc(*b"mp41")]
        );
        assert_eq!(demuxer.movie_timescale(), synth::MOVIE_TIMESCALE);
        assert_eq!(demuxer.tracks().len(), 2);

        let video = demuxer.track(VIDEO_ID).expect("track 1 exists");
        assert_eq!(video.kind, TrackKind::Video);
        assert_eq!(video.handler, FourCc(*b"vide"));
        assert_eq!(video.timescale, synth::VIDEO_TIMESCALE);
        assert_eq!(video.duration, 600);
        assert_eq!(video.language.iso639(), Some("und"));
        assert_eq!(video.codec(), Some(FourCc(*b"avc1")));
        assert_eq!(
            video.codec_config(),
            Some(&CodecConfig::Avc(synth::AVCC.to_vec()))
        );
        let info = video.video().expect("a video track has video facts");
        assert_eq!((info.width, info.height), (96, 64));
        assert_eq!(
            info.pixel_aspect,
            Some(PixelAspect {
                h_spacing: 4,
                v_spacing: 3
            })
        );
        assert_eq!(
            video.matrix,
            [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000]
        );
        assert_eq!(
            (video.display_width, video.display_height),
            (96 << 16, 64 << 16)
        );
        assert_eq!(video.sample_count, 6);
        let raw = &video.sample_entries[0].raw;
        assert_eq!(&raw[4..8], b"avc1");
        assert_eq!(
            u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize,
            raw.len()
        );

        let audio = demuxer.track(AUDIO_ID).expect("track 2 exists");
        assert_eq!(audio.kind, TrackKind::Audio);
        assert_eq!(
            audio.codec_config(),
            Some(&CodecConfig::Esds(synth::ESDS.to_vec()))
        );
        let info = audio.audio().expect("an audio track has audio facts");
        assert_eq!(
            (info.channels, info.sample_size, info.sample_rate),
            (2, 16, 48_000)
        );
        assert!(audio.edit_list.is_empty());
    }
}

#[test]
fn synthetic_edit_list_stays_raw() {
    let (demuxer, _) = open(true);
    let video = demuxer.track(VIDEO_ID).expect("track 1 exists");
    assert_eq!(video.edit_list.movie_timescale, synth::MOVIE_TIMESCALE);
    assert_eq!(video.edit_list.entries.len(), 1);
    let edit = video.edit_list.entries[0];
    assert_eq!(edit.segment_duration, u64::from(synth::EDIT.0));
    assert_eq!(edit.media_time, i64::from(synth::EDIT.1));
    assert_eq!(edit.media_rate, containr::EditEntry::NORMAL_RATE);
    let first = demuxer
        .track_packets(VIDEO_ID)
        .expect("track 1 exists")
        .next()
        .expect("track 1 has samples")
        .expect("the sample decodes");
    assert_eq!(first.dts, 0, "the demuxer must not apply the edit list");
}

#[test]
fn synthetic_packets_match_the_tables_in_both_layouts() {
    for moov_first in [true, false] {
        let (mut demuxer, expected) = open(moov_first);
        let packets: Vec<_> = demuxer
            .packets()
            .collect::<Result<_, _>>()
            .expect("every packet decodes");
        assert_eq!(packets, expected, "file order, moov_first = {moov_first}");
        for packet in &packets {
            let bytes = demuxer
                .read_packet(packet)
                .expect("the packet is in the input");
            assert_eq!(bytes, synth::packet_bytes(packet));
        }
        for id in [VIDEO_ID, AUDIO_ID] {
            let track: Vec<_> = demuxer
                .track_packets(id)
                .expect("the track exists")
                .collect::<Result<_, _>>()
                .expect("every packet decodes");
            let want: Vec<_> = expected
                .iter()
                .copied()
                .filter(|p| p.track_id == id)
                .collect();
            assert_eq!(track, want);
        }
    }
}

#[test]
fn synthetic_signed_composition_offsets_give_pts_before_dts() {
    let (demuxer, _) = open(true);
    let video: Vec<_> = demuxer
        .track_packets(VIDEO_ID)
        .expect("track 1 exists")
        .collect::<Result<_, _>>()
        .expect("every packet decodes");
    let pts: Vec<i64> = video.iter().map(|p| p.pts).collect();
    assert_eq!(pts, vec![0, 300, 100, 300, 600, 400]);
    let sync: Vec<bool> = video.iter().map(|p| p.sync).collect();
    assert_eq!(sync, vec![true, false, false, false, true, false]);
}

#[test]
fn synthetic_metadata_and_chpl_chapters_come_through() {
    let (demuxer, _) = open(true);
    let title = demuxer
        .metadata()
        .iter()
        .find(|item| item.key == MetadataKey::FourCc(FourCc([0xA9, b'n', b'a', b'm'])))
        .expect("the ilst title is present");
    assert_eq!(title.scope, MetadataScope::Ilst);
    assert_eq!(title.text(), Some("synth title"));
    let chapters = demuxer.chapters();
    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].title, "Start");
    assert_eq!(chapters[0].source, ChapterSource::Chpl);
    assert_eq!(chapters[0].timescale, 10_000_000);
}

#[test]
fn synthetic_unknown_track_id_names_the_available_ids() {
    let (demuxer, _) = open(true);
    match demuxer.track_packets(99) {
        Err(Error::TrackNotFound {
            track_id,
            available,
        }) => {
            assert_eq!(track_id, 99);
            assert_eq!(available, vec![VIDEO_ID, AUDIO_ID]);
        }
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn synthetic_fragment_is_refused_by_name() {
    let (mut bytes, _) = synth::movie(true);
    bytes.extend([0, 0, 0, 8, b'm', b'o', b'o', b'f']);
    match Demuxer::open(Cursor::new(bytes)) {
        Err(err @ Error::Fragmented { .. }) => {
            let message = err.to_string();
            assert!(message.contains("moof"), "{message}");
            assert!(message.contains("ffmpeg"), "{message}");
        }
        other => panic!("expected Fragmented, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn synthetic_missing_moov_is_named() {
    let (bytes, _) = synth::movie(false);
    let moov = crate::common::moov_range(&bytes);
    let truncated = bytes[..moov.start].to_vec();
    match Demuxer::open(Cursor::new(truncated)) {
        Err(Error::MissingBox { path }) => assert_eq!(path, "moov"),
        other => panic!("expected MissingBox, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn synthetic_packet_past_a_truncated_mdat_is_out_of_bounds() {
    let (bytes, expected) = synth::movie(true);
    let last = *expected.last().expect("the movie has packets");
    let cut = (last.offset + 2) as usize;
    let mut demuxer =
        Demuxer::open(Cursor::new(bytes[..cut].to_vec())).expect("a truncated mdat still opens");
    assert!(matches!(
        demuxer.read_packet(&last),
        Err(Error::PacketOutOfBounds { .. })
    ));
    assert!(demuxer.read_packet(&expected[0]).is_ok());
}
