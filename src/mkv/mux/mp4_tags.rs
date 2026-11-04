//! MP4 metadata as Matroska tags.
//!
//! A text item with a known key becomes a `TagString`. A QuickTime `udta`
//! text box holds a 16-bit length, a 16-bit language and the text. A named
//! key keeps its name. A track's `udta/name` box holds its title as bare text.
//! Anything else has no Matroska form, and is refused.

use crate::error::Result;
use crate::metadata::{MetadataItem, MetadataKey, MetadataScope, TagScope, TagTarget};
use crate::mkv::mux::bytes::refuse;
use crate::track::FourCc;

/// The `udta` box that holds a track title as bare text, as FFmpeg writes it.
const NAME_BOX: FourCc = FourCc(*b"name");
/// `TargetTypeValue` 50: a whole movie, album or episode.
const SEGMENT_TARGET: u64 = 50;

/// The Matroska tag name of an MP4 metadata box.
fn tag_name(key: FourCc) -> Option<&'static str> {
    Some(match &key.0 {
        b"\xA9nam" => "TITLE",
        b"\xA9too" | b"\xA9swr" => "ENCODER",
        b"\xA9ART" => "ARTIST",
        b"aART" => "ACCOMPANIMENT",
        b"\xA9alb" => "ALBUM",
        b"\xA9day" => "DATE_RELEASED",
        b"\xA9cmt" => "COMMENT",
        b"\xA9gen" => "GENRE",
        b"\xA9wrt" => "COMPOSER",
        b"desc" => "DESCRIPTION",
        b"cprt" => "COPYRIGHT",
        b"name" => "TITLE",
        _ => return None,
    })
}

/// The text of a QuickTime `udta` text box.
fn udta_text(value: &[u8]) -> Option<&str> {
    let [high, low, _, _, text @ ..] = value else {
        return None;
    };
    let text = text.get(..usize::from(u16::from_be_bytes([*high, *low])))?;
    std::str::from_utf8(text).ok()
}

fn describe(item: &MetadataItem) -> String {
    match &item.key {
        MetadataKey::FourCc(key) => format!("`{key}`"),
        MetadataKey::Named(name) => format!("`{}`", String::from_utf8_lossy(name)),
    }
}

/// Tags for the metadata of an MP4 track or movie, each with target
/// [`TagTarget::Segment`]. A Matroska tag passes through unchanged.
///
/// An item that is not text, or a box key outside the known list, gives
/// [`Error::Unsupported`](crate::Error::Unsupported), which names MP4. Filter
/// the list first to copy the rest. Known keys: `©nam`, `©too`, `©swr`, `©ART`,
/// `aART`, `©alb`, `©day`, `©cmt`, `©gen`, `©wrt`, `desc`, `cprt`, `name`.
pub fn tags_from_mp4(items: &[MetadataItem]) -> Result<Vec<MetadataItem>> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if matches!(item.scope, MetadataScope::Tag(_)) {
            out.push(item.clone());
            continue;
        }
        let name = match &item.key {
            MetadataKey::FourCc(key) => tag_name(*key).map(str::to_owned),
            MetadataKey::Named(name) => std::str::from_utf8(name).ok().map(str::to_owned),
        };
        let text = match item.scope {
            MetadataScope::Udta if item.key == MetadataKey::FourCc(NAME_BOX) => {
                std::str::from_utf8(&item.value).ok()
            }
            MetadataScope::Udta => udta_text(&item.value),
            _ => item.text(),
        };
        let (Some(name), Some(text)) = (name, text) else {
            return Err(refuse(format!(
                "metadata item {} with no Matroska tag form: this muxer maps text items with \
                 known or named keys",
                describe(item)
            )));
        };
        out.push(MetadataItem {
            scope: MetadataScope::Tag(TagScope {
                target_type_value: SEGMENT_TARGET,
                target: TagTarget::Segment,
                binary: false,
            }),
            key: MetadataKey::Named(name.into_bytes()),
            value: text.as_bytes().to_vec(),
            data: Vec::new(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::MetadataData;

    fn ilst(key: &[u8; 4], text: &str) -> MetadataItem {
        MetadataItem {
            scope: MetadataScope::Ilst,
            key: MetadataKey::FourCc(FourCc(*key)),
            value: Vec::new(),
            data: vec![MetadataData {
                type_indicator: 1,
                locale: 0,
                payload: text.as_bytes().to_vec(),
            }],
        }
    }

    #[test]
    fn known_keys_become_named_tags() {
        let tags = tags_from_mp4(&[ilst(b"\xA9too", "Lavf")]).expect("the item maps");
        assert_eq!(tags[0].name(), Some("ENCODER"));
        assert_eq!(tags[0].text(), Some("Lavf"));
    }

    #[test]
    fn quicktime_text_boxes_lose_their_length_and_language() {
        let item = MetadataItem {
            scope: MetadataScope::Udta,
            key: MetadataKey::FourCc(FourCc(*b"\xA9nam")),
            value: b"\x00\x02U\xC4hi".to_vec(),
            data: Vec::new(),
        };
        let tags = tags_from_mp4(&[item]).expect("the item maps");
        assert_eq!(tags[0].name(), Some("TITLE"));
        assert_eq!(tags[0].text(), Some("hi"));
    }

    #[test]
    fn a_name_box_is_a_title_of_bare_text() {
        let item = MetadataItem {
            scope: MetadataScope::Udta,
            key: MetadataKey::FourCc(NAME_BOX),
            value: b"Tone".to_vec(),
            data: Vec::new(),
        };
        let tags = tags_from_mp4(&[item]).expect("the item maps");
        assert_eq!(tags[0].name(), Some("TITLE"));
        assert_eq!(tags[0].text(), Some("Tone"));
    }

    #[test]
    fn other_items_are_refused_by_key() {
        let err = tags_from_mp4(&[ilst(b"covr", "x")]).expect_err("the key is unknown");
        assert!(err.to_string().contains("covr"), "{err}");
        assert!(err.to_string().contains("MP4/MOV"), "{err}");
    }
}
