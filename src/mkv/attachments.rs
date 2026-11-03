//! The `Attachments` element: attached files located, not read.
//!
//! `FileData` can be large, so the walk reads element headers and the
//! small fields only. The file bytes stay in the input.

use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::mkv::ebml::{Element, Header, child_path, indexed_path};
use crate::mkv::ids;
use crate::mkv::source::Source;

/// The largest attachment name, media type or description read, in bytes.
pub const MAX_ATTACHMENT_FIELD: u64 = 64 * 1024;

/// One `AttachedFile`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Attachment {
    /// `FileUID`. Tags name an attachment by it.
    pub uid: u64,
    /// `FileName`.
    pub name: String,
    /// `FileMediaType`, such as `font/ttf`.
    pub mime: String,
    /// `FileDescription`.
    pub description: Option<String>,
    /// The byte offset of the `FileData` body in the input.
    pub offset: u64,
    /// The `FileData` size in bytes.
    pub size: u64,
}

/// Walk the `Attachments` element at `pos`. `index` counts `Attachments`
/// elements from 1. The element lies inside the input.
pub(crate) fn scan_attachments<R: Read + Seek>(
    src: &mut Source<'_, R>,
    pos: u64,
    header: Header,
    index: usize,
    out: &mut Vec<Attachment>,
) -> Result<()> {
    let path = indexed_path("Segment", ids::ATTACHMENTS, index);
    let body = pos + header.len;
    let end = body.saturating_add(header.size.unwrap_or_default());
    let mut files = 0usize;
    for_each_child(src, body, end, &path, |src, at, child| {
        if child.id != ids::ATTACHED_FILE {
            return Ok(());
        }
        files += 1;
        let file_path = indexed_path(&path, ids::ATTACHED_FILE, files);
        let file = scan_file(src, at, child, &file_path)?;
        out.push(file);
        Ok(())
    })
}

fn scan_file<R: Read + Seek>(
    src: &mut Source<'_, R>,
    pos: u64,
    header: Header,
    path: &str,
) -> Result<Attachment> {
    let body = pos + header.len;
    let end = body.saturating_add(header.size.unwrap_or_default());
    let mut uid = None;
    let mut name = None;
    let mut mime = None;
    let mut description = None;
    let mut data = None;
    for_each_child(src, body, end, path, |src, at, child| {
        let child_body = at + child.len;
        let size = child.size.unwrap_or_default();
        let child_path = child_path(path, child.id);
        if child.id == ids::FILE_DATA {
            data = Some((child_body, size));
            return Ok(());
        }
        if !matches!(
            child.id,
            ids::FILE_UID | ids::FILE_NAME | ids::FILE_MEDIA_TYPE | ids::FILE_DESCRIPTION
        ) {
            return Ok(());
        }
        let bytes = src.read_body(child_body, size, &child_path, MAX_ATTACHMENT_FIELD)?;
        let leaf = Element {
            id: child.id,
            offset: at,
            body_offset: child_body,
            body: &bytes,
        };
        match child.id {
            ids::FILE_UID => uid = Some(leaf.uint(&child_path)?),
            ids::FILE_NAME => name = Some(leaf.string(&child_path)?),
            ids::FILE_MEDIA_TYPE => mime = Some(leaf.string(&child_path)?),
            _ => description = Some(leaf.string(&child_path)?),
        }
        Ok(())
    })?;
    let (offset, size) = data.ok_or_else(|| Error::mkv_missing(format!("{path}/FileData")))?;
    Ok(Attachment {
        uid: uid.ok_or_else(|| Error::mkv_missing(format!("{path}/FileUID")))?,
        name: name.ok_or_else(|| Error::mkv_missing(format!("{path}/FileName")))?,
        mime: mime.ok_or_else(|| Error::mkv_missing(format!("{path}/FileMediaType")))?,
        description,
        offset,
        size,
    })
}

/// Call `visit` with the offset and header of each child in
/// `body..end`. Every child has a known size and ends inside its parent.
fn for_each_child<R: Read + Seek>(
    src: &mut Source<'_, R>,
    body: u64,
    end: u64,
    path: &str,
    mut visit: impl FnMut(&mut Source<'_, R>, u64, Header) -> Result<()>,
) -> Result<()> {
    let mut at = body;
    while at < end {
        let child = src.header_at(at, path)?.ok_or_else(|| {
            Error::mkv_malformed(path, at, "an element header runs past the input end")
        })?;
        let child_path = child_path(path, child.id);
        let size = child.known_size(&child_path, at)?;
        let child_end = (at + child.len).saturating_add(size);
        if child_end > end {
            return Err(Error::mkv_malformed(
                &child_path,
                at,
                format!("the element declares {size} bytes, past its parent's end at {end}"),
            ));
        }
        visit(src, at, child)?;
        at = child_end;
    }
    Ok(())
}
