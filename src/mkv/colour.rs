//! The `Video/Colour` element: parse the colour tags, and write them back.
//!
//! The demuxer keeps the raw element in `MatroskaVideo::extra`, so
//! `MasteringMetadata`, `MaxCLL` and the other children survive stream copy.
//! `MatroskaVideo::colour` holds the four H.273 tags parsed from it.

use crate::colour::ContainerColour;
use crate::error::{Error, Result};
use crate::mkv::ebml::{Children, Element, child_path};
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;

/// The element IDs of the four tags, in the order the writer emits them.
const TAG_IDS: [u32; 4] = [
    ids::MATRIX_COEFFICIENTS,
    ids::TRANSFER_CHARACTERISTICS,
    ids::PRIMARIES,
    ids::RANGE,
];

/// Parse a `Colour` element. `None` when it holds none of the four tags.
pub(crate) fn parse_colour(colour: &Element<'_>, path: &str) -> Result<Option<ContainerColour>> {
    parse_body(colour.body, colour.body_offset, path)
}

fn parse_body(body: &[u8], offset: u64, path: &str) -> Result<Option<ContainerColour>> {
    let mut out = ContainerColour::default();
    for child in Children::new(body, offset, path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::MATRIX_COEFFICIENTS => out.matrix = Some(code_point(&child, &at)?),
            ids::TRANSFER_CHARACTERISTICS => out.transfer = Some(code_point(&child, &at)?),
            ids::PRIMARIES => out.primaries = Some(code_point(&child, &at)?),
            ids::RANGE => {
                out.full_range = match child.uint(&at)? {
                    1 => Some(false),
                    2 => Some(true),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    Ok(out.non_empty())
}

/// An H.273 code point: one byte. A larger value is malformed.
fn code_point(element: &Element<'_>, path: &str) -> Result<u8> {
    let value = element.uint(path)?;
    u8::try_from(value).map_err(|_| {
        Error::mkv_malformed(
            path,
            element.offset,
            format!(
                "the value is {value}, above 255. H.273 code points fit one byte. \
                 Remove the element or write a value from 0 to 255"
            ),
        )
    })
}

/// Write the `Colour` element for a video track.
///
/// `raw` is the body of the `Colour` element the demuxer kept, if any. It
/// is written unchanged when it parses to `colour`. Otherwise the writer
/// keeps its other children and replaces the four tags with `colour`. An
/// element left with no child is not written.
pub(crate) fn write_colour(
    out: &mut ElementBuf,
    colour: Option<ContainerColour>,
    raw: Option<&[u8]>,
) -> Result<()> {
    let want = colour.and_then(ContainerColour::non_empty);
    if let Some(body) = raw
        && matches!(parse_body(body, 0, "Colour"), Ok(parsed) if parsed == want)
    {
        return out.binary(ids::COLOUR, body);
    }
    let mut body = ElementBuf::new();
    if let Some(body_bytes) = raw {
        for child in Children::new(body_bytes, 0, "Colour").map_while(|child| child.ok()) {
            if !TAG_IDS.contains(&child.id) {
                body.binary(child.id, child.body)?;
            }
        }
    }
    if let Some(want) = want {
        if let Some(matrix) = want.matrix {
            body.uint(ids::MATRIX_COEFFICIENTS, u64::from(matrix))?;
        }
        if let Some(transfer) = want.transfer {
            body.uint(ids::TRANSFER_CHARACTERISTICS, u64::from(transfer))?;
        }
        if let Some(primaries) = want.primaries {
            body.uint(ids::PRIMARIES, u64::from(primaries))?;
        }
        if let Some(full_range) = want.full_range {
            body.uint(ids::RANGE, if full_range { 2 } else { 1 })?;
        }
    }
    if body.is_empty() {
        return Ok(());
    }
    out.master(ids::COLOUR, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(body: &[u8]) -> Element<'_> {
        Element {
            id: ids::COLOUR,
            offset: 0,
            body_offset: 0,
            body,
        }
    }

    fn written(colour: Option<ContainerColour>, raw: Option<&[u8]>) -> Vec<u8> {
        let mut out = ElementBuf::new();
        write_colour(&mut out, colour, raw).expect("the element fits");
        out.into_bytes()
    }

    fn bt2020_pq() -> ContainerColour {
        ContainerColour {
            primaries: Some(9),
            transfer: Some(16),
            matrix: Some(9),
            full_range: Some(false),
        }
    }

    #[test]
    fn written_tags_read_back() {
        let bytes = written(Some(bt2020_pq()), None);
        let mut children = Children::new(&bytes, 0, "t");
        let Some(Ok(parsed)) = children.next() else {
            panic!("the element is written");
        };
        assert_eq!(parsed.id, ids::COLOUR);
        assert_eq!(
            parse_colour(&parsed, "t/Colour").ok(),
            Some(Some(bt2020_pq()))
        );
    }

    #[test]
    fn range_values_map_and_unspecified_gives_none() {
        for (value, expected) in [(0u8, None), (1, Some(false)), (2, Some(true)), (3, None)] {
            let body = [0x55, 0xB9, 0x81, value, 0x55, 0xBB, 0x81, 1];
            let parsed = parse_colour(&element(&body), "Colour")
                .expect("the element is well formed")
                .expect("primaries are set");
            assert_eq!(parsed.full_range, expected);
            assert_eq!(parsed.primaries, Some(1));
        }
    }

    #[test]
    fn a_value_above_255_names_the_element() {
        let body = [0x55, 0xBA, 0x82, 0x01, 0x00];
        let err = parse_colour(&element(&body), "Segment/Tracks/Colour")
            .expect_err("256 is refused")
            .to_string();
        assert!(err.contains("TransferCharacteristics"), "{err}");
        assert!(err.contains("256"), "{err}");
    }

    #[test]
    fn matching_raw_bytes_are_kept_and_other_children_survive_a_change() {
        // Range 1, then a child this module does not model.
        let raw = [0x55, 0xB9, 0x81, 1, 0x55, 0xBC, 0x81, 7];
        let same = ContainerColour {
            full_range: Some(false),
            ..ContainerColour::default()
        };
        let kept = written(Some(same), Some(&raw));
        assert_eq!(&kept[kept.len() - raw.len()..], &raw);
        let changed = ContainerColour {
            full_range: Some(true),
            ..ContainerColour::default()
        };
        let rewritten = written(Some(changed), Some(&raw));
        let mut children = Children::new(&rewritten, 0, "t");
        let Some(Ok(parsed)) = children.next() else {
            panic!("the element is written");
        };
        let inner: Vec<u32> = parsed
            .children("t")
            .filter_map(|child| child.ok().map(|child| child.id))
            .collect();
        assert_eq!(inner, vec![0x55BC, ids::RANGE]);
    }

    #[test]
    fn clearing_every_tag_drops_an_element_with_no_other_child() {
        let raw = [0x55, 0xB9, 0x81, 2];
        assert!(written(None, Some(&raw)).is_empty());
        assert!(written(None, None).is_empty());
    }
}
