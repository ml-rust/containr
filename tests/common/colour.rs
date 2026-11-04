//! Container colour tags: ffprobe's names as H.273 code points, and the
//! comparison of a demuxed value against ffprobe.

use containr::ContainerColour;
use serde_json::Value;

use super::array;

const PRIMARIES: &[(&str, u8)] = &[
    ("bt709", 1),
    ("unknown", 2),
    ("bt470m", 4),
    ("bt470bg", 5),
    ("smpte170m", 6),
    ("smpte240m", 7),
    ("film", 8),
    ("bt2020", 9),
    ("smpte428", 10),
    ("smpte431", 11),
    ("smpte432", 12),
];

const TRANSFER: &[(&str, u8)] = &[
    ("bt709", 1),
    ("unknown", 2),
    ("bt470m", 4),
    ("bt470bg", 5),
    ("smpte170m", 6),
    ("smpte240m", 7),
    ("linear", 8),
    ("iec61966-2-4", 11),
    ("iec61966-2-1", 13),
    ("bt2020-10", 14),
    ("bt2020-12", 15),
    ("smpte2084", 16),
    ("smpte428", 17),
    ("arib-std-b67", 18),
];

const MATRIX: &[(&str, u8)] = &[
    ("gbr", 0),
    ("bt709", 1),
    ("unknown", 2),
    ("fcc", 4),
    ("bt470bg", 5),
    ("smpte170m", 6),
    ("smpte240m", 7),
    ("ycgco", 8),
    ("bt2020nc", 9),
    ("bt2020c", 10),
];

fn code(table: &[(&str, u8)], stream: &Value, field: &str) -> Option<u8> {
    let name = stream[field].as_str()?;
    let found = table.iter().find(|(known, _)| *known == name);
    match found {
        Some((_, value)) => Some(*value),
        None => panic!("ffprobe {field} {name:?} is not in the test table. Add it to colour.rs."),
    }
}

/// The colour tags ffprobe reports for a stream. A field ffprobe leaves out
/// is `None`. ffprobe prints `unknown` for the H.273 value 2, which maps
/// back to 2.
pub fn probe_colour(stream: &Value) -> ContainerColour {
    ContainerColour {
        primaries: code(PRIMARIES, stream, "color_primaries"),
        transfer: code(TRANSFER, stream, "color_transfer"),
        matrix: code(MATRIX, stream, "color_space"),
        full_range: match stream["color_range"].as_str() {
            Some("tv") => Some(false),
            Some("pc") => Some(true),
            Some("unknown") | None => None,
            Some(other) => panic!("ffprobe color_range {other:?} is neither tv nor pc"),
        },
    }
}

/// The first video stream of a probe.
pub fn first_video(probe: &Value) -> &Value {
    array(probe, "streams")
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .unwrap_or_else(|| panic!("the probe has no video stream"))
}

/// Check a demuxed value against ffprobe, field by field.
///
/// A field the container tags must equal ffprobe's. A field the container
/// leaves out is not compared, since ffprobe fills it from the bitstream.
/// A container value of 2 (unspecified) equals ffprobe's `unknown` or a
/// missing field.
pub fn assert_matches_probe(label: &str, ours: Option<ContainerColour>, probe: &Value) {
    let theirs = probe_colour(first_video(probe));
    let ours = ours.unwrap_or_default();
    let unspecified = |value: Option<u8>| value.filter(|&code| code != 2);
    for (field, ours, theirs) in [
        ("primaries", ours.primaries, theirs.primaries),
        ("transfer", ours.transfer, theirs.transfer),
        ("matrix", ours.matrix, theirs.matrix),
    ] {
        if ours.is_some() {
            assert_eq!(
                unspecified(ours),
                unspecified(theirs),
                "{label}: {field} differs from ffprobe"
            );
        }
    }
    if ours.full_range.is_some() {
        assert_eq!(
            ours.full_range, theirs.full_range,
            "{label}: full_range differs from ffprobe"
        );
    }
}

/// True when the container tags at least the three code points.
pub fn has_code_points(colour: Option<ContainerColour>) -> bool {
    colour.is_some_and(|colour| {
        colour.primaries.is_some() && colour.transfer.is_some() && colour.matrix.is_some()
    })
}
