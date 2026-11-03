//! The `Tags` element: one [`MetadataItem`] per `SimpleTag` and target.

use crate::error::{Error, Result};
use crate::metadata::{MetadataItem, MetadataKey, MetadataScope, TagScope, TagTarget};
use crate::mkv::chapters::MAX_NESTING;
use crate::mkv::ebml::{Element, child_path, indexed_path};
use crate::mkv::ids;

/// Matroska's default `TargetTypeValue`: 50, an album, movie or episode.
const DEFAULT_TARGET_TYPE_VALUE: u64 = 50;

/// Parse one `Tags` element body. `index` counts `Tags` elements from 1.
pub(crate) fn parse_tags(tags: &Element<'_>, index: usize) -> Result<Vec<MetadataItem>> {
    let path = indexed_path("Segment", ids::TAGS, index);
    let mut out = Vec::new();
    let mut count = 0usize;
    for tag in tags.children(&path) {
        let tag = tag?;
        if tag.id != ids::TAG {
            continue;
        }
        count += 1;
        let tag_path = indexed_path(&path, ids::TAG, count);
        let mut target_type_value = DEFAULT_TARGET_TYPE_VALUE;
        let mut targets = Vec::new();
        let mut simple = Vec::new();
        for child in tag.children(&tag_path) {
            let child = child?;
            match child.id {
                ids::TARGETS => {
                    let at = child_path(&tag_path, child.id);
                    target_type_value = parse_targets(&child, &at, &mut targets)?;
                }
                ids::SIMPLE_TAG => simple.push(child),
                _ => {}
            }
        }
        if targets.is_empty() {
            targets.push(TagTarget::Segment);
        }
        for (n, simple) in simple.iter().enumerate() {
            let simple_path = indexed_path(&tag_path, ids::SIMPLE_TAG, n + 1);
            let mut values = Vec::new();
            parse_simple(simple, &simple_path, "", 0, &mut values)?;
            for (name, value, binary) in values {
                for &target in &targets {
                    out.push(MetadataItem {
                        scope: MetadataScope::Tag(TagScope {
                            target_type_value,
                            target,
                            binary,
                        }),
                        key: MetadataKey::Named(name.clone().into_bytes()),
                        value: value.clone(),
                        data: Vec::new(),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Collect the UIDs of a `Targets` element. Returns its `TargetTypeValue`.
fn parse_targets(targets: &Element<'_>, path: &str, out: &mut Vec<TagTarget>) -> Result<u64> {
    let mut type_value = DEFAULT_TARGET_TYPE_VALUE;
    for child in targets.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        let target = match child.id {
            ids::TARGET_TYPE_VALUE => {
                type_value = child.uint(&at)?;
                continue;
            }
            ids::TAG_TRACK_UID => TagTarget::Track(child.uint(&at)?),
            ids::TAG_EDITION_UID => TagTarget::Edition(child.uint(&at)?),
            ids::TAG_CHAPTER_UID => TagTarget::Chapter(child.uint(&at)?),
            ids::TAG_ATTACHMENT_UID => TagTarget::Attachment(child.uint(&at)?),
            _ => continue,
        };
        out.push(target);
    }
    Ok(type_value)
}

/// One `SimpleTag` and its nested tags, as (name, value, binary) triples.
/// A nested name joins its parents' names with `/`.
fn parse_simple(
    simple: &Element<'_>,
    path: &str,
    prefix: &str,
    depth: u32,
    out: &mut Vec<(String, Vec<u8>, bool)>,
) -> Result<()> {
    if depth >= MAX_NESTING {
        return Err(Error::mkv_too_large(
            path,
            u64::from(depth) + 1,
            u64::from(MAX_NESTING),
        ));
    }
    let mut name = None;
    let mut value: Option<(Vec<u8>, bool)> = None;
    let mut nested = Vec::new();
    for child in simple.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::TAG_NAME => name = Some(child.string(&at)?),
            ids::TAG_STRING => value = Some((child.string(&at)?.into_bytes(), false)),
            ids::TAG_BINARY => value = Some((child.binary(), true)),
            ids::SIMPLE_TAG => nested.push(child),
            _ => {}
        }
    }
    let name = name.ok_or_else(|| Error::mkv_missing(format!("{path}/TagName")))?;
    let full = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if let Some((bytes, binary)) = value {
        out.push((full.clone(), bytes, binary));
    }
    for (n, child) in nested.iter().enumerate() {
        let child_path = indexed_path(path, ids::SIMPLE_TAG, n + 1);
        parse_simple(child, &child_path, &full, depth + 1, out)?;
    }
    Ok(())
}
