//! Container colour tags: `Video/Colour`, read back and compared against
//! ffprobe.

use std::io::Cursor;

use containr::mkv::Demuxer;
use containr::{ContainerColour, Error, TrackKind};

use crate::common::colour::assert_matches_probe;
use crate::common::{self, mkv_synth};

fn colour(code_points: [u8; 3], full_range: Option<bool>) -> Option<ContainerColour> {
    Some(ContainerColour {
        primaries: Some(code_points[0]),
        transfer: Some(code_points[1]),
        matrix: Some(code_points[2]),
        full_range,
    })
}

fn fixture_colour(name: &str, expected: Option<ContainerColour>) {
    let Some(fx) = common::fixture(name) else {
        return;
    };
    let demuxer =
        Demuxer::open(Cursor::new(fx.bytes.clone())).unwrap_or_else(|err| panic!("{err}"));
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .unwrap_or_else(|| panic!("{name}: no video track"));
    assert_eq!(video.colour(), expected, "{name}");
    assert_matches_probe(name, video.colour(), &fx.probe);
    // The raw element stays in `extra`, so a stream copy keeps its bytes.
    let extra = &video
        .matroska
        .as_ref()
        .and_then(|mkv| mkv.video.as_ref())
        .unwrap_or_else(|| panic!("{name}: no Video element"))
        .extra;
    assert!(extra.iter().any(|element| element.id == 0x55B0), "{name}");
}

#[test]
fn fixtures_match_ffprobe() {
    fixture_colour("colour_bt709.mkv", colour([1, 1, 1], Some(false)));
    fixture_colour("colour_bt709_full.mkv", colour([1, 1, 1], Some(true)));
    fixture_colour("colour_bt2020_pq.mkv", colour([9, 16, 9], Some(false)));
    fixture_colour("colour_bt2020_pq.webm", colour([9, 16, 9], Some(false)));
}

#[test]
fn an_untagged_fixture_has_no_colour() {
    let Some(fx) = common::fixture("h264_aac.mkv") else {
        return;
    };
    let demuxer = Demuxer::open(Cursor::new(fx.bytes)).unwrap_or_else(|err| panic!("{err}"));
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .unwrap_or_else(|| panic!("no video track"));
    // FFmpeg's Matroska writer tags `Range` on every file, so only the code
    // points stay unset.
    let colour = video.colour().unwrap_or_default();
    assert_eq!(
        (colour.primaries, colour.transfer, colour.matrix),
        (None, None, None)
    );
}

fn synthetic(body: Option<&[u8]>) -> Result<Option<ContainerColour>, Error> {
    let synth = mkv_synth::file_with_colour(false, body);
    let demuxer = Demuxer::open(Cursor::new(synth.bytes))?;
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.id == mkv_synth::VIDEO)
        .unwrap_or_else(|| panic!("the synthetic file has a video track"));
    Ok(video.colour())
}

#[test]
fn synthetic_range_values_map() {
    // Range, then Primaries 9.
    for (range, expected) in [(0u8, None), (1, Some(false)), (2, Some(true)), (3, None)] {
        let body = [0x55, 0xB9, 0x81, range, 0x55, 0xBB, 0x81, 9];
        let parsed = synthetic(Some(&body)).expect("the file is well formed");
        assert_eq!(
            parsed,
            Some(ContainerColour {
                primaries: Some(9),
                transfer: None,
                matrix: None,
                full_range: expected,
            }),
            "Range {range}"
        );
    }
}

#[test]
fn a_colour_element_with_no_tags_gives_none() {
    // MaxCLL only.
    let body = [0x55, 0xBC, 0x82, 0x03, 0xE8];
    assert_eq!(synthetic(Some(&body)).ok(), Some(None));
    assert_eq!(synthetic(None).ok(), Some(None));
}

#[test]
fn a_code_point_above_255_names_the_element_path() {
    for (id, name) in [
        (0xB1u8, "MatrixCoefficients"),
        (0xBA, "TransferCharacteristics"),
        (0xBB, "Primaries"),
    ] {
        let body = [0x55, id, 0x82, 0x01, 0x00];
        let err = synthetic(Some(&body)).expect_err("256 is refused");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");
        let text = err.to_string();
        assert!(text.contains(&format!("Video/Colour/{name}")), "{text}");
        assert!(text.contains("256"), "{text}");
    }
}
