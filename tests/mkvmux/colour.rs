//! Container colour tags through the Matroska muxer: stream copy from
//! Matroska and MP4 fixtures, and a spec built by hand. ffprobe reads each
//! output.

use std::io::Cursor;

use containr::matroska::{DisplayUnit, MatroskaVideo, RawElement};
use containr::mkv::{MkvOptions, Muxer, PacketIn, TrackSpec};
use containr::{ContainerColour, Timebase, Track, TrackKind};

use crate::common::colour::{assert_matches_probe, first_video, probe_colour};
use crate::common::{self, mkv_probe};
use crate::mkvmux::support::{copy_mkv, open_mkv, remux_mp4, write_output};

const MS: Timebase = Timebase::from_nanoseconds(1_000_000);
const COLOUR_ID: u32 = 0x55B0;

fn colour(code_points: [u8; 3], full_range: Option<bool>) -> ContainerColour {
    ContainerColour {
        primaries: Some(code_points[0]),
        transfer: Some(code_points[1]),
        matrix: Some(code_points[2]),
        full_range,
    }
}

fn video_track(bytes: &[u8]) -> Track {
    open_mkv(bytes)
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .unwrap_or_else(|| panic!("the file has no video track"))
        .clone()
}

/// The raw `Colour` element bodies of a video track.
fn raw_colour(track: &Track) -> Vec<Vec<u8>> {
    track
        .matroska
        .as_ref()
        .and_then(|mkv| mkv.video.as_ref())
        .map(|video| {
            video
                .extra
                .iter()
                .filter(|element| element.id == COLOUR_ID)
                .map(|element| element.body.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Check a copy of Matroska fixture `name`: same colour, same raw element,
/// and ffprobe reads the same colour from both files.
fn check_mkv(name: &str, expected: ContainerColour) {
    let Some(fixture) = common::fixture(name) else {
        return;
    };
    let source = video_track(&fixture.bytes);
    assert_eq!(source.colour(), Some(expected), "{name}: source colour");
    assert_matches_probe(name, source.colour(), &fixture.probe);
    let copy = copy_mkv(name, &fixture.bytes, |_| {});
    let copied = video_track(&copy);
    assert_eq!(copied.colour(), source.colour(), "{name}: copied colour");
    assert_eq!(
        raw_colour(&copied),
        raw_colour(&source),
        "{name}: the Colour element changed"
    );
    let path = write_output(&format!("colour_copy_{name}"), &copy);
    let Some(probe) = mkv_probe::probe_file(&path) else {
        return;
    };
    assert_matches_probe(&format!("copy_{name}"), copied.colour(), &probe);
    assert_eq!(
        probe_colour(first_video(&probe)),
        probe_colour(first_video(&fixture.probe)),
        "{name}: ffprobe reads a different colour from the copy"
    );
}

#[test]
fn mkv_to_mkv_keeps_the_colour_element() {
    check_mkv("colour_bt709.mkv", colour([1, 1, 1], Some(false)));
    check_mkv("colour_bt709_full.mkv", colour([1, 1, 1], Some(true)));
    check_mkv("colour_bt2020_pq.mkv", colour([9, 16, 9], Some(false)));
    check_mkv("colour_bt2020_pq.webm", colour([9, 16, 9], Some(false)));
}

/// Check a copy of MP4 fixture `name`: the `colr` tags map to `Colour`.
fn check_mp4(name: &str, expected: ContainerColour) {
    let Some(remux) = remux_mp4(name) else {
        return;
    };
    assert!(remux.refused.is_empty(), "{name}: a track was refused");
    let source = remux
        .copied
        .iter()
        .find(|copied| copied.source.kind == TrackKind::Video)
        .unwrap_or_else(|| panic!("{name}: no video track was copied"))
        .source
        .colour();
    assert_eq!(source, Some(expected), "{name}: source colour");
    assert_matches_probe(name, source, &remux.fixture.probe);
    let copied = video_track(&remux.bytes).colour();
    assert_eq!(copied, source, "{name}: copied colour");
    let path = write_output(&format!("colour_mp4_{name}.mkv"), &remux.bytes);
    let Some(probe) = mkv_probe::probe_file(&path) else {
        return;
    };
    assert_matches_probe(&format!("mp4_{name}"), copied, &probe);
}

#[test]
fn mp4_nclx_maps_to_a_colour_element() {
    check_mp4("colour_bt709.mp4", colour([1, 1, 1], Some(false)));
    check_mp4("colour_bt2020_pq.mp4", colour([9, 16, 9], Some(false)));
}

#[test]
fn quicktime_nclc_maps_to_a_colour_element_without_range() {
    check_mp4("colour_bt709_full.mov", colour([1, 1, 1], None));
    check_mp4("colour_bt2020_pq.mov", colour([9, 16, 9], None));
}

fn spec(colour: Option<ContainerColour>, extra: Vec<RawElement>) -> TrackSpec {
    let mut spec = TrackSpec::new(TrackKind::Video, "V_VP9", MS);
    spec.matroska.video = Some(MatroskaVideo {
        pixel_width: 16,
        pixel_height: 16,
        display_width: None,
        display_height: None,
        display_unit: DisplayUnit::Pixels,
        crop: [0; 4],
        interlaced: 0,
        projection: None,
        colour,
        extra,
    });
    spec.matroska.default_duration = Some(40_000_000);
    spec
}

fn mux(spec: TrackSpec) -> Vec<u8> {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let track = muxer.add_track(spec).expect("a track");
    for pts in [0, 40] {
        let packet = PacketIn {
            data: &[1, 2],
            pts,
            duration: Some(40),
            sync: pts == 0,
            discardable: false,
            discard_padding: None,
        };
        muxer.write_packet(track, &packet).expect("a packet");
    }
    muxer.finish().expect("the file ends").into_inner()
}

#[test]
fn a_spec_built_by_hand_writes_its_colour() {
    let partial = ContainerColour {
        primaries: Some(9),
        ..ContainerColour::default()
    };
    for expected in [
        colour([1, 1, 1], Some(false)),
        colour([9, 16, 9], Some(true)),
        partial,
    ] {
        let track = video_track(&mux(spec(Some(expected), Vec::new())));
        assert_eq!(track.colour(), Some(expected));
        assert_eq!(raw_colour(&track).len(), 1);
    }
}

#[test]
fn no_colour_writes_no_element() {
    for empty in [None, Some(ContainerColour::default())] {
        let track = video_track(&mux(spec(empty, Vec::new())));
        assert_eq!(track.colour(), None);
        assert!(raw_colour(&track).is_empty());
    }
}

fn raw(body: &[u8]) -> Vec<RawElement> {
    vec![RawElement {
        id: COLOUR_ID,
        body: body.to_vec(),
    }]
}

#[test]
fn a_changed_colour_keeps_the_other_children_of_the_raw_element() {
    // Range 1 and a MaxCLL of 7.
    let source = [0x55, 0xB9, 0x81, 1, 0x55, 0xBC, 0x81, 7];
    let wanted = colour([9, 16, 9], Some(true));
    let track = video_track(&mux(spec(Some(wanted), raw(&source))));
    assert_eq!(track.colour(), Some(wanted));
    let bodies = raw_colour(&track);
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0]
            .windows(4)
            .any(|window| window == [0x55, 0xBC, 0x81, 7]),
        "MaxCLL was dropped"
    );
}

#[test]
fn clearing_the_colour_drops_the_tags_and_keeps_the_rest() {
    let source = [0x55, 0xB9, 0x81, 1, 0x55, 0xBC, 0x81, 7];
    let track = video_track(&mux(spec(None, raw(&source))));
    assert_eq!(track.colour(), None);
    assert_eq!(raw_colour(&track), vec![vec![0x55, 0xBC, 0x81, 7]]);
}

#[test]
fn a_matching_raw_element_is_written_unchanged() {
    let source = [0x55, 0xB9, 0x81, 1, 0x55, 0xBC, 0x81, 7];
    let same = ContainerColour {
        full_range: Some(false),
        ..ContainerColour::default()
    };
    let track = video_track(&mux(spec(Some(same), raw(&source))));
    assert_eq!(raw_colour(&track), vec![source.to_vec()]);
}
