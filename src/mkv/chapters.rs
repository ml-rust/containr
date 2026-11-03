//! The `Chapters` element: editions and their chapter atoms.

use crate::error::{Error, Result};
use crate::metadata::{Chapter, ChapterSource};
use crate::mkv::ebml::{Element, child_path, indexed_path};
use crate::mkv::ids;

/// The deepest nesting of `ChapterAtom` or `SimpleTag` elements read.
pub const MAX_NESTING: u32 = 32;

/// Matroska chapter times count in nanoseconds.
const CHAPTER_TIMESCALE: u32 = 1_000_000_000;

/// Parse the `Chapters` element body. Atoms of every edition come in file
/// order, each nested atom after its parent.
pub(crate) fn parse_chapters(chapters: &Element<'_>) -> Result<Vec<Chapter>> {
    let path = "Segment/Chapters";
    let mut out = Vec::new();
    let mut editions = 0usize;
    for edition in chapters.children(path) {
        let edition = edition?;
        if edition.id != ids::EDITION_ENTRY {
            continue;
        }
        editions += 1;
        let edition_path = indexed_path(path, ids::EDITION_ENTRY, editions);
        let mut edition_uid = None;
        let mut atoms = Vec::new();
        for child in edition.children(&edition_path) {
            let child = child?;
            match child.id {
                ids::EDITION_UID => {
                    edition_uid = Some(child.uint(&child_path(&edition_path, child.id))?);
                }
                ids::CHAPTER_ATOM => atoms.push(child),
                _ => {}
            }
        }
        for (n, atom) in atoms.iter().enumerate() {
            let atom_path = indexed_path(&edition_path, ids::CHAPTER_ATOM, n + 1);
            parse_atom(atom, &atom_path, edition_uid, 0, &mut out)?;
        }
    }
    Ok(out)
}

fn parse_atom(
    atom: &Element<'_>,
    path: &str,
    edition_uid: Option<u64>,
    depth: u32,
    out: &mut Vec<Chapter>,
) -> Result<()> {
    if depth >= MAX_NESTING {
        return Err(Error::mkv_too_large(
            path,
            u64::from(depth) + 1,
            u64::from(MAX_NESTING),
        ));
    }
    let mut uid = None;
    let mut start = None;
    let mut end = None;
    let mut title = None;
    let mut nested = Vec::new();
    for child in atom.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::CHAPTER_UID => uid = Some(child.uint(&at)?),
            ids::CHAPTER_TIME_START => start = Some(child.uint(&at)?),
            ids::CHAPTER_TIME_END => end = Some(child.uint(&at)?),
            ids::CHAPTER_DISPLAY if title.is_none() => {
                for display in child.children(&at) {
                    let display = display?;
                    if display.id == ids::CHAP_STRING {
                        title = Some(display.string(&child_path(&at, display.id))?);
                        break;
                    }
                }
            }
            ids::CHAPTER_ATOM => nested.push(child),
            _ => {}
        }
    }
    let chapter_uid = uid.ok_or_else(|| Error::mkv_missing(format!("{path}/ChapterUID")))?;
    let start = start.ok_or_else(|| Error::mkv_missing(format!("{path}/ChapterTimeStart")))?;
    out.push(Chapter {
        start,
        duration: end.and_then(|end| end.checked_sub(start)),
        timescale: CHAPTER_TIMESCALE,
        title: title.unwrap_or_default(),
        source: ChapterSource::Matroska {
            edition_uid,
            chapter_uid,
            depth,
        },
    });
    for (n, child) in nested.iter().enumerate() {
        let child_path = indexed_path(path, ids::CHAPTER_ATOM, n + 1);
        parse_atom(child, &child_path, edition_uid, depth + 1, out)?;
    }
    Ok(())
}
