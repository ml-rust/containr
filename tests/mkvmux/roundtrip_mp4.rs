//! MP4 and MOV fixtures: demux, stream copy into Matroska, demux again. Each
//! mapped track keeps its packets and facts at exact timestamps. Each other
//! track is refused by name.

use containr::{Container, Error, Language, MetadataItem, Timebase, TrackKind};
use serde_json::Value;

use crate::common::{adler32, array, stream_track_id, text};
use crate::mkvmux::support::{Copied, Remux, collect, ok, open_mkv, open_mp4, remux_mp4};
use crate::mkvmux::walk::walk;

/// `value` ticks of the source, in output ticks.
fn ticks(value: i128, timebase: Timebase, scale: u64) -> i128 {
    let nanos = value * i128::from(timebase.num) * 1_000_000_000;
    assert_eq!(nanos % i128::from(timebase.den), 0, "{value} has whole ns");
    let nanos = nanos / i128::from(timebase.den);
    assert_eq!(nanos % i128::from(scale), 0, "{nanos} ns is a whole tick");
    nanos / i128::from(scale)
}

fn source_stream<'a>(remux: &'a Remux, copied: &Copied) -> &'a Value {
    array(&remux.fixture.probe, "streams")
        .iter()
        .find(|stream| stream_track_id(stream) == copied.source.id)
        .unwrap_or_else(|| panic!("no ffprobe stream for track {}", copied.source.id))
}

fn check_config(name: &str, remux: &Remux, copied: &Copied, private: Option<&Vec<u8>>) {
    let stream = source_stream(remux, copied);
    if text(stream, "codec_type") == "subtitle" {
        assert_eq!(private, None, "{name}: S_TEXT/UTF8 has no CodecPrivate");
        return;
    }
    match stream.get("extradata_hash") {
        Some(hash) => {
            let private =
                private.unwrap_or_else(|| panic!("{name}: the track has no CodecPrivate"));
            assert_eq!(
                Some(adler32(private).as_str()),
                hash.as_str(),
                "{name}: CodecPrivate"
            );
        }
        None => assert_eq!(private, None, "{name}: the source has no config"),
    }
}

fn check_track(name: &str, remux: &Remux, copied: &Copied) {
    let what = format!("{name} track {}", copied.source.id);
    let mut input = open_mp4(&remux.fixture.bytes);
    let mut output = open_mkv(&remux.bytes);
    let ours = output
        .track(copied.id.get())
        .unwrap_or_else(|| panic!("{what}: absent from the output"))
        .clone();
    let mkv = ours
        .matroska
        .as_ref()
        .unwrap_or_else(|| panic!("{what}: no Matroska facts"));
    let source = &copied.source;
    assert_eq!(ours.kind, source.kind, "{what}: kind");
    // QuickTime code 0x7FFF means unspecified, which Matroska spells `und`.
    let expected_language = match source.language {
        Language::Other(0x7FFF) => Language::Iso639(*b"und"),
        language => language,
    };
    assert_eq!(ours.language, expected_language, "{what}: language");
    let expected_codec = match source.codec().map(|c| c.to_string()).as_deref() {
        Some("avc1") => "V_MPEG4/ISO/AVC",
        Some("hvc1") => "V_MPEGH/ISO/HEVC",
        Some("av01") => "V_AV1",
        Some("vp09") => "V_VP9",
        Some("mp4a") => "A_AAC",
        Some("Opus") => "A_OPUS",
        Some("tx3g") => "S_TEXT/UTF8",
        other => panic!("{what}: unexpected sample entry {other:?}"),
    };
    assert_eq!(mkv.codec_id, expected_codec, "{what}: CodecID");
    check_config(&what, remux, copied, mkv.codec_private.as_ref());
    if source.kind == TrackKind::Video {
        let (video, facts) = (
            mkv.video.as_ref().expect("a Video element"),
            source.video().expect("visual facts"),
        );
        assert_eq!(video.pixel_width, u64::from(facts.width), "{what}: width");
        assert_eq!(
            video.pixel_height,
            u64::from(facts.height),
            "{what}: height"
        );
    }
    if let (Some(audio), Some(facts)) = (&mkv.audio, source.audio()) {
        assert_eq!(
            audio.channels,
            u64::from(facts.channels),
            "{what}: channels"
        );
        assert_eq!(
            audio.sampling_frequency,
            f64::from(facts.sample_rate),
            "{what}: sample rate"
        );
    }

    let all = collect(name, ok(name, input.track_packets(source.id)));
    let ours_packets = collect(name, ok(name, output.track_packets(ours.id)));
    let planned = copied.plan.timing();
    assert_eq!(planned.len(), all.len(), "{what}: timing count");
    assert_eq!(copied.packets, all, "{what}: planned packets");
    let mut mine = ours_packets.iter();
    for (n, (packet, timing)) in all.iter().zip(planned).enumerate() {
        let at = format!("{what} packet {n}");
        let Some(timing) = timing else {
            assert_eq!(packet.size, 2, "{at}: only an empty text sample is skipped");
            continue;
        };
        let our = mine
            .next()
            .unwrap_or_else(|| panic!("{at}: absent from the output"));
        let timebase = source.timebase;
        assert_eq!(
            i128::from(our.pts),
            ticks(i128::from(timing.pts), timebase, remux.scale),
            "{at}: pts"
        );
        let duration = packet.duration.expect("MP4 samples have durations");
        assert_eq!(timing.duration, Some(duration), "{at}: planned duration");
        assert_eq!(
            our.duration.map(i128::from),
            Some(ticks(i128::from(duration), timebase, remux.scale)),
            "{at}: duration"
        );
        assert_eq!(our.sync, packet.sync, "{at}: sync");
        assert_eq!(our.dts, None, "{at}: dts");
        let their_bytes = ok(name, input.read_packet(packet));
        let our_bytes = ok(name, output.read_packet(our));
        if expected_codec == "S_TEXT/UTF8" {
            assert_eq!(our_bytes, their_bytes[2..], "{at}: text");
        } else {
            assert_eq!(our_bytes, their_bytes, "{at}: bytes");
        }
    }
    assert!(mine.next().is_none(), "{what}: extra packets");
    let last = walk(&remux.bytes);
    let stored = last
        .last_block(u64::from(ours.id))
        .unwrap_or_else(|| panic!("{what}: no block"));
    assert_eq!(
        stored.duration,
        ours_packets.last().and_then(|packet| packet.duration),
        "{what}: the last block states its duration"
    );
    assert!(stored.duration.is_some(), "{what}: explicit last duration");
}

fn tag<'a>(items: &'a [MetadataItem], name: &str) -> Option<&'a str> {
    items
        .iter()
        .find(|item| item.name() == Some(name))
        .and_then(|item| item.text())
}

/// Remux `name`, with `refused` tracks refused by name.
fn check(name: &str, refused: usize) -> Option<Remux> {
    let remux = remux_mp4(name)?;
    assert_eq!(remux.refused.len(), refused, "{name}: refused tracks");
    for (id, err) in &remux.refused {
        assert!(
            matches!(
                err,
                Error::Unsupported {
                    container: Container::Matroska,
                    use_instead: Container::Mp4,
                    ..
                }
            ),
            "{name} track {id}: {err}"
        );
        assert!(err.to_string().contains("MP4/MOV"), "{name} {id}: {err}");
    }
    for copied in &remux.copied {
        check_track(name, &remux, copied);
    }
    Some(remux)
}

#[test]
fn round_trip_h264_negative_cts() {
    check("h264_negative_cts.mp4", 0);
}

#[test]
fn round_trip_hevc_opus_with_a_delay_and_an_aspect() {
    let Some(remux) = check("hevc_opus.mp4", 0) else {
        return;
    };
    assert_eq!(remux.scale, 500_000, "312 ticks at 48 kHz are 6.5 ms");
    let output = open_mkv(&remux.bytes);
    let opus = output
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Audio)
        .and_then(|track| track.matroska.as_ref())
        .expect("an Opus track");
    assert_eq!(opus.codec_delay, 6_500_000, "the edit list media time");
    assert_eq!(opus.seek_pre_roll, 80_000_000);
    assert_eq!(opus.default_duration, Some(20_000_000));
    let video = output
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .and_then(|track| track.matroska.as_ref())
        .expect("a video track");
    assert_eq!(video.default_duration, Some(40_000_000));
    let facts = video.video.as_ref().expect("a Video element");
    assert_eq!(facts.display_width, Some(128), "setsar=4/3 on 96 pixels");
}

#[test]
fn round_trip_vp9_opus() {
    check("vp9_opus.mp4", 0);
}

#[test]
fn round_trip_av1() {
    check("av1.mp4", 0);
}

#[test]
fn round_trip_aac_at_32_khz_maps_with_its_priming_as_a_delay() {
    let Some(remux) = check("aac_32k.mp4", 0) else {
        return;
    };
    let output = open_mkv(&remux.bytes);
    let aac = output.tracks()[0].matroska.as_ref().expect("an AAC track");
    assert_eq!(aac.codec_id, "A_AAC");
    assert_eq!(aac.codec_delay, 32_000_000, "1024 ticks at 32 kHz");
    assert_eq!(aac.default_duration, Some(32_000_000));
}

#[test]
fn round_trip_mov_text_drops_gaps_and_framing() {
    check("mov_text.mp4", 0);
}

#[test]
fn round_trip_aac_at_48_khz_is_refused_by_name_and_the_video_stays() {
    for name in ["h264_aac_moov_end.mp4", "h264_aac_faststart.mp4"] {
        let Some(remux) = check(name, 1) else {
            return;
        };
        let (_, err) = &remux.refused[0];
        assert!(err.to_string().contains("nanoseconds"), "{name}: {err}");
        assert_eq!(remux.scale, 1_000_000, "{name}: video at 25 fps");
    }
}

#[test]
fn round_trip_rotate90_keeps_the_rotation() {
    let Some(remux) = check("rotate90.mp4", 1) else {
        return;
    };
    let output = open_mkv(&remux.bytes);
    let video = output.tracks()[0].matroska.as_ref().expect("a track");
    let projection = video
        .video
        .as_ref()
        .and_then(|video| video.projection.as_ref())
        .expect("a Projection element");
    assert_eq!(projection.pose_roll, 90.0);
}

#[test]
fn round_trip_itsoffset_turns_the_empty_edit_into_a_start_time() {
    let Some(remux) = check("itsoffset.mp4", 1) else {
        return;
    };
    let output = open_mkv(&remux.bytes);
    let packets = collect("itsoffset", ok("itsoffset", output.track_packets(1)));
    let start = packets.iter().map(|packet| packet.pts).min();
    assert_eq!(start, Some(500), "the 0.5 s empty edit at 1 ms per tick");
}

#[test]
fn round_trip_ss_copy_refuses_frames_the_edit_cuts() {
    let Some(remux) = check("ss_copy.mp4", 2) else {
        return;
    };
    assert!(remux.copied.is_empty());
    let video_err = remux
        .refused
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, err)| err.to_string())
        .expect("the video track is refused");
    assert!(video_err.contains("never negative"), "{video_err}");
}

#[test]
fn round_trip_chapters_and_tags() {
    for name in ["chapters_mp4.mp4", "chapters_mov.mov"] {
        let Some(remux) = check(name, 1) else {
            return;
        };
        let output = open_mkv(&remux.bytes);
        assert_eq!(
            tag(output.metadata(), "TITLE"),
            Some("containr fixture"),
            "{name}"
        );
        assert!(tag(output.metadata(), "ENCODER").is_some(), "{name}");
        let titles: Vec<&str> = output
            .chapters()
            .iter()
            .map(|chapter| chapter.title.as_str())
            .collect();
        assert_eq!(titles, ["Opening", "Closing"], "{name}");
        let times: Vec<(u64, Option<u64>)> = output
            .chapters()
            .iter()
            .map(|chapter| (chapter.start, chapter.duration))
            .collect();
        assert_eq!(
            times,
            [(0, Some(800_000_000)), (800_000_000, Some(1_200_000_000))],
            "{name}"
        );
    }
}
