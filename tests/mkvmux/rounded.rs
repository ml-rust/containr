//! `StreamCopy::plan_rounded`: each rounding edge by hand, then FFmpeg-made
//! 29.97 fps H.264 and 48 kHz AAC copied to Matroska and read back.
//!
//! Every check of an exact time here is integer arithmetic on the source's own
//! values, apart from the rounding code.

use containr::mkv::{StreamCopy, TrackSpec};
use containr::{CodecConfig, EditList, Error, Packet, Timebase, Track, TrackKind};

use crate::mkvmux::mapping::{assert_names_mp4, edit, entry, packet, track};
use crate::mkvmux::support::{
    Copied, collect, ok, open_mkv, open_mp4, remux_mp4, remux_mp4_rounded, within_half_a_tick,
};

const MS: u64 = 1_000_000;

fn video(timescale: u32) -> Track {
    track(TrackKind::Video, timescale, vec![entry(b"vp09", None)])
}

fn audio(timescale: u32) -> Track {
    let dops = CodecConfig::Opus(vec![0; 11]);
    track(
        TrackKind::Audio,
        timescale,
        vec![entry(b"Opus", Some(dops))],
    )
}

fn rounded(track: &Track, packets: &[Packet], scale: u64) -> StreamCopy {
    StreamCopy::plan_rounded(track, packets, scale).unwrap_or_else(|err| panic!("{err}"))
}

fn pts_and_durations(plan: &StreamCopy) -> (Vec<i64>, Vec<u64>) {
    let blocks = plan.timing().iter().flatten();
    blocks
        .map(|block| (block.pts, block.duration.expect("a rounded duration")))
        .unzip()
}

/// A run of samples: `(pts, duration)`, in decode order.
fn samples(list: &[(i64, u64)]) -> Vec<Packet> {
    list.iter()
        .enumerate()
        .map(|(n, &(pts, duration))| packet(n as u64, pts, duration, 1))
        .collect()
}

#[test]
fn the_default_plan_refuses_29_97_fps_video_in_a_30000_timescale() {
    let packets = samples(&[(0, 1001), (1001, 1001), (2002, 1001)]);
    let err = StreamCopy::plan(&video(30_000), &packets).expect_err("1001/30000 s");
    assert_names_mp4(&err, "nanoseconds");
    let text = err.to_string();
    assert!(text.contains("1/30000"), "{text}");
    assert!(text.contains("StreamCopy::plan_rounded"), "{text}");
}

#[test]
fn the_default_plan_refuses_a_29_97_fps_fixture_and_its_48_khz_aac() {
    let Some(remux) = remux_mp4("ntsc_h264_aac.mp4") else {
        return;
    };
    assert!(remux.copied.is_empty(), "both tracks are inexact");
    assert_eq!(remux.refused.len(), 2);
    for (id, err) in &remux.refused {
        assert_names_mp4(err, "nanoseconds");
        assert!(err.to_string().contains("plan_rounded"), "{id}: {err}");
    }
}

#[test]
fn each_timestamp_rounds_on_its_own_and_durations_come_from_neighbours() {
    let packets = samples(&[(0, 1001), (1001, 1001), (2002, 1001)]);
    let plan = rounded(&video(30_000), &packets, MS);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 33, 67], "0, 33.37 and 66.73 ms");
    assert_eq!(durations, [33, 34, 33], "the end rounds from 100.1 ms");
    assert_eq!(plan.spec.timebase, Timebase::from_nanoseconds(1_000_000));
    assert_eq!(plan.spec.matroska.default_duration, Some(33_366_667));
    let rounding = plan.rounding().expect("a rounded plan");
    assert_eq!(rounding.scale, MS);
    assert_eq!(rounding.max_error_ns, 366_667, "33.3667 ms against 33 ms");
}

#[test]
fn ties_round_to_the_even_tick() {
    // One tick of 2000 per second is half a millisecond.
    let packets = samples(&[(1, 2), (3, 4), (7, 3)]);
    let plan = rounded(&video(2000), &packets, MS);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 2, 4], "0.5, 1.5 and 3.5 ms");
    assert_eq!(durations, [2, 2, 1], "the end is 5 ms");
    assert_eq!(plan.rounding().map(|r| r.max_error_ns), Some(500_000));
}

#[test]
fn a_tie_at_a_scale_of_one_nanosecond_goes_to_even() {
    // 1 tick at 2 000 000 000 per second is 0.5 ns, and 3 ticks are 1.5 ns.
    let packets = samples(&[(1, 2), (3, 2)]);
    let plan = rounded(&video(2_000_000_000), &packets, 1);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 2]);
    assert_eq!(durations, [2, 0], "the end of 2.5 ns also rounds to 2");
}

#[test]
fn a_negative_time_is_refused_even_when_it_rounds_to_zero() {
    let mut source = video(30_000);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(1000, 1)],
    };
    // -1/30000 s is 0 ticks at 1 ms, and the frame is still one the edit cuts.
    let err = StreamCopy::plan_rounded(&source, &samples(&[(0, 1001), (1001, 1001)]), MS)
        .expect_err("a frame before the edit");
    assert_names_mp4(&err, "never negative");
}

#[test]
fn a_negative_source_pts_rounds_up_through_the_edit() {
    let mut source = video(30_000);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(1000, -1001)],
    };
    let plan = rounded(&source, &samples(&[(-1001, 1001), (0, 1001)]), MS);
    let (pts, _) = pts_and_durations(&plan);
    // The shift adds 1001 ticks, so the frames sit at 0 and 33.37 ms.
    assert_eq!(pts, [0, 33]);
}

#[test]
fn a_collapse_is_refused_and_names_a_scale_that_holds_the_track() {
    let source = audio(48_000);
    let packets = samples(&[(0, 1024), (1024, 1024), (2048, 1024)]);
    let err = StreamCopy::plan_rounded(&source, &packets, 100 * MS).expect_err("21 ms at 100 ms");
    assert_names_mp4(&err, "one tick");
    let text = err.to_string();
    assert!(text.contains("packets 0 and 1 of track 3"), "{text}");
    assert!(text.contains("scale of 21333333"), "{text}");
    let plan = rounded(&source, &packets, 21_333_333);
    let (pts, _) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 1, 2], "the named scale separates them");
}

#[test]
fn packets_at_one_source_pts_are_refused_as_the_source_fact_they_are() {
    let packets = samples(&[(0, 1001), (0, 1001)]);
    let err = StreamCopy::plan_rounded(&video(30_000), &packets, 1).expect_err("one pts");
    assert_names_mp4(&err, "one source `pts`");
}

#[test]
fn decode_order_and_sync_flags_stay_and_durations_follow_presentation_order() {
    let mut packets = samples(&[(0, 1001), (3003, 1001), (1001, 1001), (2002, 1001)]);
    packets[1].sync = false;
    packets[2].sync = false;
    let plan = rounded(&video(30_000), &packets, MS);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 100, 33, 67], "decode order");
    assert_eq!(durations, [33, 33, 34, 33], "the last end is 133.47 ms");
    for packet in &packets {
        let block = plan
            .packet(packet, &[9])
            .expect("a planned packet")
            .expect("a block");
        assert_eq!(block.sync, packet.sync, "packet {}", packet.index);
    }
}

#[test]
fn no_error_accumulates_across_a_long_track() {
    let packets: Vec<Packet> = (0..200_000)
        .map(|n| packet(n, n as i64 * 1024, 1024, 1))
        .collect();
    let plan = rounded(&audio(48_000), &packets, MS);
    let (pts, durations) = pts_and_durations(&plan);
    for (n, &tick) in pts.iter().enumerate() {
        // One packet lasts 64/3 ms. The offset counts in thirds of a tick.
        let off = (i128::from(tick) * 3 - n as i128 * 64).abs();
        assert!(off * 2 <= 3, "packet {n}: {tick} is off by {off}/3 ms");
    }
    let total: u64 = durations.iter().sum();
    assert_eq!(total, 4_266_667, "the rounded end of 200 000 frames");
}

#[test]
fn an_audio_edit_rounds_codec_delay_and_discard_padding_to_nanoseconds() {
    let mut source = audio(48_000);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(40, 1024)],
    };
    let packets = samples(&[(0, 1024), (1024, 1024), (2048, 1024)]);
    let err = StreamCopy::plan(&source, &packets).expect_err("the exact plan refuses");
    assert_names_mp4(&err, "plan_rounded");
    let plan = rounded(&source, &packets, MS);
    assert_eq!(plan.spec.matroska.codec_delay, 21_333_333, "1024 ticks");
    let last = plan.timing()[2].expect("a block");
    assert_eq!(last.discard_padding, Some(2_666_667), "128 ticks");
    assert_eq!(plan.timing()[0].expect("a block").discard_padding, None);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 21, 43], "the audio edit moves no block");
    assert_eq!(durations, [21, 22, 21], "the end is 64 ms");
}

#[test]
fn an_end_trim_past_the_last_packet_is_refused() {
    let mut source = audio(48_000);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(1, 0)],
    };
    let packets = samples(&[(0, 1024), (1024, 1024)]);
    let err = StreamCopy::plan_rounded(&source, &packets, MS).expect_err("more than one block");
    assert_names_mp4(&err, "last packet");
}

#[test]
fn a_video_start_delay_with_no_whole_track_tick_rounds() {
    let mut source = video(30_000);
    source.edit_list = EditList {
        movie_timescale: 7,
        entries: vec![edit(1, -1), edit(1, 0)],
    };
    let packets = samples(&[(0, 1001)]);
    let err = StreamCopy::plan(&source, &packets).expect_err("1/7 s in 1/30000 s ticks");
    assert_names_mp4(&err, "plan_rounded");
    let plan = rounded(&source, &packets, MS);
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [143], "1/7 s is 142.857 ms");
    assert_eq!(
        durations,
        [33],
        "the end is 176.22 ms, from the exact value"
    );
}

#[test]
fn an_empty_text_sample_ends_the_cue_before_it_and_writes_nothing() {
    let source = track(TrackKind::Subtitle, 30_000, vec![entry(b"tx3g", None)]);
    let mut packets = samples(&[(0, 1001), (1001, 2002), (3003, 1001)]);
    packets[1].size = 2;
    let plan = rounded(&source, &packets, MS);
    assert_eq!(plan.timing()[1], None, "the gap writes no block");
    let cue = plan.timing()[0].expect("a cue");
    assert_eq!((cue.pts, cue.duration), (0, Some(33)), "33.37 ms");
    let last = plan.timing()[2].expect("a cue");
    assert_eq!((last.pts, last.duration), (100, Some(33)), "133.47 ms");
    assert_eq!(plan.spec.matroska.default_duration, None);
}

#[test]
fn scale_is_checked() {
    let packets = samples(&[(0, 1001)]);
    for scale in [0, 1 << 32] {
        let err = StreamCopy::plan_rounded(&video(30_000), &packets, scale).expect_err("a scale");
        let text = err.to_string();
        assert!(text.contains(&format!("scale {scale}")), "{text}");
        assert!(matches!(err, Error::InvalidMux { .. }), "{text}");
    }
}

#[test]
fn a_matroska_track_is_refused_and_names_plan() {
    let mut source = video(1000);
    let spec = TrackSpec::new(TrackKind::Video, "V_VP9", Timebase::from_nanoseconds(1000));
    source.matroska = Some(spec.matroska);
    let err = StreamCopy::plan_rounded(&source, &samples(&[(0, 1)]), MS).expect_err("Matroska");
    assert!(err.to_string().contains("`StreamCopy::plan`"), "{err}");
}

#[test]
fn packets_out_of_order_name_the_call() {
    let packets = [packet(1, 0, 1, 1)];
    let err = StreamCopy::plan_rounded(&video(1000), &packets, MS).expect_err("index 1 first");
    let text = err.to_string();
    assert!(text.contains("`StreamCopy::plan_rounded`"), "{text}");
    assert!(text.contains("decode order"), "{text}");
}

#[test]
fn exact_values_round_with_no_error_and_the_exact_plan_reports_none() {
    let source = video(12_800);
    let packets = samples(&[(0, 512), (512, 512)]);
    let exact = StreamCopy::plan(&source, &packets).expect("40 ms is exact");
    assert_eq!(exact.rounding(), None);
    let plan = rounded(&source, &packets, MS);
    assert_eq!(plan.rounding().map(|r| r.max_error_ns), Some(0));
    let (pts, durations) = pts_and_durations(&plan);
    assert_eq!(pts, [0, 40]);
    assert_eq!(durations, [40, 40]);
    assert_eq!(plan.spec.matroska.default_duration, Some(40_000_000));
}

#[test]
fn the_output_scale_divides_rounded_and_exact_plans() {
    let packets = samples(&[(0, 1001), (1001, 1001)]);
    let at_ms = rounded(&video(30_000), &packets, MS);
    let at_third = rounded(&video(30_000), &packets, 333_333);
    let opus = StreamCopy::plan(&audio(48_000), &samples(&[(0, 312)])).expect("6.5 ms is exact");
    assert_eq!(
        StreamCopy::timestamp_scale(std::slice::from_ref(&at_ms)),
        MS
    );
    assert_eq!(
        StreamCopy::timestamp_scale(std::slice::from_ref(&at_third)),
        333_333,
        "a rounded plan keeps the scale it names"
    );
    assert_eq!(
        StreamCopy::timestamp_scale(&[at_ms, opus]),
        500_000,
        "the scale also holds the exact plan"
    );
}

/// The exact start of `pts`, in units of 1 / `per_second` seconds.
fn exact_start(source: &Track, pts: i64) -> (i128, i128) {
    let list = &source.edit_list;
    let movie = i128::from(list.movie_timescale.max(1));
    let timescale = i128::from(source.timebase.den);
    let mut shift = 0;
    for entry in &list.entries {
        if entry.is_empty() {
            shift += i128::from(entry.segment_duration) * timescale;
        } else if source.kind != TrackKind::Audio {
            shift -= i128::from(entry.media_time) * movie;
        }
    }
    (i128::from(pts) * movie + shift, timescale * movie)
}

fn check_track(name: &str, bytes: &[u8], source_bytes: &[u8], copied: &Copied, scale: u64) {
    let what = format!("{name} track {} at {scale} ns", copied.source.id);
    let mut input = open_mp4(source_bytes);
    let mut output = open_mkv(bytes);
    let ours = collect(name, ok(name, output.track_packets(copied.id.get())));
    let theirs = collect(name, ok(name, input.track_packets(copied.source.id)));
    assert_eq!(ours.len(), theirs.len(), "{what}: packet count");
    for (n, (our, their)) in ours.iter().zip(&theirs).enumerate() {
        let at = format!("{what} packet {n}");
        assert_eq!(our.sync, their.sync, "{at}: sync");
        assert_eq!(
            ok(name, output.read_packet(our)),
            ok(name, input.read_packet(their)),
            "{at}: bytes, in decode order"
        );
        let (value, per_second) = exact_start(&copied.source, their.pts);
        assert!(
            within_half_a_tick(i128::from(our.pts), scale, value, per_second),
            "{at}: pts {} against the exact {value} over {per_second}",
            our.pts
        );
    }
    // Durations in presentation order sum to the rounded end: no drift.
    let last_their = theirs
        .iter()
        .max_by_key(|packet| packet.pts)
        .expect("a track with packets");
    let first = ours
        .iter()
        .map(|packet| packet.pts)
        .min()
        .expect("a packet");
    let total: u64 = ours
        .iter()
        .map(|packet| packet.duration.expect("a duration"))
        .sum();
    let end = i128::from(first) + i128::from(total);
    let (value, per_second) = exact_start(&copied.source, last_their.pts);
    let span = i128::from(last_their.duration.expect("MP4 samples have durations"));
    let movie = per_second / i128::from(copied.source.timebase.den);
    assert!(
        within_half_a_tick(end, scale, value + span * movie, per_second),
        "{what}: the end {end} drifts from the exact end"
    );
}

#[test]
fn a_29_97_fps_fixture_with_48_khz_aac_copies_at_each_scale() {
    for scale in [MS, 1000] {
        let name = "ntsc_h264_aac.mp4";
        let Some(remux) = remux_mp4_rounded(name, scale) else {
            return;
        };
        assert!(remux.refused.is_empty(), "{:?}", remux.refused);
        assert_eq!(remux.copied.len(), 2);
        assert_eq!(remux.scale, scale);
        let output = open_mkv(&remux.bytes);
        assert_eq!(output.info().timestamp_scale, scale);
        for copied in &remux.copied {
            check_track(name, &remux.bytes, &remux.fixture.bytes, copied, scale);
        }
        let aac = remux
            .copied
            .iter()
            .find(|copied| copied.source.kind == TrackKind::Audio)
            .expect("an AAC track");
        assert_eq!(aac.plan.spec.matroska.codec_delay, 21_333_333, "1024 ticks");
        assert_eq!(aac.plan.spec.matroska.default_duration, Some(21_333_333));
    }
}
