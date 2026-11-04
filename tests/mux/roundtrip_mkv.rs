//! Matroska and WebM fixtures: demux, stream copy into MP4, demux again.
//! Each mapped track keeps its packets and facts. Each other track is
//! refused by name.

use containr::mp4::IDENTITY_MATRIX;
use containr::{ChapterSource, CodecConfig, Container, EditEntry, Error, Timebase};

use crate::mux::support::{Copied, Remux, collect, ok, open_mkv, remux_mkv};

/// The tag names and texts of a metadata list.
fn tags(items: &[containr::MetadataItem]) -> Vec<(Option<String>, Option<String>)> {
    items
        .iter()
        .map(|item| {
            (
                item.name().map(str::to_owned),
                item.text().map(str::to_owned),
            )
        })
        .collect()
}

/// The AudioSpecificConfig inside an `esds` body.
fn esds_config(body: &[u8]) -> Vec<u8> {
    fn descriptor(bytes: &[u8], at: usize) -> (u8, usize, usize) {
        let tag = bytes[at];
        let mut pos = at + 1;
        let mut len = 0usize;
        loop {
            let byte = bytes[pos];
            pos += 1;
            len = (len << 7) | usize::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                return (tag, len, pos);
            }
        }
    }
    let (tag, _, es) = descriptor(body, 4);
    assert_eq!(tag, 0x03, "ES descriptor");
    let (tag, _, decoder) = descriptor(body, es + 3);
    assert_eq!(tag, 0x04, "decoder config descriptor");
    let (tag, len, config) = descriptor(body, decoder + 13);
    assert_eq!(tag, 0x05, "decoder specific info");
    body[config..config + len].to_vec()
}

/// The `dOps` body an `OpusHead` converts to: 19 bytes with mapping family 0.
fn dops_of(head: &[u8]) -> Vec<u8> {
    assert_eq!(&head[..8], b"OpusHead");
    assert_eq!(head[18], 0, "the fixture uses mapping family 0");
    vec![
        0, head[9], head[11], head[10], head[15], head[14], head[13], head[12], head[17], head[16],
        0,
    ]
}

fn check_config(what: &str, copied: &Copied, config: Option<&CodecConfig>) {
    let mkv = copied.source.matroska.as_ref().expect("a Matroska track");
    let private = mkv.codec_private.as_deref().unwrap_or_default();
    match (mkv.codec_id.as_str(), config) {
        ("V_MPEG4/ISO/AVC", Some(CodecConfig::Avc(bytes)))
        | ("V_MPEGH/ISO/HEVC", Some(CodecConfig::Hevc(bytes)))
        | ("V_AV1", Some(CodecConfig::Av1(bytes))) => {
            assert_eq!(bytes.as_slice(), private, "{what}: config");
        }
        ("A_AAC", Some(CodecConfig::Esds(bytes))) => {
            assert_eq!(esds_config(bytes), private, "{what}: AudioSpecificConfig");
        }
        ("A_OPUS", Some(CodecConfig::Opus(bytes))) => {
            assert_eq!(bytes, &dops_of(private), "{what}: dOps");
        }
        ("S_TEXT/UTF8", None) => {}
        (id, other) => panic!("{what}: codec {id} gives config {other:?}"),
    }
}

/// The matrix a fixture's rotation gives: only `rotate90.mkv` rotates.
fn expected_matrix(copied: &Copied) -> [i32; 9] {
    let roll = copied
        .source
        .matroska
        .as_ref()
        .and_then(|mkv| mkv.video.as_ref())
        .and_then(|video| video.projection.as_ref())
        .map(|projection| projection.pose_roll);
    if roll == Some(90.0) {
        [0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x4000_0000]
    } else {
        IDENTITY_MATRIX
    }
}

fn check_track(name: &str, remux: &Remux, copied: &Copied) {
    let what = format!("{name} track {}", copied.source.id);
    let mut input = open_mkv(&remux.fixture.bytes);
    let mut output = remux.output();
    let ours = output
        .track(copied.id.get())
        .unwrap_or_else(|| panic!("{what}: absent from the output"))
        .clone();
    let source = &copied.source;
    assert_eq!(ours.kind, source.kind, "{what}: kind");
    assert_eq!(ours.language, source.language, "{what}: language");
    // The output counts in the source tick, or in a whole multiple of it when
    // an AAC `CodecDelay` needs finer ticks.
    let source_rate = source.timebase.den / source.timebase.num;
    assert_eq!(
        ours.timebase,
        Timebase::from_timescale(copied.plan.spec.timescale),
        "{what}: timescale"
    );
    assert_eq!(
        copied.plan.spec.timescale % source_rate,
        0,
        "{what}: timescale is a multiple of the source tick rate"
    );
    let factor = i64::from(copied.plan.spec.timescale / source_rate);
    assert_eq!(ours.matrix, expected_matrix(copied), "{what}: matrix");
    assert_eq!(tags(&ours.metadata), tags(&source.metadata), "{what}: tags");
    check_config(&what, copied, ours.codec_config());
    let mkv = source.matroska.as_ref().expect("a Matroska track");
    if let (Some(video), Some(facts)) = (&mkv.video, ours.video()) {
        assert_eq!(u64::from(facts.width), video.pixel_width, "{what}: width");
        assert_eq!(
            u64::from(facts.height),
            video.pixel_height,
            "{what}: height"
        );
    }
    if let (Some(audio), Some(facts)) = (&mkv.audio, ours.audio()) {
        assert_eq!(
            u64::from(facts.channels),
            audio.channels,
            "{what}: channels"
        );
    }

    let all = collect(name, ok(name, output.track_packets(ours.id)));
    let total: u64 = all.iter().map(|p| p.duration.unwrap_or(0)).sum();
    assert_eq!(ours.duration, Some(total), "{what}: mdhd duration");
    let text = mkv.codec_id == "S_TEXT/UTF8";
    // An empty cue fills each gap in a `tx3g` track. It holds a length only.
    let mine: Vec<_> = all.iter().filter(|p| !text || p.size != 2).collect();
    let timing = copied.plan.timing();
    assert_eq!(mine.len(), copied.packets.len(), "{what}: packet count");
    assert_eq!(timing.len(), copied.packets.len(), "{what}: timing count");
    for (n, ((mine, theirs), planned)) in mine.iter().zip(&copied.packets).zip(timing).enumerate() {
        let at = format!("{what} packet {n}");
        assert_eq!(mine.pts, theirs.pts * factor, "{at}: pts");
        assert_eq!(mine.dts, Some(planned.dts), "{at}: dts");
        assert_eq!(mine.duration, planned.duration, "{at}: duration");
        assert_eq!(mine.sync, theirs.sync, "{at}: sync");
        let my_bytes = ok(name, output.read_packet(mine));
        let their_bytes = ok(name, input.read_packet(theirs));
        if text {
            let len = u16::from_be_bytes([my_bytes[0], my_bytes[1]]);
            assert_eq!(usize::from(len), their_bytes.len(), "{at}: text length");
            assert_eq!(&my_bytes[2..], their_bytes.as_slice(), "{at}: text");
        } else {
            assert_eq!(my_bytes, their_bytes, "{at}: bytes");
        }
    }
    let delay_ns = mkv.codec_delay;
    let rate = u128::from(copied.plan.spec.timescale);
    // An AAC edit skips the delay. An Opus edit starts at 0, since `dOps`
    // holds the delay.
    let start = if mkv.codec_id == "A_AAC" {
        (u128::from(delay_ns) * rate + 500_000_000) / 1_000_000_000
    } else {
        0
    };
    let delayed = copied.packets.iter().any(|p| p.pts < 0);
    let padded = copied.packets.iter().any(|p| p.discard_padding.is_some());
    if delayed || padded || start > 0 {
        let end = timing
            .iter()
            .map(|t| t.pts + t.duration.unwrap_or(0) as i64)
            .max()
            .expect("the track has packets");
        let want = (u128::from(end as u64) - start) * u128::from(remux.movie_timescale) / rate;
        assert_eq!(
            ours.edit_list.entries,
            vec![EditEntry {
                segment_duration: want as u64,
                media_time: start as i64,
                media_rate: EditEntry::NORMAL_RATE,
            }],
            "{what}: edit list"
        );
    } else {
        assert!(ours.edit_list.is_empty(), "{what}: edit list");
    }
    if mkv.audio.is_some() {
        check_first_sample(&what, copied, &ours);
    }
}

/// The presentation time of the first audio sample, in nanoseconds, matches
/// the source. A Matroska reader shows a block at its timestamp minus
/// `CodecDelay`. In MP4 the edit start and the `dOps` `pre_skip` both move the
/// first sample back. The integer divisions allow 2 ns.
fn check_first_sample(what: &str, copied: &Copied, ours: &containr::Track) {
    let source = &copied.source;
    let mkv = source.matroska.as_ref().expect("a Matroska track");
    let first = copied.packets.first().expect("the track has packets");
    let source_rate = i128::from(source.timebase.den / source.timebase.num);
    let want = i128::from(first.pts) * 1_000_000_000 / source_rate - i128::from(mkv.codec_delay);

    let rate = i128::from(ours.timebase.den);
    let edit = ours.edit_list.entries.first().map_or(0, |e| e.media_time);
    let planned = copied.plan.timing().first().expect("the track has timing");
    let mut got = (i128::from(planned.pts) - i128::from(edit)) * 1_000_000_000 / rate;
    if let Some(CodecConfig::Opus(dops)) = ours.codec_config() {
        let pre_skip = i128::from(u16::from_be_bytes([dops[2], dops[3]]));
        got -= pre_skip * 1_000_000_000 / 48_000;
    }
    assert!(
        (got - want).abs() <= 2,
        "{what}: the first sample presents at {got} ns, the source at {want} ns"
    );
}

fn check_chapters(name: &str, remux: &Remux) {
    let input = open_mkv(&remux.fixture.bytes);
    let output = remux.output();
    assert_eq!(
        tags(output.metadata()),
        tags(input.metadata()),
        "{name}: tags"
    );
    let theirs = input.chapters();
    let from_track: Vec<_> = output
        .chapters()
        .iter()
        .filter(|c| matches!(c.source, ChapterSource::Track(_)))
        .collect();
    let from_chpl: Vec<_> = output
        .chapters()
        .iter()
        .filter(|c| c.source == ChapterSource::Chpl)
        .collect();
    assert_eq!(from_track.len(), theirs.len(), "{name}: track chapters");
    assert_eq!(from_chpl.len(), theirs.len(), "{name}: chpl chapters");
    for (n, their) in theirs.iter().enumerate() {
        let at = format!("{name} chapter {n}");
        assert_eq!(from_track[n].title, their.title, "{at}: track title");
        // Nanosecond chapters count in milliseconds in the chapter track.
        assert_eq!(from_track[n].timescale, 1000, "{at}: timescale");
        assert_eq!(
            from_track[n].start * 1_000_000,
            their.start,
            "{at}: track start"
        );
        assert_eq!(from_chpl[n].title, their.title, "{at}: chpl title");
        assert_eq!(from_chpl[n].start, their.start / 100, "{at}: chpl start");
    }
}

/// Remux `name`, with `refused` tracks refused by name.
fn check(name: &str, refused: usize) {
    for faststart in [false, true] {
        let Some(remux) = remux_mkv(name, faststart) else {
            return;
        };
        assert_eq!(remux.refused.len(), refused, "{name}: refused tracks");
        for (id, err) in &remux.refused {
            assert!(
                matches!(
                    err,
                    Error::Unsupported {
                        use_instead: Container::Matroska,
                        ..
                    }
                ),
                "{name} track {id}: {err}"
            );
            assert!(
                err.to_string().contains("Matroska/WebM"),
                "{name} track {id}: {err}"
            );
        }
        for copied in &remux.copied {
            check_track(name, &remux, copied);
        }
        check_chapters(name, &remux);
    }
}

#[test]
fn round_trip_h264_aac() {
    check("h264_aac.mkv", 0);
}

#[test]
fn round_trip_hevc_opus() {
    check("hevc_opus.mkv", 0);
}

#[test]
fn round_trip_av1() {
    check("av1.webm", 0);
}

#[test]
fn round_trip_chapters() {
    check("chapters.mkv", 0);
}

#[test]
fn round_trip_rotate90() {
    check("rotate90.mkv", 0);
}

#[test]
fn round_trip_opus_trim() {
    check("opus_trim.webm", 0);
}

#[test]
fn round_trip_attachment_drops_only_the_attachment() {
    check("attachment.mkv", 0);
}

#[test]
fn round_trip_subtitles_refuses_ass_and_keeps_subrip() {
    check("subtitles.mkv", 1);
}

#[test]
fn round_trip_vp9_opus_refuses_vp9_and_keeps_opus() {
    check("vp9_opus.webm", 1);
}

#[test]
fn round_trip_live_refuses_vp9_and_keeps_opus() {
    check("live.webm", 1);
}
