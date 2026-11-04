//! Movies built from codec configuration, with no fixture: the muxer alone,
//! read back with the demuxer.

use std::io::Cursor;

use containr::mp4::{
    AudioCodec, Brand, Demuxer, MuxOptions, Muxer, PacketFormat, PacketIn, SampleEntrySpec,
    TrackId, TrackSpec, VisualCodec,
};
use containr::{
    Chapter, ChapterSource, CodecConfig, EditEntry, EditList, FourCc, MetadataData, MetadataItem,
    MetadataKey, MetadataScope, PixelAspect, TrackKind,
};

use crate::common::top_level;

const AVCC: [u8; 4] = [1, 0x64, 0, 0x1E];
const ESDS: [u8; 6] = [0, 0, 0, 0, 3, 0];
const VIDEO_TIMESCALE: u32 = 90_000;
const AUDIO_TIMESCALE: u32 = 48_000;
const VIDEO_FRAME: i64 = 3600;
const AUDIO_FRAME: i64 = 1024;

fn video_spec() -> TrackSpec {
    let mut spec = TrackSpec::new(
        TrackKind::Video,
        VIDEO_TIMESCALE,
        SampleEntrySpec::Visual {
            codec: VisualCodec::Avc,
            config: AVCC.to_vec(),
            width: 96,
            height: 64,
        },
    );
    spec.pixel_aspect = Some(PixelAspect {
        h_spacing: 4,
        v_spacing: 3,
    });
    spec.display_width = 128 << 16;
    spec.display_height = 64 << 16;
    spec.matrix = [0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x4000_0000];
    spec
}

fn audio_spec() -> TrackSpec {
    let mut spec = TrackSpec::new(
        TrackKind::Audio,
        AUDIO_TIMESCALE,
        SampleEntrySpec::Audio {
            codec: AudioCodec::Aac,
            config: ESDS.to_vec(),
            channels: 2,
            sample_rate: 48_000,
        },
    );
    spec.language = containr::Language::Iso639(*b"eng");
    spec.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![EditEntry {
            segment_duration: 500,
            media_time: 1024,
            media_rate: EditEntry::NORMAL_RATE,
        }],
    };
    spec
}

fn title_item() -> MetadataItem {
    MetadataItem {
        scope: MetadataScope::Ilst,
        key: MetadataKey::FourCc(FourCc([0xA9, b'n', b'a', b'm'])),
        value: vec![
            0, 0, 0, 17, b'd', b'a', b't', b'a', 0, 0, 0, 1, 0, 0, 0, 0, b'T',
        ],
        data: vec![MetadataData {
            type_indicator: 1,
            locale: 0,
            payload: b"T".to_vec(),
        }],
    }
}

fn chapter(start: u64, title: &str) -> Chapter {
    Chapter {
        start,
        duration: None,
        timescale: 1000,
        title: title.to_owned(),
        source: ChapterSource::Chpl,
    }
}

/// `frames` video frames with B-frame style reordering and two seconds of
/// audio, written track by track in decode time order.
fn movie(options: MuxOptions, frames: i64) -> Vec<u8> {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let video = muxer
        .add_track(video_spec())
        .expect("the video spec is valid");
    let audio = muxer
        .add_track(audio_spec())
        .expect("the audio spec is valid");
    muxer
        .set_metadata(vec![title_item()])
        .expect("the item fits");
    muxer
        .set_chapters(vec![chapter(0, "One"), chapter(500, "Two")])
        .expect("the chapters fit");
    let video_data = vec![7u8; 400];
    let audio_data = vec![9u8; 100];
    let audio_frames = frames * VIDEO_FRAME * i64::from(AUDIO_TIMESCALE)
        / i64::from(VIDEO_TIMESCALE)
        / AUDIO_FRAME;
    let (mut v, mut a) = (0i64, 0i64);
    while v < frames || a < audio_frames {
        let video_time = v * VIDEO_FRAME * i64::from(AUDIO_TIMESCALE);
        let audio_time = a * AUDIO_FRAME * i64::from(VIDEO_TIMESCALE);
        if v < frames && (a >= audio_frames || video_time <= audio_time) {
            let dts = v * VIDEO_FRAME;
            // Decode order I P B: every third frame presents late.
            let pts = dts + if v % 3 == 1 { VIDEO_FRAME } else { 0 };
            let packet = PacketIn {
                data: &video_data,
                pts,
                dts,
                duration: Some(VIDEO_FRAME as u64),
                sync: v % 12 == 0,
            };
            muxer.write_packet(video, &packet).expect("a video packet");
            v += 1;
        } else {
            let dts = a * AUDIO_FRAME;
            let packet = PacketIn {
                data: &audio_data,
                pts: dts,
                dts,
                duration: Some(AUDIO_FRAME as u64),
                sync: true,
            };
            muxer.write_packet(audio, &packet).expect("an audio packet");
            a += 1;
        }
    }
    muxer.finish().expect("the movie finishes").into_inner()
}

#[test]
fn built_entries_read_back() {
    let bytes = movie(MuxOptions::default(), 50);
    let demuxer = Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    assert_eq!(demuxer.major_brand(), Some(FourCc(*b"isom")));
    assert_eq!(demuxer.tracks().len(), 3);
    let video = &demuxer.tracks()[0];
    assert_eq!(video.kind, TrackKind::Video);
    assert_eq!(video.codec(), Some(FourCc(*b"avc1")));
    assert_eq!(video.codec_config(), Some(&CodecConfig::Avc(AVCC.to_vec())));
    let facts = video.video().expect("video facts");
    assert_eq!((facts.width, facts.height), (96, 64));
    assert_eq!(
        facts.pixel_aspect,
        Some(PixelAspect {
            h_spacing: 4,
            v_spacing: 3
        })
    );
    assert_eq!(video.display_width, 128 << 16);
    assert_eq!(video.matrix[1], -0x10000);
    assert_eq!(video.duration, Some(50 * VIDEO_FRAME as u64));
    let audio = &demuxer.tracks()[1];
    assert_eq!(audio.codec(), Some(FourCc(*b"mp4a")));
    assert_eq!(
        audio.codec_config(),
        Some(&CodecConfig::Esds(ESDS.to_vec()))
    );
    assert_eq!(audio.language.iso639(), Some("eng"));
    assert_eq!(audio.edit_list.entries[0].media_time, 1024);
    assert_eq!(audio.edit_list.entries[0].segment_duration, 500);
    let facts = audio.audio().expect("audio facts");
    assert_eq!((facts.channels, facts.sample_rate), (2, 48_000));
    assert!(demuxer.tracks()[2].chapter_track);
    assert_eq!(demuxer.metadata(), [title_item()]);
    let titles: Vec<&str> = demuxer
        .chapters()
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert_eq!(titles, ["One", "Two", "One", "Two"]);
}

#[test]
fn chunks_interleave_by_decode_time_within_the_duration_bound() {
    let bytes = movie(MuxOptions::default(), 50);
    let demuxer = Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    let timed: Vec<(u32, i128)> = demuxer
        .packets()
        .map(|packet| packet.expect("a packet"))
        .filter(|packet| packet.track_id <= 2)
        .map(|packet| {
            let scale = if packet.track_id == 1 {
                VIDEO_TIMESCALE
            } else {
                AUDIO_TIMESCALE
            };
            let dts = i128::from(packet.dts.expect("an MP4 packet has a dts"));
            (packet.track_id, dts * 1_000_000 / i128::from(scale))
        })
        .collect();
    // Packets of one chunk share a track, and a chunk spans at most 500 ms
    // plus one frame. So no packet is written more than 1 s of decode time
    // after one that follows it.
    let mut latest = i128::MIN;
    for &(_, time) in &timed {
        assert!(latest - time <= 1_000_000, "{latest} then {time}");
        latest = latest.max(time);
    }
    let switches = timed.windows(2).filter(|w| w[0].0 != w[1].0).count();
    assert!(switches >= 3, "the tracks alternate: {switches} switches");
}

#[test]
fn a_tight_buffer_bound_still_writes_everything() {
    let options = MuxOptions {
        max_buffer: containr::mp4::MAX_CHUNK_BYTES,
        ..MuxOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let video = muxer
        .add_track(video_spec())
        .expect("the video spec is valid");
    // This track never sends a packet, so only the bound lets video out.
    let _idle: TrackId = muxer
        .add_track(audio_spec())
        .expect("the audio spec is valid");
    let data = vec![1u8; 300 * 1024];
    for n in 0..20 {
        let packet = PacketIn {
            data: &data,
            pts: n * VIDEO_FRAME,
            dts: n * VIDEO_FRAME,
            duration: Some(VIDEO_FRAME as u64),
            sync: true,
        };
        muxer.write_packet(video, &packet).expect("a video packet");
    }
    let bytes = muxer.finish().expect("the movie finishes").into_inner();
    let mut demuxer = Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    let packets: Vec<_> = demuxer.track_packets(1).expect("track 1").collect();
    assert_eq!(packets.len(), 20);
    for packet in packets {
        let packet = packet.expect("a packet");
        assert_eq!(demuxer.read_packet(&packet).expect("the bytes"), data);
    }
}

#[test]
fn quicktime_brand_writes_the_quicktime_handler_form() {
    let options = MuxOptions {
        brand: Brand::QuickTime,
        ..MuxOptions::default()
    };
    let bytes = movie(options, 10);
    assert!(bytes.windows(8).any(|w| w == b"mhlrvide"));
    assert!(bytes.windows(8).any(|w| w == b"dhlrurl "));
    let demuxer = Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    assert_eq!(demuxer.major_brand(), Some(FourCc(*b"qt  ")));
    assert_eq!(demuxer.tracks()[0].handler, Some(FourCc(*b"vide")));
}

#[test]
fn plain_text_cues_become_tx3g_samples_with_empty_gaps() {
    let mut muxer =
        Muxer::new(Cursor::new(Vec::new()), MuxOptions::default()).expect("the options are valid");
    let mut spec = TrackSpec::new(TrackKind::Subtitle, 1000, SampleEntrySpec::Tx3g);
    spec.packet_format = PacketFormat::PlainText;
    let id = muxer.add_track(spec).expect("the spec is valid");
    for (start, length, text) in [(200, 700, "first"), (900, 100, "next"), (1100, 700, "last")] {
        let packet = PacketIn {
            data: text.as_bytes(),
            pts: start,
            dts: start,
            duration: Some(length),
            sync: true,
        };
        muxer.write_packet(id, &packet).expect("a cue");
    }
    let overlap = PacketIn {
        data: b"late",
        pts: 1200,
        dts: 1200,
        duration: Some(1),
        sync: true,
    };
    assert!(matches!(
        muxer.write_packet(id, &overlap),
        Err(containr::Error::Unsupported { .. })
    ));
    let bytes = muxer.finish().expect("the movie finishes").into_inner();
    let mut demuxer = Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    assert_eq!(demuxer.tracks()[0].codec(), Some(FourCc(*b"tx3g")));
    let packets: Vec<_> = demuxer
        .track_packets(1)
        .expect("track 1")
        .map(|packet| packet.expect("a packet"))
        .collect();
    let mut seen = Vec::new();
    for packet in &packets {
        let bytes = demuxer.read_packet(packet).expect("the bytes");
        seen.push((
            packet.dts,
            packet.duration,
            String::from_utf8_lossy(&bytes[2..]).into_owned(),
        ));
    }
    let want: [(i64, u64, &str); 5] = [
        (0, 200, ""),
        (200, 700, "first"),
        (900, 100, "next"),
        (1000, 100, ""),
        (1100, 700, "last"),
    ];
    let want: Vec<_> = want
        .iter()
        .map(|&(dts, duration, text)| (Some(dts), Some(duration), text.to_owned()))
        .collect();
    assert_eq!(seen, want);
}

#[test]
fn large_mdat_header_only_appears_when_needed() {
    let bytes = movie(MuxOptions::default(), 5);
    let boxes = top_level(&bytes);
    let mdat = boxes
        .iter()
        .find(|(kind, _, _)| kind == b"mdat")
        .expect("the file has mdat");
    assert_eq!(
        u32::from_be_bytes(bytes[mdat.1..mdat.1 + 4].try_into().expect("4 bytes")) as usize,
        mdat.2
    );
}

#[test]
fn a_single_pass_edit_list_set_after_the_packets_reads_back() {
    let mut muxer =
        Muxer::new(Cursor::new(Vec::new()), MuxOptions::default()).expect("the options are valid");
    let video = muxer
        .add_track(video_spec())
        .expect("the video spec is valid");
    for n in 0..3 {
        let packet = PacketIn {
            data: &[1, 2, 3],
            pts: n * VIDEO_FRAME + VIDEO_FRAME,
            dts: n * VIDEO_FRAME,
            duration: Some(VIDEO_FRAME as u64),
            sync: n == 0,
        };
        muxer.write_packet(video, &packet).expect("a video packet");
    }
    let edits = EditList {
        movie_timescale: 1000,
        entries: vec![
            EditEntry {
                segment_duration: 500,
                media_time: EditEntry::EMPTY_MEDIA_TIME,
                media_rate: EditEntry::NORMAL_RATE,
            },
            EditEntry {
                segment_duration: 120,
                media_time: VIDEO_FRAME,
                media_rate: EditEntry::NORMAL_RATE,
            },
        ],
    };
    muxer
        .set_edit_list(video, &edits)
        .expect("the edit list is valid");
    let bytes = muxer.finish().expect("the movie finishes").into_inner();
    let input = Demuxer::open(Cursor::new(bytes)).expect("the movie reads back");
    assert_eq!(input.tracks()[0].edit_list, edits);
}

#[test]
fn set_edit_list_refuses_an_inexact_timescale_and_an_unknown_track() {
    let options = MuxOptions {
        movie_timescale: 7,
        ..MuxOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let video = muxer
        .add_track(video_spec())
        .expect("the video spec is valid");
    let inexact = EditList {
        movie_timescale: 1000,
        entries: vec![EditEntry {
            segment_duration: 1,
            media_time: 0,
            media_rate: EditEntry::NORMAL_RATE,
        }],
    };
    let error = muxer
        .set_edit_list(video, &inexact)
        .expect_err("1 ms has no exact value at 7 ticks per second");
    assert!(error.to_string().contains("movie_timescale"), "{error}");

    let mut other =
        Muxer::new(Cursor::new(Vec::new()), MuxOptions::default()).expect("the options are valid");
    assert!(other.set_edit_list(video, &EditList::default()).is_err());
}

#[test]
fn buffered_bytes_stay_within_the_bound_after_every_packet() {
    let options = MuxOptions {
        max_buffer: containr::mp4::MAX_CHUNK_BYTES,
        ..MuxOptions::default()
    };
    let bound = options.max_buffer;
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let video = muxer
        .add_track(video_spec())
        .expect("the video spec is valid");
    // This track never sends a packet, so only the bound lets video out.
    let _idle = muxer
        .add_track(audio_spec())
        .expect("the audio spec is valid");
    let data = vec![1u8; 300 * 1024];
    let mut peak = 0;
    for n in 0..20 {
        let packet = PacketIn {
            data: &data,
            pts: n * VIDEO_FRAME,
            dts: n * VIDEO_FRAME,
            duration: Some(VIDEO_FRAME as u64),
            sync: true,
        };
        muxer.write_packet(video, &packet).expect("a video packet");
        assert!(muxer.buffered_bytes() <= bound, "packet {n}");
        peak = peak.max(muxer.buffered_bytes());
    }
    assert!(peak > 0, "the muxer holds packets before it writes them");
}
