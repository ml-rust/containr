//! `udta`, `meta`, `ilst`, `keys` and `chpl` from shared metadata.
//!
//! Each item lands where the demuxer reads it back with the same scope and
//! key:
//! - [`MetadataScope::Udta`] with a four-character key: a raw `udta` child.
//! - [`MetadataScope::Ilst`] with a four-character key: an item of
//!   `udta/meta/ilst`, under an `mdir` handler, as FFmpeg writes it.
//! - A named key, from `ilst` or a Matroska tag: an item of `meta/ilst`,
//!   under an `mdta` handler with a `keys` box, as QuickTime writes it.
//!
//! Raw items keep their bodies unchanged. A Matroska tag becomes one `data`
//! box: type 1 (UTF-8) for a `TagString`, type 0 for a `TagBinary`.

use mp4_atom::{Chpl, ChplEntry};

use crate::error::{Container, Error, Result};
use crate::metadata::{Chapter, MetadataItem, MetadataKey, MetadataScope, TagTarget};
use crate::mp4::metadata::CHPL_TIMESCALE;
use crate::mp4::mux::bytes::BoxBuf;
use crate::mp4::mux::bytes::invalid;

/// The most chapters `chpl` holds: its count is 8 bits.
pub const MAX_CHPL_CHAPTERS: usize = 255;

/// Where a metadata item belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Movie,
    Track,
}

/// Check that every item fits MP4 at `level`.
pub(crate) fn check_items(items: &[MetadataItem], level: Level) -> Result<()> {
    for item in items {
        match (item.scope, &item.key) {
            (MetadataScope::Udta, MetadataKey::Named(name)) => {
                return Err(invalid(format!(
                    "a `udta` item has the named key {:?}. A `udta` child needs a \
                     four-character key",
                    String::from_utf8_lossy(name)
                )));
            }
            (MetadataScope::Tag(scope), _) => {
                let fits = match scope.target {
                    TagTarget::Segment => true,
                    TagTarget::Track(_) => level == Level::Track,
                    TagTarget::Edition(_) | TagTarget::Chapter(_) | TagTarget::Attachment(_) => {
                        false
                    }
                };
                if !fits {
                    return Err(Error::Unsupported {
                        container: Container::Mp4,
                        what: format!(
                            "a Matroska tag `{}` that targets {:?}",
                            item.name().unwrap_or("?"),
                            scope.target
                        ),
                        use_instead: Container::Matroska,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Check chapters: starts in order, positive timescales, and titles that
/// fit `chpl`.
pub(crate) fn check_chapters(chapters: &[Chapter]) -> Result<()> {
    if chapters.len() > MAX_CHPL_CHAPTERS {
        return Err(invalid(format!(
            "{} chapters exceed the {MAX_CHPL_CHAPTERS} that `chpl` counts. Pass fewer chapters",
            chapters.len()
        )));
    }
    let mut previous: Option<u128> = None;
    for (n, chapter) in chapters.iter().enumerate() {
        if chapter.timescale == 0 {
            return Err(invalid(format!(
                "chapter {n} has timescale 0. Use a positive timescale"
            )));
        }
        if chapter.title.len() > usize::from(u8::MAX) {
            return Err(invalid(format!(
                "chapter {n} has a title of {} bytes, above the 255 that `chpl` holds. Shorten the title",
                chapter.title.len()
            )));
        }
        // Compare starts in nanoseconds, exact for every u32 timescale.
        let start = u128::from(chapter.start) * 1_000_000_000 / u128::from(chapter.timescale);
        if previous.is_some_and(|previous| start <= previous) {
            return Err(invalid(format!(
                "chapter {n} starts at {} / {} s, not after the chapter before it. Pass \
                 chapters in start order, without two at one start",
                chapter.start, chapter.timescale
            )));
        }
        previous = Some(start);
    }
    Ok(())
}

/// `start` ticks of `from` per second, in ticks of `to` per second, rounded
/// down.
pub(crate) fn rescale(start: u64, from: u32, to: u32) -> u64 {
    let scaled = u128::from(start) * u128::from(to) / u128::from(from.max(1));
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

fn hdlr(buf: &mut BoxBuf, handler: &[u8; 4], path: &str) -> Result<()> {
    let start = buf.open_full(b"hdlr", 0, 0);
    buf.u32(0);
    buf.bytes(handler);
    buf.bytes(if handler == b"mdir" { b"appl" } else { &[0; 4] });
    buf.zeros(8);
    buf.u8(0);
    buf.close(start, &format!("{path}/hdlr"))
}

/// One `data` box.
fn data_box(buf: &mut BoxBuf, type_indicator: u32, payload: &[u8], path: &str) -> Result<()> {
    let start = buf.open(b"data");
    buf.u32(type_indicator);
    buf.u32(0);
    buf.bytes(payload);
    buf.close(start, &format!("{path}/data"))
}

/// The body of an `ilst` item: the raw body of an MP4 item, or one `data`
/// box for a Matroska tag.
fn item_body(buf: &mut BoxBuf, item: &MetadataItem, path: &str) -> Result<()> {
    match item.scope {
        MetadataScope::Tag(scope) => {
            data_box(buf, if scope.binary { 0 } else { 1 }, &item.value, path)
        }
        MetadataScope::Udta | MetadataScope::Ilst => {
            buf.bytes(&item.value);
            Ok(())
        }
    }
}

/// Write `udta` (raw items, `meta` with `mdir` items, `chpl`) and the
/// `meta` box of named items, as children of the box being built.
pub(crate) fn write_metadata(
    buf: &mut BoxBuf,
    items: &[MetadataItem],
    chapters: &[Chapter],
    path: &str,
) -> Result<()> {
    let raw: Vec<&MetadataItem> = items
        .iter()
        .filter(|item| item.scope == MetadataScope::Udta)
        .collect();
    let mdir: Vec<&MetadataItem> = items
        .iter()
        .filter(|item| item.scope != MetadataScope::Udta)
        .filter(|item| matches!(item.key, MetadataKey::FourCc(_)))
        .collect();
    let named: Vec<(&[u8], &MetadataItem)> = items
        .iter()
        .filter(|item| item.scope != MetadataScope::Udta)
        .filter_map(|item| match &item.key {
            MetadataKey::Named(name) => Some((name.as_slice(), item)),
            MetadataKey::FourCc(_) => None,
        })
        .collect();

    if !raw.is_empty() || !mdir.is_empty() || !chapters.is_empty() {
        let udta_path = format!("{path}/udta");
        let udta = buf.open(b"udta");
        for item in &raw {
            if let MetadataKey::FourCc(kind) = item.key {
                buf.boxed(&kind.0, &item.value, &format!("{udta_path}/{kind}"))?;
            }
        }
        if !mdir.is_empty() {
            let meta_path = format!("{udta_path}/meta");
            let meta = buf.open_full(b"meta", 0, 0);
            hdlr(buf, b"mdir", &meta_path)?;
            let ilst = buf.open(b"ilst");
            for item in &mdir {
                if let MetadataKey::FourCc(kind) = item.key {
                    let item_path = format!("{meta_path}/ilst/{kind}");
                    let start = buf.open(&kind.0);
                    item_body(buf, item, &item_path)?;
                    buf.close(start, &item_path)?;
                }
            }
            buf.close(ilst, &format!("{meta_path}/ilst"))?;
            buf.close(meta, &meta_path)?;
        }
        if !chapters.is_empty() {
            let chpl = Chpl {
                entries: chapters
                    .iter()
                    .map(|chapter| ChplEntry {
                        start: rescale(chapter.start, chapter.timescale, CHPL_TIMESCALE),
                        title: chapter.title.clone(),
                    })
                    .collect(),
            };
            buf.encode(&chpl, &format!("{udta_path}/chpl"))?;
        }
        buf.close(udta, &udta_path)?;
    }

    if !named.is_empty() {
        let meta_path = format!("{path}/meta");
        let meta = buf.open_full(b"meta", 0, 0);
        hdlr(buf, b"mdta", &meta_path)?;
        let keys = buf.open_full(b"keys", 0, 0);
        buf.u32(named.len() as u32);
        for (name, _) in &named {
            let size = u32::try_from(name.len() + 8).map_err(|_| {
                invalid(format!(
                    "a metadata key of {} bytes is too long for a `keys` entry. Shorten the key",
                    name.len()
                ))
            })?;
            buf.u32(size);
            buf.bytes(b"mdta");
            buf.bytes(name);
        }
        buf.close(keys, &format!("{meta_path}/keys"))?;
        let ilst = buf.open(b"ilst");
        for (n, (_, item)) in named.iter().enumerate() {
            let index = (n as u32 + 1).to_be_bytes();
            let item_path = format!("{meta_path}/ilst[{}]", n + 1);
            let start = buf.open(&index);
            item_body(buf, item, &item_path)?;
            buf.close(start, &item_path)?;
        }
        buf.close(ilst, &format!("{meta_path}/ilst"))?;
        buf.close(meta, &meta_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{MetadataData, TagScope};
    use crate::mp4::boxes::RawBox;
    use crate::mp4::metadata::{parse_meta, parse_udta};
    use crate::track::FourCc;

    const NAM: [u8; 4] = [0xA9, b'n', b'a', b'm'];

    fn children(bytes: &[u8]) -> Vec<RawBox<'_>> {
        crate::mp4::boxes::Children::new(bytes, 0, "test")
            .collect::<Result<_>>()
            .expect("the boxes are well formed")
    }

    #[test]
    fn items_read_back_with_their_scope_and_key() {
        let ilst_body = [
            0, 0, 0, 17, b'd', b'a', b't', b'a', 0, 0, 0, 1, 0, 0, 0, 0, b'T',
        ];
        let items = vec![
            MetadataItem {
                scope: MetadataScope::Udta,
                key: MetadataKey::FourCc(FourCc(*b"cprt")),
                value: vec![1, 2, 3],
                data: Vec::new(),
            },
            MetadataItem {
                scope: MetadataScope::Ilst,
                key: MetadataKey::FourCc(FourCc(NAM)),
                value: ilst_body.to_vec(),
                data: vec![MetadataData {
                    type_indicator: 1,
                    locale: 0,
                    payload: b"T".to_vec(),
                }],
            },
            MetadataItem {
                scope: MetadataScope::Tag(TagScope {
                    target_type_value: 50,
                    target: TagTarget::Segment,
                    binary: false,
                }),
                key: MetadataKey::Named(b"ENCODER".to_vec()),
                value: b"Lavf".to_vec(),
                data: Vec::new(),
            },
        ];
        check_items(&items, Level::Movie).expect("the items fit");
        let mut buf = BoxBuf::new();
        write_metadata(&mut buf, &items, &[], "moov").expect("the items encode");
        let bytes = buf.into_bytes();
        let boxes = children(&bytes);
        assert_eq!(boxes.len(), 2);
        let udta = parse_udta(&boxes[0], "udta").expect("udta reads back");
        assert_eq!(udta.items[0], items[0]);
        assert_eq!(udta.items[1], items[1]);
        let named = parse_meta(&boxes[1], "meta").expect("meta reads back");
        assert_eq!(named[0].key, MetadataKey::Named(b"ENCODER".to_vec()));
        assert_eq!(named[0].text(), Some("Lavf"));
    }

    #[test]
    fn chapters_become_chpl_in_100_ns_ticks() {
        let chapters = [Chapter {
            start: 38_400,
            duration: None,
            timescale: 48_000,
            title: "Closing".to_owned(),
            source: crate::metadata::ChapterSource::Chpl,
        }];
        check_chapters(&chapters).expect("the chapter fits");
        let mut buf = BoxBuf::new();
        write_metadata(&mut buf, &[], &chapters, "moov").expect("the chapter encodes");
        let bytes = buf.into_bytes();
        let udta = parse_udta(&children(&bytes)[0], "udta").expect("udta reads back");
        assert_eq!(udta.chapters[0].start, 8_000_000);
        assert_eq!(udta.chapters[0].title, "Closing");
    }

    #[test]
    fn a_chapter_tag_is_refused_by_name() {
        let item = MetadataItem {
            scope: MetadataScope::Tag(TagScope {
                target_type_value: 30,
                target: TagTarget::Chapter(7),
                binary: false,
            }),
            key: MetadataKey::Named(b"TITLE".to_vec()),
            value: b"x".to_vec(),
            data: Vec::new(),
        };
        assert!(matches!(
            check_items(&[item], Level::Movie),
            Err(Error::Unsupported {
                use_instead: Container::Matroska,
                ..
            })
        ));
    }
}
