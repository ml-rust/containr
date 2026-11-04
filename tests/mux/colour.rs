//! Container colour tags through the MP4 muxer: stream copy from MP4 and
//! Matroska fixtures, and a spec built by hand. ffprobe reads each output.

use std::io::Cursor;

use containr::mp4::{MuxOptions, Muxer, PacketIn, SampleEntrySpec, TrackSpec, VisualCodec};
use containr::{ContainerColour, TrackKind};

use crate::common::colour::{assert_matches_probe, first_video, has_code_points, probe_colour};
use crate::common::mp4_probe;
use crate::mux::support::{Remux, open_mp4, remux_mkv, remux_mp4, write_output};

fn colour(code_points: [u8; 3], full_range: Option<bool>) -> ContainerColour {
    ContainerColour {
        primaries: Some(code_points[0]),
        transfer: Some(code_points[1]),
        matrix: Some(code_points[2]),
        full_range,
    }
}

/// The colour of the first video track in `bytes`.
fn output_colour(bytes: &[u8]) -> Option<ContainerColour> {
    open_mp4(bytes)
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .and_then(|track| track.colour())
}

/// Check one copy. The source colour must be `expected`. The output must
/// carry the same colour in the demuxer, and ffprobe must read the same
/// colour from the output as from the source.
fn check_copy(label: &str, remux: &Remux, expected: ContainerColour) {
    let source = remux
        .copied
        .iter()
        .find(|copied| copied.source.kind == TrackKind::Video)
        .unwrap_or_else(|| panic!("{label}: no video track was copied"))
        .source
        .colour();
    assert_eq!(source, Some(expected), "{label}: source colour");
    assert_matches_probe(&format!("{label} source"), source, &remux.fixture.probe);
    let copied = output_colour(&remux.bytes);
    assert_eq!(copied, source, "{label}: output colour");
    let path = write_output(&format!("colour_{label}"), &remux.bytes);
    let Some(probe) = mp4_probe::probe_file(&path) else {
        return;
    };
    assert_matches_probe(&format!("{label} output"), copied, &probe);
    assert_eq!(
        probe_colour(first_video(&probe)),
        probe_colour(first_video(&remux.fixture.probe)),
        "{label}: ffprobe reads a different colour from the copy"
    );
}

fn check_mp4(name: &str, expected: ContainerColour) {
    for faststart in [false, true] {
        let Some(remux) = remux_mp4(name, faststart) else {
            return;
        };
        let suffix = if faststart { "fast" } else { "end" };
        check_copy(&format!("{suffix}_{name}"), &remux, expected);
    }
}

fn check_mkv(name: &str, expected: ContainerColour) {
    let Some(remux) = remux_mkv(name, false) else {
        return;
    };
    assert!(remux.refused.is_empty(), "{name}: a track was refused");
    let copied = output_colour(&remux.bytes);
    assert!(has_code_points(copied), "{name}: the copy lost its tags");
    check_copy(&format!("mkv_{name}.mp4"), &remux, expected);
}

#[test]
fn mp4_to_mp4_keeps_nclx_colour() {
    check_mp4("colour_bt709.mp4", colour([1, 1, 1], Some(false)));
    check_mp4("colour_bt2020_pq.mp4", colour([9, 16, 9], Some(false)));
}

#[test]
fn mov_to_mov_keeps_nclc_colour() {
    let nclc = |code_points| colour(code_points, None);
    check_mp4("colour_bt709_full.mov", nclc([1, 1, 1]));
    check_mp4("colour_bt2020_pq.mov", nclc([9, 16, 9]));
}

#[test]
fn mkv_to_mp4_maps_colour_to_an_nclx_box() {
    check_mkv("colour_bt709.mkv", colour([1, 1, 1], Some(false)));
    check_mkv("colour_bt709_full.mkv", colour([1, 1, 1], Some(true)));
    check_mkv("colour_bt2020_pq.mkv", colour([9, 16, 9], Some(false)));
}

fn built(colour: Option<ContainerColour>) -> Vec<u8> {
    let mut spec = TrackSpec::new(
        TrackKind::Video,
        90_000,
        SampleEntrySpec::Visual {
            codec: VisualCodec::Avc,
            config: vec![1, 0x64, 0, 0x1E],
            width: 96,
            height: 64,
        },
    );
    spec.colour = colour;
    let mut muxer =
        Muxer::new(Cursor::new(Vec::new()), MuxOptions::default()).expect("the options are valid");
    let track = muxer.add_track(spec).expect("the spec is valid");
    for index in 0..3i64 {
        let packet = PacketIn {
            data: &[7; 16],
            pts: index * 3600,
            dts: index * 3600,
            duration: Some(3600),
            sync: index == 0,
        };
        muxer.write_packet(track, &packet).expect("a packet");
    }
    muxer.finish().expect("the movie finishes").into_inner()
}

#[test]
fn a_spec_built_by_hand_writes_its_colour() {
    for expected in [
        colour([1, 1, 1], Some(false)),
        colour([9, 16, 9], Some(true)),
    ] {
        assert_eq!(output_colour(&built(Some(expected))), Some(expected));
    }
}

#[test]
fn an_unset_tag_is_written_as_unspecified() {
    let partial = ContainerColour {
        primaries: Some(9),
        ..ContainerColour::default()
    };
    assert_eq!(
        output_colour(&built(Some(partial))),
        Some(ContainerColour {
            primaries: Some(9),
            transfer: Some(2),
            matrix: Some(2),
            full_range: Some(false),
        })
    );
}

#[test]
fn no_colour_writes_no_colr_box() {
    for spec_colour in [None, Some(ContainerColour::default())] {
        let bytes = built(spec_colour);
        assert!(!bytes.windows(4).any(|window| window == b"colr"));
        assert_eq!(output_colour(&bytes), None);
    }
}
