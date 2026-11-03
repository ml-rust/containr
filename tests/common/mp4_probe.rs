//! MP4/MOV files compared against ffprobe: track facts, packets, packet
//! bytes and edit lists. Used for FFmpeg-made fixtures and for files the
//! muxer writes.

use std::path::Path;
use std::process::Command;

use containr::mp4::{Demuxer, IDENTITY_MATRIX};
use containr::{Track, TrackKind};
use serde_json::{Value, json};

use super::{Fixture, adler32, array, int, ratio, stream_track_id, text};

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
        (u64::from(track.timebase.num), u64::from(track.timebase.den)),
        "{what}: timescale"
    );
    // libavformat rewrites the duration of a chapter track, and reports a
    // subtitle stream's duration_ts from the movie, not from `mdhd`.
    if !track.chapter_track && track.kind != TrackKind::Subtitle {
        assert_eq!(
            Some(int(stream, "duration_ts")),
            track.duration.map(|d| d as i64),
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
                None => assert_eq!(track.matrix, IDENTITY_MATRIX, "{what}: tkhd matrix"),
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
    let dts = |packet: &containr::Packet| {
        packet
            .dts
            .unwrap_or_else(|| panic!("{what}: an MP4 packet has a dts"))
    };
    let shift = ours
        .first()
        .map_or(0, |first| dts(first) - int(theirs[0], "dts"));
    assert!(shift >= 0, "{what}: dts shift {shift} is negative");
    for (n, (our, their)) in ours.iter().zip(&theirs).enumerate() {
        let at = format!("{what} packet {n}");
        assert_eq!(our.index, n as u64, "{at}: index");
        assert_eq!(our.pts, int(their, "pts"), "{at}: pts");
        assert_eq!(dts(our) - shift, int(their, "dts"), "{at}: dts");
        // A zero `stts` duration on the last sample stays zero here.
        // libavformat fills it up to the stream duration it reports, which
        // is the `mdhd` duration for audio and video and the movie duration
        // for subtitles.
        let duration = our
            .duration
            .unwrap_or_else(|| panic!("{at}: an MP4 packet has a duration"));
        let want = if duration == 0 && n + 1 == ours.len() {
            int(stream, "duration_ts") - dts(our)
        } else {
            duration as i64
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

/// Compare `fx` against its reference data.
pub fn check_against_probe(fx: &Fixture) {
    let name = &fx.name;
    let mut demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{err}"));
    let streams = array(&fx.probe, "streams");
    assert_eq!(streams.len(), demuxer.tracks().len(), "{name}: track count");
    for stream in streams {
        let id = stream_track_id(stream);
        let track = demuxer
            .track(id)
            .unwrap_or_else(|| panic!("{name}: ffprobe lists track {id}, the demuxer does not"))
            .clone();
        check_stream(fx, stream, &track);
        check_packets(fx, &mut demuxer, stream, &track);
    }
    check_edit_lists(fx, &demuxer);

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

/// Parse one `duration=D time=T rate=R` trace line into an edit, as
/// gen_mp4.py does.
fn parse_edit(line: &str) -> Option<Value> {
    let rest = &line[line.find("duration=")? + "duration=".len()..];
    let (duration, rest) = rest.split_once(" time=")?;
    let (time, rate) = rest.split_once(" rate=")?;
    let duration: i64 = duration.parse().ok()?;
    let time: i64 = time.parse().ok()?;
    let rate: f64 = rate.trim().parse().ok()?;
    Some(json!({
        "segment_duration": duration,
        "media_time": time,
        // The trace prints the rate as a float. The tests compare raw 16.16.
        "media_rate": (rate * 65536.0).round() as i64,
    }))
}

/// Run ffprobe on `path` and build the reference data gen_mp4.py builds:
/// `streams`, `packets`, `chapters`, `format` and `edit_lists`. Returns
/// `None`, with a notice, when ffprobe is not on PATH.
pub fn probe_file(path: &Path) -> Option<Value> {
    let run = |args: &[&str]| {
        let output = Command::new("ffprobe")
            .args(args)
            .arg(path)
            .output()
            .map_err(|err| eprintln!("skipping: ffprobe does not run: {err}"))
            .ok()?;
        assert!(
            output.status.success(),
            "ffprobe {args:?} {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        Some(output)
    };
    let shown = run(&[
        "-v",
        "error",
        "-ignore_editlist",
        "1",
        "-show_streams",
        "-show_packets",
        "-show_data_hash",
        "adler32",
        "-show_chapters",
        "-show_format",
        "-of",
        "json",
    ])?;
    let data: Value = serde_json::from_slice(&shown.stdout)
        .unwrap_or_else(|err| panic!("ffprobe wrote invalid JSON: {err}"));
    let trace = run(&["-v", "trace", "-show_format", "-of", "json"])?;
    let mut lists: Vec<Vec<Value>> = Vec::new();
    for line in String::from_utf8_lossy(&trace.stderr).lines() {
        if line.contains("type:'trak' parent:'moov'") {
            lists.push(Vec::new());
        } else if let (Some(list), Some(edit)) = (lists.last_mut(), parse_edit(line)) {
            list.push(edit);
        }
    }
    Some(json!({
        "streams": data["streams"],
        "packets": data["packets"],
        "chapters": data["chapters"],
        "format": data["format"],
        "edit_lists": lists,
    }))
}
