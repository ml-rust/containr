//! The `colr` box of a visual sample entry: read the H.273 tags, and write
//! an `nclx` box.
//!
//! `nclx` holds the three code points and the full-range bit. QuickTime
//! `nclc` holds the three code points only. `prof` and `rICC` hold an ICC
//! profile, which gives no H.273 values.

use crate::colour::ContainerColour;
use crate::error::{Error, Result};
use crate::mp4::boxes::RawBox;

/// Bytes of an `nclx` body: type, three 16-bit values, one flag byte.
pub(crate) const NCLX_BODY_LEN: usize = 11;
/// The H.273 value for "unspecified" in all three code point fields.
const UNSPECIFIED: u8 = 2;

/// Parse a `colr` box. `None` for a colour type with no H.273 values.
pub(crate) fn parse_colr(colr: &RawBox<'_>, path: &str) -> Result<Option<ContainerColour>> {
    let mut reader = colr.reader(path);
    let colour_type = reader.fourcc("colour_type")?;
    let has_range = match &colour_type.0 {
        b"nclx" => true,
        b"nclc" => false,
        _ => return Ok(None),
    };
    let primaries = code_point(
        reader.u16("colour_primaries")?,
        "colour_primaries",
        colr,
        path,
    )?;
    let transfer = code_point(
        reader.u16("transfer_characteristics")?,
        "transfer_characteristics",
        colr,
        path,
    )?;
    let matrix = code_point(
        reader.u16("matrix_coefficients")?,
        "matrix_coefficients",
        colr,
        path,
    )?;
    let full_range = if has_range {
        Some(reader.u8("full_range_flag")? & 0x80 != 0)
    } else {
        None
    };
    Ok(Some(ContainerColour {
        primaries: Some(primaries),
        transfer: Some(transfer),
        matrix: Some(matrix),
        full_range,
    }))
}

/// An H.273 code point: one byte. A larger value is malformed.
fn code_point(value: u16, field: &str, colr: &RawBox<'_>, path: &str) -> Result<u8> {
    u8::try_from(value).map_err(|_| {
        Error::mp4_malformed(
            path,
            colr.offset,
            format!(
                "field `{field}` is {value}, above 255. H.273 code points fit one byte. \
                 Fix the source tag or remove the `colr` box"
            ),
        )
    })
}

/// The body of an `nclx` `colr` box for `colour`. A tag the container does
/// not set is written as 2 (unspecified), and a missing range as limited.
/// `None` when no field is set, since the box carries no information.
pub(crate) fn nclx_body(colour: &ContainerColour) -> Option<[u8; NCLX_BODY_LEN]> {
    if colour.is_empty() {
        return None;
    }
    let [primaries, transfer, matrix] = [colour.primaries, colour.transfer, colour.matrix]
        .map(|value| u16::from(value.unwrap_or(UNSPECIFIED)).to_be_bytes());
    let range = if colour.full_range == Some(true) {
        0x80
    } else {
        0
    };
    Some([
        b'n',
        b'c',
        b'l',
        b'x',
        primaries[0],
        primaries[1],
        transfer[0],
        transfer[1],
        matrix[0],
        matrix[1],
        range,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colr(body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"colr");
        out.extend_from_slice(body);
        out
    }

    fn parse(body: &[u8]) -> Result<Option<ContainerColour>> {
        let bytes = colr(body);
        let raw = RawBox::whole(&bytes, 0, "colr")?;
        parse_colr(&raw, "colr")
    }

    #[test]
    fn nclx_yields_code_points_and_range() {
        let parsed = parse(&[b'n', b'c', b'l', b'x', 0, 9, 0, 16, 0, 9, 0x80])
            .expect("the box is well formed");
        assert_eq!(
            parsed,
            Some(ContainerColour {
                primaries: Some(9),
                transfer: Some(16),
                matrix: Some(9),
                full_range: Some(true),
            })
        );
    }

    #[test]
    fn nclc_has_no_range() {
        let parsed = parse(&[b'n', b'c', b'l', b'c', 0, 1, 0, 1, 0, 1]).expect("well formed");
        assert_eq!(parsed.and_then(|colour| colour.full_range), None);
        assert_eq!(parsed.and_then(|colour| colour.primaries), Some(1));
    }

    #[test]
    fn icc_profiles_give_no_values() {
        assert_eq!(parse(b"prof\x01\x02\x03").ok(), Some(None));
        assert_eq!(parse(b"rICC").ok(), Some(None));
    }

    #[test]
    fn short_and_oversized_boxes_are_malformed() {
        assert!(matches!(
            parse(&[b'n', b'c', b'l', b'x', 0, 9, 0, 16]),
            Err(Error::Malformed { .. })
        ));
        assert!(matches!(
            parse(&[b'n', b'c', b'l', b'x', 0, 9, 0, 16, 0, 9]),
            Err(Error::Malformed { .. })
        ));
        assert!(matches!(
            parse(&[b'n', b'c', b'l', b'x', 1, 0, 0, 16, 0, 9, 0]),
            Err(Error::Malformed { .. })
        ));
        assert!(matches!(parse(b"nc"), Err(Error::Malformed { .. })));
    }

    #[test]
    fn nclx_body_round_trips_and_fills_unset_tags() {
        let colour = ContainerColour {
            primaries: Some(9),
            transfer: Some(16),
            matrix: None,
            full_range: Some(false),
        };
        let body = nclx_body(&colour).expect("a tagged colour writes");
        assert_eq!(
            parse(&body).ok().flatten(),
            Some(ContainerColour {
                matrix: Some(2),
                ..colour
            })
        );
        assert_eq!(nclx_body(&ContainerColour::default()), None);
    }
}
