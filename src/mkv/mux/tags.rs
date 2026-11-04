//! The `Tags` element from shared metadata.
//!
//! Consecutive items of one target share a `Tag`. A name such as `ARTIST/URL`
//! becomes a nested `SimpleTag`, the form the demuxer joins with `/`.

use crate::error::Result;
use crate::metadata::{MetadataItem, MetadataKey, MetadataScope, TagScope, TagTarget};
use crate::mkv::chapters::MAX_NESTING;
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;
use crate::mkv::mux::bytes::invalid;

/// Check that every item is a Matroska tag this muxer can write.
pub(crate) fn check_items(items: &[MetadataItem]) -> Result<()> {
    for item in items {
        let (MetadataScope::Tag(scope), MetadataKey::Named(name)) = (item.scope, &item.key) else {
            return Err(invalid(format!(
                "metadata item {:?} is not a named Matroska tag. Convert MP4 items with \
                 `mkv::tags_from_mp4`",
                item.key
            )));
        };
        let Ok(name) = std::str::from_utf8(name) else {
            return Err(invalid(format!(
                "a tag name is not UTF-8: {name:?}. `TagName` holds UTF-8"
            )));
        };
        if !scope.binary && std::str::from_utf8(&item.value).is_err() {
            return Err(invalid(format!(
                "tag `{name}` is a `TagString` whose value is not UTF-8. Set `TagScope::binary`"
            )));
        }
        let depth = name.split('/').count();
        if depth > MAX_NESTING as usize {
            return Err(invalid(format!(
                "tag `{name}` nests {depth} levels, above the limit of {MAX_NESTING}. Use a \
                 shorter name"
            )));
        }
    }
    Ok(())
}

/// The UIDs tags can target.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Targets<'a> {
    pub tracks: &'a [u64],
    pub editions: &'a [u64],
    pub chapters: &'a [u64],
    pub attachments: &'a [u64],
}

/// Check that each tag names a UID the output holds.
pub(crate) fn check_targets(items: &[MetadataItem], targets: &Targets<'_>) -> Result<()> {
    for item in items {
        let MetadataScope::Tag(scope) = item.scope else {
            continue;
        };
        let known = match scope.target {
            TagTarget::Segment => true,
            TagTarget::Track(uid) => targets.tracks.contains(&uid),
            TagTarget::Edition(uid) => targets.editions.contains(&uid),
            TagTarget::Chapter(uid) => targets.chapters.contains(&uid),
            TagTarget::Attachment(uid) => targets.attachments.contains(&uid),
        };
        if !known {
            return Err(invalid(format!(
                "tag `{}` targets {:?}, which the output does not hold. Add it, or drop the tag",
                item.name().unwrap_or("?"),
                scope.target
            )));
        }
    }
    Ok(())
}

/// One `SimpleTag` and its nested tags.
#[derive(Debug)]
struct Node<'a> {
    name: &'a str,
    value: Option<(&'a [u8], bool)>,
    children: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    fn new(name: &'a str) -> Self {
        Self {
            name,
            value: None,
            children: Vec::new(),
        }
    }
}

/// Add a value at `path`, under the last node of each name. A node already
/// holding a value or children takes a new sibling, so read-back order holds.
fn insert<'a>(list: &mut Vec<Node<'a>>, path: &[&'a str], value: (&'a [u8], bool)) {
    let Some((&first, rest)) = path.split_first() else {
        return;
    };
    if rest.is_empty() {
        match list.last_mut() {
            Some(node)
                if node.name == first && node.value.is_none() && node.children.is_empty() =>
            {
                node.value = Some(value);
            }
            _ => {
                let mut node = Node::new(first);
                node.value = Some(value);
                list.push(node);
            }
        }
        return;
    }
    if list.last().is_none_or(|node| node.name != first) {
        list.push(Node::new(first));
    }
    if let Some(node) = list.last_mut() {
        insert(&mut node.children, rest, value);
    }
}

fn node_body(node: &Node<'_>) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.text(ids::TAG_NAME, node.name)?;
    match node.value {
        Some((value, true)) => out.binary(ids::TAG_BINARY, value)?,
        Some((value, false)) => out.binary(ids::TAG_STRING, value)?,
        None => {}
    }
    for child in &node.children {
        out.master(ids::SIMPLE_TAG, &node_body(child)?)?;
    }
    Ok(out)
}

fn targets_body(scope: TagScope) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::TARGET_TYPE_VALUE, scope.target_type_value)?;
    match scope.target {
        TagTarget::Segment => {}
        TagTarget::Track(uid) => out.uint(ids::TAG_TRACK_UID, uid)?,
        TagTarget::Edition(uid) => out.uint(ids::TAG_EDITION_UID, uid)?,
        TagTarget::Chapter(uid) => out.uint(ids::TAG_CHAPTER_UID, uid)?,
        TagTarget::Attachment(uid) => out.uint(ids::TAG_ATTACHMENT_UID, uid)?,
    }
    Ok(out)
}

/// Append one `Tag` per run of items with one target. `check_items` has
/// checked the names and scopes.
pub(crate) fn append_tags(body: &mut ElementBuf, items: &[MetadataItem]) -> Result<()> {
    let same_target = |a: &MetadataItem, b: &MetadataItem| match (a.scope, b.scope) {
        (MetadataScope::Tag(a), MetadataScope::Tag(b)) => {
            a.target == b.target && a.target_type_value == b.target_type_value
        }
        _ => false,
    };
    for run in items.chunk_by(same_target) {
        let Some(MetadataScope::Tag(scope)) = run.first().map(|item| item.scope) else {
            continue;
        };
        let mut nodes = Vec::new();
        for item in run {
            let (Some(name), MetadataScope::Tag(item_scope)) = (item.name(), item.scope) else {
                continue;
            };
            let path: Vec<&str> = name.split('/').collect();
            insert(
                &mut nodes,
                &path,
                (item.value.as_slice(), item_scope.binary),
            );
        }
        let mut tag = ElementBuf::new();
        tag.master(ids::TARGETS, &targets_body(scope)?)?;
        for node in &nodes {
            tag.master(ids::SIMPLE_TAG, &node_body(node)?)?;
        }
        body.master(ids::TAG, &tag)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mkv::ebml::Element;
    use crate::mkv::tags::parse_tags;

    fn item(name: &str, value: &[u8], target: TagTarget, binary: bool) -> MetadataItem {
        MetadataItem {
            scope: MetadataScope::Tag(TagScope {
                target_type_value: 50,
                target,
                binary,
            }),
            key: MetadataKey::Named(name.as_bytes().to_vec()),
            value: value.to_vec(),
            data: Vec::new(),
        }
    }

    #[test]
    fn items_read_back_in_order_with_nesting() {
        let items = vec![
            item("A", b"1", TagTarget::Segment, false),
            item("A/B", b"2", TagTarget::Segment, false),
            item("A/B/C", b"3", TagTarget::Segment, false),
            item("D/E", b"4", TagTarget::Segment, false),
            item("A", b"5", TagTarget::Segment, false),
            item("ROLE", &[1, 2], TagTarget::Attachment(9), true),
            item("ENCODER", b"e", TagTarget::Track(3), false),
        ];
        check_items(&items).expect("the items are tags");
        let mut body = ElementBuf::new();
        append_tags(&mut body, &items).expect("the items fit");
        let bytes = body.into_bytes();
        let element = Element {
            id: ids::TAGS,
            offset: 0,
            body_offset: 0,
            body: &bytes,
        };
        assert_eq!(parse_tags(&element, 1).expect("the tags read back"), items);
    }

    #[test]
    fn items_that_are_not_tags_are_refused() {
        let mut other = item("A", b"1", TagTarget::Segment, false);
        other.scope = MetadataScope::Ilst;
        assert!(check_items(&[other]).is_err());
        let bad = item("A", &[0xFF], TagTarget::Segment, false);
        assert!(check_items(&[bad]).is_err());
        let deep = item(&"a/".repeat(40), b"1", TagTarget::Segment, false);
        assert!(check_items(&[deep]).is_err());
    }

    #[test]
    fn a_tag_must_name_a_uid_the_output_holds() {
        let targets = Targets {
            tracks: &[3],
            editions: &[],
            chapters: &[],
            attachments: &[],
        };
        let good = item("A", b"1", TagTarget::Track(3), false);
        assert!(check_targets(&[good], &targets).is_ok());
        let bad = item("A", b"1", TagTarget::Chapter(4), false);
        assert!(check_targets(&[bad], &targets).is_err());
    }
}
