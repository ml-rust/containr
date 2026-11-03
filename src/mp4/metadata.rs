//! `udta`, `meta`, `ilst` and `chpl`, kept raw.

use mp4_atom::Chpl;

use crate::error::{Error, Result};
use crate::metadata::{
    Chapter, ChapterSource, MetadataData, MetadataItem, MetadataKey, MetadataScope,
};
use crate::mp4::boxes::{RawBox, collect_children};

/// `chpl` start times count in 100-nanosecond ticks.
pub(crate) const CHPL_TIMESCALE: u32 = 10_000_000;

/// Metadata and `chpl` chapters from one `udta` box.
#[derive(Debug, Default)]
pub(crate) struct Udta {
    pub items: Vec<MetadataItem>,
    pub chapters: Vec<Chapter>,
}

/// Parse a `udta` box. `meta` children yield their `ilst` items, `chpl`
/// yields chapters, and every other child is one raw item.
pub(crate) fn parse_udta(udta: &RawBox<'_>, path: &str) -> Result<Udta> {
    let mut out = Udta::default();
    for child in collect_children(udta, 0, path)? {
        let child_path = format!("{path}/{}", child.kind);
        match &child.kind.0 {
            b"meta" => out.items.extend(parse_meta(&child, &child_path)?),
            b"chpl" => {
                let chpl = child.decode::<Chpl>(&child_path)?;
                out.chapters
                    .extend(chpl.entries.into_iter().map(|entry| Chapter {
                        start: entry.start,
                        duration: None,
                        timescale: CHPL_TIMESCALE,
                        title: entry.title,
                        source: ChapterSource::Chpl,
                    }));
            }
            _ => out.items.push(MetadataItem {
                scope: MetadataScope::Udta,
                key: MetadataKey::FourCc(child.kind),
                value: child.body.to_vec(),
                data: Vec::new(),
            }),
        }
    }
    Ok(out)
}

/// Parse a `meta` box into its `ilst` items.
///
/// ISO `meta` is a full box: version and flags precede the children.
/// QuickTime `meta` is not, and its first child is `hdlr`. The body tells
/// them apart.
pub(crate) fn parse_meta(meta: &RawBox<'_>, path: &str) -> Result<Vec<MetadataItem>> {
    let skip = if meta.body.get(4..8) == Some(b"hdlr".as_slice()) {
        0
    } else {
        4
    };
    let children = collect_children(meta, skip, path)?;
    let mut keys = Vec::new();
    for child in &children {
        if &child.kind.0 == b"keys" {
            keys = parse_keys(child, &format!("{path}/keys"))?;
        }
    }
    let mut items = Vec::new();
    for child in &children {
        if &child.kind.0 == b"ilst" {
            let ilst_path = format!("{path}/ilst");
            for item in collect_children(child, 0, &ilst_path)? {
                items.push(parse_item(
                    &item,
                    &format!("{ilst_path}/{}", item.kind),
                    &keys,
                )?);
            }
        }
    }
    Ok(items)
}

/// Parse a QuickTime `keys` box into key names, namespace excluded.
fn parse_keys(keys: &RawBox<'_>, path: &str) -> Result<Vec<Vec<u8>>> {
    let mut reader = keys.reader(path);
    reader.skip(4, "version and flags")?;
    let count = reader.u32("entry_count")?;
    let mut names = Vec::new();
    for i in 0..count {
        let field = format!("key {i}");
        let size = reader.u32(&field)?;
        let name_len = (size as usize).checked_sub(8).ok_or_else(|| {
            Error::mp4_malformed(
                path,
                keys.offset,
                format!("key {i} declares size {size}, below the 8-byte minimum"),
            )
        })?;
        reader.skip(4, &field)?;
        names.push(reader.bytes(name_len, &field)?.to_vec());
    }
    Ok(names)
}

fn parse_item(item: &RawBox<'_>, path: &str, keys: &[Vec<u8>]) -> Result<MetadataItem> {
    let index = u32::from_be_bytes(item.kind.0) as usize;
    let key = match index.checked_sub(1).and_then(|i| keys.get(i)) {
        Some(name) => MetadataKey::Named(name.clone()),
        None => MetadataKey::FourCc(item.kind),
    };
    let mut data = Vec::new();
    for child in collect_children(item, 0, path)? {
        if &child.kind.0 == b"data" {
            let data_path = format!("{path}/data");
            let mut reader = child.reader(&data_path);
            let type_indicator = reader.u32("type indicator")?;
            let locale = reader.u32("locale")?;
            data.push(MetadataData {
                type_indicator,
                locale,
                payload: reader.rest().to_vec(),
            });
        }
    }
    Ok(MetadataItem {
        scope: MetadataScope::Ilst,
        key,
        value: item.body.to_vec(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::FourCc;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn title_item(kind: &[u8; 4], text: &[u8]) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
        data.extend_from_slice(text);
        boxed(kind, &boxed(b"data", &data))
    }

    #[test]
    fn iso_meta_yields_ilst_items() {
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(boxed(b"hdlr", &[0; 25]));
        meta.extend(boxed(
            b"ilst",
            &title_item(&[0xA9, b'n', b'a', b'm'], b"Title"),
        ));
        let bytes = boxed(b"meta", &meta);
        let raw = RawBox::whole(&bytes, 0, "meta").expect("the box is well formed");
        let items = parse_meta(&raw, "meta").expect("the box is well formed");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].key,
            MetadataKey::FourCc(FourCc([0xA9, b'n', b'a', b'm']))
        );
        assert_eq!(items[0].text(), Some("Title"));
    }

    #[test]
    fn quicktime_meta_resolves_keys() {
        let mut keys = vec![0, 0, 0, 0, 0, 0, 0, 1];
        keys.extend(boxed(b"mdta", b"com.apple.quicktime.title"));
        let mut meta = boxed(b"hdlr", &[0; 25]);
        meta.extend(boxed(b"keys", &keys));
        meta.extend(boxed(b"ilst", &title_item(&[0, 0, 0, 1], b"Clip")));
        let bytes = boxed(b"meta", &meta);
        let raw = RawBox::whole(&bytes, 0, "meta").expect("the box is well formed");
        let items = parse_meta(&raw, "meta").expect("the box is well formed");
        assert_eq!(
            items[0].key,
            MetadataKey::Named(b"com.apple.quicktime.title".to_vec())
        );
        assert_eq!(items[0].text(), Some("Clip"));
    }

    #[test]
    fn udta_keeps_unknown_children_raw() {
        let bytes = boxed(
            b"udta",
            &boxed(
                &[0xA9, b'n', b'a', b'm'],
                &[0, 4, 0x55, 0xC4, b'T', b'e', b's', b't'],
            ),
        );
        let raw = RawBox::whole(&bytes, 0, "udta").expect("the box is well formed");
        let udta = parse_udta(&raw, "udta").expect("the box is well formed");
        assert_eq!(udta.items[0].scope, MetadataScope::Udta);
        assert_eq!(
            udta.items[0].value,
            vec![0, 4, 0x55, 0xC4, b'T', b'e', b's', b't']
        );
    }

    #[test]
    fn keys_rejects_a_short_entry() {
        let bytes = boxed(
            b"keys",
            &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, b'm', b'd', b't', b'a'],
        );
        let raw = RawBox::whole(&bytes, 0, "keys").expect("the box is well formed");
        assert!(parse_keys(&raw, "keys").is_err());
    }
}
