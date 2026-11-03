//! MP4 and MOV fixtures: demux, stream copy, mux, demux again. Everything
//! the demuxer reports must come back equal.

use containr::{ChapterSource, MetadataItem};

use crate::mux::support::{Remux, collect, ok, open_mp4, remux_mp4};

/// Metadata in a stable order: the muxer groups items by where they belong.
fn sorted(items: &[MetadataItem]) -> Vec<String> {
    let mut out: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();
    out.sort();
    out
}

fn check_tracks(name: &str, remux: &Remux) {
    let mut input = open_mp4(&remux.fixture.bytes);
    let mut output = remux.output();
    for copied in &remux.copied {
        let what = format!("{name} track {}", copied.source.id);
        let ours = output
            .track(copied.id.get())
            .unwrap_or_else(|| panic!("{what}: absent from the output"))
            .clone();
        let source = &copied.source;
        assert_eq!(ours.kind, source.kind, "{what}: kind");
        assert_eq!(ours.handler, source.handler, "{what}: handler");
        assert_eq!(ours.timebase, source.timebase, "{what}: timescale");
        assert_eq!(ours.language, source.language, "{what}: language");
        assert_eq!(ours.matrix, source.matrix, "{what}: matrix");
        assert_eq!(ours.display_width, source.display_width, "{what}: width");
        assert_eq!(ours.display_height, source.display_height, "{what}: height");
        assert_eq!(ours.edit_list, source.edit_list, "{what}: edit list");
        assert_eq!(
            ours.sample_entries, source.sample_entries,
            "{what}: sample entry"
        );
        assert_eq!(ours.codec_config(), source.codec_config(), "{what}: config");
        assert_eq!(ours.sample_count, source.sample_count, "{what}: samples");
        assert_eq!(
            sorted(&ours.metadata),
            sorted(&source.metadata),
            "{what}: metadata"
        );

        let theirs = collect(name, ok(name, input.track_packets(source.id)));
        let mine = collect(name, ok(name, output.track_packets(ours.id)));
        assert_eq!(mine.len(), theirs.len(), "{what}: packet count");
        let mut total = 0u64;
        for (n, (mine, theirs)) in mine.iter().zip(&theirs).enumerate() {
            let at = format!("{what} packet {n}");
            assert_eq!(mine.pts, theirs.pts, "{at}: pts");
            assert_eq!(mine.dts, theirs.dts, "{at}: dts");
            assert_eq!(mine.duration, theirs.duration, "{at}: duration");
            assert_eq!(mine.sync, theirs.sync, "{at}: sync");
            assert_eq!(mine.size, theirs.size, "{at}: size");
            total += mine.duration.unwrap_or(0);
            let their_bytes = ok(name, input.read_packet(theirs));
            let my_bytes = ok(name, output.read_packet(mine));
            assert_eq!(my_bytes, their_bytes, "{at}: bytes");
        }
        assert_eq!(ours.duration, Some(total), "{what}: mdhd duration");
    }
    assert_eq!(
        sorted(output.metadata()),
        sorted(input.metadata()),
        "{name}: movie metadata"
    );
    assert_eq!(
        output.movie_timescale(),
        input.movie_timescale(),
        "{name}: movie timescale"
    );
}

/// `chpl` and chapter-track chapters come back with their starts and titles.
fn check_chapters(name: &str, remux: &Remux) {
    let input = open_mp4(&remux.fixture.bytes);
    let output = remux.output();
    let key = |chapter: &containr::Chapter| {
        (
            chapter.start,
            chapter.duration,
            chapter.timescale,
            chapter.title.clone(),
        )
    };
    let split = |chapters: &[containr::Chapter]| {
        let (from_chpl, from_track): (Vec<_>, Vec<_>) = chapters
            .iter()
            .partition(|chapter| chapter.source == ChapterSource::Chpl);
        (
            from_chpl.into_iter().map(key).collect::<Vec<_>>(),
            from_track.into_iter().map(key).collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        split(output.chapters()),
        split(input.chapters()),
        "{name}: chapters"
    );
    assert_eq!(
        output.tracks().iter().filter(|t| t.chapter_track).count(),
        input.tracks().iter().filter(|t| t.chapter_track).count(),
        "{name}: chapter tracks"
    );
}

fn check(name: &str) {
    for faststart in [false, true] {
        let Some(remux) = remux_mp4(name, faststart) else {
            return;
        };
        check_tracks(name, &remux);
        check_chapters(name, &remux);
    }
}

#[test]
fn round_trip_h264_aac_moov_at_end() {
    check("h264_aac_moov_end.mp4");
}

#[test]
fn round_trip_h264_aac_faststart() {
    check("h264_aac_faststart.mp4");
}

#[test]
fn round_trip_h264_negative_cts() {
    check("h264_negative_cts.mp4");
}

#[test]
fn round_trip_hevc_opus() {
    check("hevc_opus.mp4");
}

#[test]
fn round_trip_mov_text() {
    check("mov_text.mp4");
}

#[test]
fn round_trip_rotate90() {
    check("rotate90.mp4");
}

#[test]
fn round_trip_itsoffset() {
    check("itsoffset.mp4");
}

#[test]
fn round_trip_ss_copy() {
    check("ss_copy.mp4");
}

#[test]
fn round_trip_chapters_mp4() {
    check("chapters_mp4.mp4");
}

#[test]
fn round_trip_chapters_mov() {
    check("chapters_mov.mov");
}
