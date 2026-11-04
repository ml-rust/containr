//! Container colour tags: the `colr` box, read back and compared against
//! ffprobe.

use std::io::Cursor;

use containr::mp4::Demuxer;
use containr::{ContainerColour, Error};

use crate::common::colour::assert_matches_probe;
use crate::common::{self, synth};

const NCLX_BT2020_PQ_FULL: [u8; 11] = [b'n', b'c', b'l', b'x', 0, 9, 0, 16, 0, 9, 0x80];
const NCLC_BT709: [u8; 10] = [b'n', b'c', b'l', b'c', 0, 1, 0, 1, 0, 1];

fn colour(code_points: [u8; 3], full_range: Option<bool>) -> Option<ContainerColour> {
    Some(ContainerColour {
        primaries: Some(code_points[0]),
        transfer: Some(code_points[1]),
        matrix: Some(code_points[2]),
        full_range,
    })
}

/// The colour of the first video track of fixture `name`, and its probe.
fn fixture_colour(name: &str, expected: Option<ContainerColour>) {
    let Some(fx) = common::fixture(name) else {
        return;
    };
    let demuxer =
        Demuxer::open(Cursor::new(fx.bytes.clone())).unwrap_or_else(|err| panic!("{err}"));
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.kind == containr::TrackKind::Video)
        .unwrap_or_else(|| panic!("{name}: no video track"));
    assert_eq!(video.colour(), expected, "{name}");
    assert_eq!(
        video.video().and_then(|info| info.colour),
        expected,
        "{name}"
    );
    assert_matches_probe(name, video.colour(), &fx.probe);
}

#[test]
fn nclx_fixtures_match_ffprobe() {
    fixture_colour("colour_bt709.mp4", colour([1, 1, 1], Some(false)));
    fixture_colour("colour_bt2020_pq.mp4", colour([9, 16, 9], Some(false)));
}

#[test]
fn quicktime_nclc_fixtures_match_ffprobe_and_carry_no_range() {
    fixture_colour("colour_bt709_full.mov", colour([1, 1, 1], None));
    fixture_colour("colour_bt2020_pq.mov", colour([9, 16, 9], None));
}

#[test]
fn an_untagged_fixture_has_no_colour() {
    fixture_colour("h264_aac_moov_end.mp4", None);
}

fn synthetic(colrs: &[&[u8]]) -> Result<Option<ContainerColour>, Error> {
    let (bytes, _) = synth::movie_with_colr(true, colrs);
    let demuxer = Demuxer::open(Cursor::new(bytes))?;
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.id == synth::VIDEO_ID)
        .unwrap_or_else(|| panic!("the synthetic movie has a video track"));
    Ok(video.colour())
}

#[test]
fn synthetic_nclx_and_nclc_boxes_parse() {
    assert_eq!(
        synthetic(&[&NCLX_BT2020_PQ_FULL]).ok(),
        Some(colour([9, 16, 9], Some(true)))
    );
    assert_eq!(
        synthetic(&[&NCLC_BT709]).ok(),
        Some(colour([1, 1, 1], None))
    );
    assert_eq!(synthetic(&[]).ok(), Some(None));
}

#[test]
fn icc_boxes_give_no_values_unless_an_nclx_box_follows() {
    assert_eq!(synthetic(&[b"prof\x00\x01"]).ok(), Some(None));
    assert_eq!(synthetic(&[b"rICC\x00\x01"]).ok(), Some(None));
    assert_eq!(
        synthetic(&[b"prof\x00\x01", &NCLX_BT2020_PQ_FULL]).ok(),
        Some(colour([9, 16, 9], Some(true)))
    );
}

#[test]
fn the_first_box_with_values_wins() {
    assert_eq!(
        synthetic(&[&NCLC_BT709, &NCLX_BT2020_PQ_FULL]).ok(),
        Some(colour([1, 1, 1], None))
    );
}
