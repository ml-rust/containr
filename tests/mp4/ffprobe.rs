//! FFmpeg-made fixtures compared against ffprobe: track facts, packets,
//! packet bytes and edit lists.

use containr::mp4::Demuxer;
use containr::{Track, TrackKind};
use serde_json::Value;

use crate::common::{self, Fixture, adler32, array, int, ratio, stream_track_id, text};

const IDENTITY: [i32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The 9 values of an ffprobe `displaymatrix` dump.
fn display_matrix(dump: &str) -> Vec<i32> {
    dump.lines()
        .filter_map(|line| line.split_once(':'))
        .flat_map(|(_, values)| values.split_whitespace())
        .map(|value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("{value:?} in the display matrix is not an integer"))
        })
        .collect()
}

fn check_stream(fx: &Fixture, stream: &Value, track: &Track) {
    let what = format!("{} track {}", fx.name, track.id);
    assert_eq!(
        track.codec().map(|c| c.to_string()).as_deref(),
        Some(text(stream, "codec_tag_string")),
        "{what}: codec"
    );
    let (num, den) = ratio(text(stream, "time_base"), '/');
    assert_eq!(
        (num, den),
        (1, u64::from(track.timescale)),
        "{what}: timescale"
    );
    // libavformat rewrites the duration of a chapter track, and reports a
    // subtitle stream's duration_ts from the movie, not from `mdhd`.
    if !track.chapter_track && track.kind != TrackKind::Subtitle {
        assert_eq!(
            int(stream, "duration_ts"),
            track.duration as i64,
            "{what}: mdhd duration"
        );
    }
    if let Some(language) = track.language.iso639() {
        assert_eq!(
            stream["tags"]["language"].as_str(),
            Some(language),
            "{what}: language"
        );
    }
    match text(stream, "codec_type") {
        "video" => {
            let video = track
                .video()
                .unwrap_or_else(|| panic!("{what}: no video facts"));
            assert_eq!(
                int(stream, "width"),
                i64::from(video.width),
                "{what}: width"
            );
            assert_eq!(
                int(stream, "height"),
                i64::from(video.height),
                "{what}: height"
            );
            if let Some(pasp) = video.pixel_aspect {
                let g = gcd(u64::from(pasp.h_spacing), u64::from(pasp.v_spacing));
                let sar = format!(
                    "{}:{}",
                    u64::from(pasp.h_spacing) / g,
                    u64::from(pasp.v_spacing) / g
                );
                assert_eq!(text(stream, "sample_aspect_ratio"), sar, "{what}: pasp");
            }
            let matrix = array(stream, "side_data_list")
                .iter()
                .find(|side| side["side_data_type"] == "Display Matrix")
                .map(|side| display_matrix(text(side, "displaymatrix")));
            match matrix {
                Some(values) => assert_eq!(values, track.matrix.to_vec(), "{what}: tkhd matrix"),
                None => assert_eq!(track.matrix, IDENTITY, "{what}: tkhd matrix"),
            }
        }
        "audio" => {
            let audio = track
                .audio()
                .unwrap_or_else(|| panic!("{what}: no audio facts"));
            assert_eq!(
                int(stream, "sample_rate"),
                i64::from(audio.sample_rate),
                "{what}: sample rate"
            );
            assert_eq!(
                int(stream, "channels"),
                i64::from(audio.channels),
                "{what}: channels"
            );
        }
        _ => {}
    }
}

fn check_packets(
    fx: &Fixture,
    demuxer: &mut Demuxer<std::io::BufReader<std::fs::File>>,
    stream: &Value,
    track: &Track,
) {
    let what = format!("{} track {}", fx.name, track.id);
    let index = int(stream, "index");
    let theirs: Vec<&Value> = array(&fx.probe, "packets")
        .iter()
        .filter(|packet| int(packet, "stream_index") == index)
        .collect();
    let ours: Vec<_> = demuxer
        .track_packets(track.id)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{what}: {err}"));
    if theirs.is_empty() && track.chapter_track {
        // libavformat reads chapter tracks into chapters, not packets.
        return;
    }
    assert_eq!(ours.len(), theirs.len(), "{what}: packet count");
    assert_eq!(
        track.sample_count,
        ours.len() as u64,
        "{what}: sample count"
    );
    // libavformat shifts dts down by the largest negative composition offset
    // and keeps pts. The shift is the same for every packet of the track.
    let shift = ours
        .first()
        .map_or(0, |first| first.dts - int(theirs[0], "dts"));
    assert!(shift >= 0, "{what}: dts shift {shift} is negative");
    for (n, (our, their)) in ours.iter().zip(&theirs).enumerate() {
        let at = format!("{what} packet {n}");
        assert_eq!(our.index, n as u64, "{at}: index");
        assert_eq!(our.pts, int(their, "pts"), "{at}: pts");
        assert_eq!(our.dts - shift, int(their, "dts"), "{at}: dts");
        // A zero `stts` duration on the last sample stays zero here.
        // libavformat fills it up to the stream duration it reports, which
        // is the `mdhd` duration for audio and video and the movie duration
        // for subtitles.
        let want = if our.duration == 0 && n + 1 == ours.len() {
            int(stream, "duration_ts") - our.dts
        } else {
            i64::from(our.duration)
        };
        assert_eq!(want, int(their, "duration"), "{at}: duration");
        assert_eq!(i64::from(our.size), int(their, "size"), "{at}: size");
        assert_eq!(our.offset as i64, int(their, "pos"), "{at}: offset");
        assert_eq!(
            our.sync,
            text(their, "flags").starts_with('K'),
            "{at}: sync"
        );
        let bytes = demuxer
            .read_packet(our)
            .unwrap_or_else(|err| panic!("{at}: {err}"));
        assert_eq!(adler32(&bytes), text(their, "data_hash"), "{at}: bytes");
    }
}

fn check_edit_lists(fx: &Fixture, demuxer: &Demuxer<std::io::BufReader<std::fs::File>>) {
    let lists = array(&fx.probe, "edit_lists");
    assert_eq!(
        lists.len(),
        demuxer.tracks().len(),
        "{}: trak count",
        fx.name
    );
    for (track, list) in demuxer.tracks().iter().zip(lists) {
        let what = format!("{} track {}", fx.name, track.id);
        assert_eq!(
            track.edit_list.movie_timescale,
            demuxer.movie_timescale(),
            "{what}: movie timescale"
        );
        let theirs = list.as_array().map(Vec::as_slice).unwrap_or_default();
        assert_eq!(
            track.edit_list.entries.len(),
            theirs.len(),
            "{what}: edit count"
        );
        for (our, their) in track.edit_list.entries.iter().zip(theirs) {
            assert_eq!(
                our.segment_duration as i64,
                int(their, "segment_duration"),
                "{what}: segment_duration"
            );
            assert_eq!(
                our.media_time,
                int(their, "media_time"),
                "{what}: media_time"
            );
            assert_eq!(
                i64::from(our.media_rate),
                int(their, "media_rate"),
                "{what}: media_rate"
            );
        }
    }
}

/// Compare one fixture against its ffprobe reference. Skips when the
/// generator has not run.
pub fn check_fixture(name: &str) {
    let Some(fx) = common::fixture(name) else {
        return;
    };
    let mut demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{err}"));
    let streams = array(&fx.probe, "streams");
    assert_eq!(streams.len(), demuxer.tracks().len(), "{name}: track count");
    for stream in streams {
        let id = stream_track_id(stream);
        let track = demuxer
            .track(id)
            .unwrap_or_else(|| panic!("{name}: ffprobe lists track {id}, the demuxer does not"))
            .clone();
        check_stream(&fx, stream, &track);
        check_packets(&fx, &mut demuxer, stream, &track);
    }
    check_edit_lists(&fx, &demuxer);

    let mut previous = 0;
    let mut count = 0u64;
    for packet in demuxer.packets() {
        let packet = packet.unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(
            packet.offset >= previous,
            "{name}: packets() leaves file order at {packet:?}"
        );
        previous = packet.offset;
        count += 1;
    }
    let samples: u64 = demuxer
        .tracks()
        .iter()
        .map(|track| track.sample_count)
        .sum();
    assert_eq!(count, samples, "{name}: packets() covers every sample");
}

#[test]
fn ffprobe_h264_aac_moov_at_end() {
    check_fixture("h264_aac_moov_end.mp4");
}

#[test]
fn ffprobe_h264_aac_faststart() {
    check_fixture("h264_aac_faststart.mp4");
}

#[test]
fn ffprobe_h264_negative_cts() {
    check_fixture("h264_negative_cts.mp4");
}

#[test]
fn ffprobe_hevc_opus() {
    check_fixture("hevc_opus.mp4");
}

#[test]
fn ffprobe_mov_text() {
    check_fixture("mov_text.mp4");
}

#[test]
fn ffprobe_rotate90() {
    check_fixture("rotate90.mp4");
}

#[test]
fn ffprobe_itsoffset() {
    check_fixture("itsoffset.mp4");
}

#[test]
fn ffprobe_ss_copy() {
    check_fixture("ss_copy.mp4");
}

#[test]
fn ffprobe_chapters_mp4() {
    check_fixture("chapters_mp4.mp4");
}

#[test]
fn ffprobe_chapters_mov() {
    check_fixture("chapters_mov.mov");
}
