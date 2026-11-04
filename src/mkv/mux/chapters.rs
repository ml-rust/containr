//! The `Chapters` element from shared chapters.
//!
//! Chapters come flat in depth-first order, as the demuxer lists them. Atoms
//! nest back from their depth, and a new edition starts wherever the edition
//! UID changes. Every atom has a `ChapterUID`: its own, else a derived one. A
//! chapter time must be a whole number of nanoseconds, or it is refused.

use std::collections::HashSet;

use crate::error::Result;
use crate::metadata::{Chapter, ChapterSource};
use crate::mkv::chapters::MAX_NESTING;
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;
use crate::mkv::mux::bytes::{derive_uid, invalid, refuse};
use crate::mkv::mux::timing::nanos_of;
use crate::track::Timebase;

/// `ChapLanguage`: the shared chapter type holds no language.
const CHAPTER_LANGUAGE: &str = "und";

/// One `ChapterAtom`.
#[derive(Debug)]
pub(crate) struct Atom {
    uid: u64,
    start: u64,
    end: Option<u64>,
    title: String,
    children: Vec<Atom>,
}

/// One `EditionEntry`.
#[derive(Debug)]
pub(crate) struct Edition {
    pub uid: Option<u64>,
    atoms: Vec<Atom>,
}

/// Chapters ready to write.
#[derive(Debug, Default)]
pub(crate) struct ChapterPlan {
    pub editions: Vec<Edition>,
    /// Every `ChapterUID`, for tag targets.
    pub uids: Vec<u64>,
}

/// A chapter time in nanoseconds.
fn nanos(value: u64, timescale: u32, n: usize) -> Result<u64> {
    if timescale == 0 {
        return Err(invalid(format!(
            "chapter {n} has timescale 0. Use a positive timescale"
        )));
    }
    nanos_of(i128::from(value), Timebase::from_timescale(timescale))
        .and_then(|nanos| u64::try_from(nanos).ok())
        .ok_or_else(|| {
            refuse(format!(
                "chapter {n}: {value} ticks at {timescale} ticks per second is no whole number \
                 of nanoseconds below 2^64, and a Matroska chapter time counts them"
            ))
        })
}

/// Place `atom` under the last atom `depth` levels down.
fn place(list: &mut Vec<Atom>, depth: u32, atom: Atom, n: usize) -> Result<()> {
    if depth == 0 {
        list.push(atom);
        return Ok(());
    }
    let Some(parent) = list.last_mut() else {
        return Err(invalid(format!(
            "chapter {n} sits at a depth with no parent before it. List nested chapters after \
             their parent, depth first"
        )));
    };
    place(&mut parent.children, depth - 1, atom, n)
}

/// Check `chapters` and arrange them into editions.
pub(crate) fn plan_chapters(chapters: &[Chapter]) -> Result<ChapterPlan> {
    let mut plan = ChapterPlan::default();
    let given: HashSet<u64> = chapters
        .iter()
        .filter_map(|chapter| match chapter.source {
            ChapterSource::Matroska { chapter_uid, .. } if chapter_uid != 0 => Some(chapter_uid),
            _ => None,
        })
        .collect();
    let mut derived: HashSet<u64> = HashSet::new();
    for (n, chapter) in chapters.iter().enumerate() {
        let (edition_uid, own_uid, depth) = match chapter.source {
            ChapterSource::Matroska {
                edition_uid,
                chapter_uid,
                depth,
            } => (
                edition_uid,
                Some(chapter_uid).filter(|&uid| uid != 0),
                depth,
            ),
            ChapterSource::Chpl | ChapterSource::Track(_) => (None, None, 0),
        };
        if depth >= MAX_NESTING {
            return Err(invalid(format!(
                "chapter {n} sits at depth {depth}, above the limit of {MAX_NESTING}. Flatten \
                 the chapters"
            )));
        }
        let uid = own_uid.unwrap_or_else(|| {
            let uid = derive_uid(chapter.title.as_bytes(), n as u64, |uid| {
                given.contains(&uid) || derived.contains(&uid)
            });
            derived.insert(uid);
            uid
        });
        let start = nanos(chapter.start, chapter.timescale, n)?;
        let end = match chapter.duration {
            Some(duration) => {
                let end = chapter.start.checked_add(duration).ok_or_else(|| {
                    invalid(format!(
                        "chapter {n}: start {} plus duration {duration} overflows 64 bits. Use \
                         a shorter chapter",
                        chapter.start
                    ))
                })?;
                Some(nanos(end, chapter.timescale, n)?)
            }
            None => None,
        };
        let atom = Atom {
            uid,
            start,
            end,
            title: chapter.title.clone(),
            children: Vec::new(),
        };
        let starts_edition = plan
            .editions
            .last()
            .is_none_or(|edition| edition.uid != edition_uid);
        if starts_edition {
            plan.editions.push(Edition {
                uid: edition_uid,
                atoms: Vec::new(),
            });
        }
        if let Some(edition) = plan.editions.last_mut() {
            place(&mut edition.atoms, depth, atom, n)?;
        }
        plan.uids.push(uid);
    }
    Ok(plan)
}

fn atom_body(atom: &Atom) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::CHAPTER_UID, atom.uid)?;
    out.uint(ids::CHAPTER_TIME_START, atom.start)?;
    if let Some(end) = atom.end {
        out.uint(ids::CHAPTER_TIME_END, end)?;
    }
    let mut display = ElementBuf::new();
    display.text(ids::CHAP_STRING, &atom.title)?;
    display.text(ids::CHAP_LANGUAGE, CHAPTER_LANGUAGE)?;
    out.master(ids::CHAPTER_DISPLAY, &display)?;
    for child in &atom.children {
        out.master(ids::CHAPTER_ATOM, &atom_body(child)?)?;
    }
    Ok(out)
}

/// The `Chapters` body.
pub(crate) fn chapters_body(plan: &ChapterPlan) -> Result<ElementBuf> {
    let mut body = ElementBuf::new();
    for edition in &plan.editions {
        let mut entry = ElementBuf::new();
        if let Some(uid) = edition.uid {
            entry.uint(ids::EDITION_UID, uid)?;
        }
        for atom in &edition.atoms {
            entry.master(ids::CHAPTER_ATOM, &atom_body(atom)?)?;
        }
        body.master(ids::EDITION_ENTRY, &entry)?;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mkv::chapters::parse_chapters;
    use crate::mkv::ebml::Element;

    fn matroska(uid: u64, depth: u32, edition_uid: Option<u64>) -> Chapter {
        Chapter {
            start: uid * 1000,
            duration: Some(500),
            timescale: 1_000_000_000,
            title: format!("chapter {uid}"),
            source: ChapterSource::Matroska {
                edition_uid,
                chapter_uid: uid,
                depth,
            },
        }
    }

    fn read_back(plan: &ChapterPlan) -> Vec<Chapter> {
        let bytes = chapters_body(plan).expect("the chapters fit").into_bytes();
        let element = Element {
            id: ids::CHAPTERS,
            offset: 0,
            body_offset: 0,
            body: &bytes,
        };
        parse_chapters(&element).expect("the chapters read back")
    }

    #[test]
    fn nesting_and_editions_read_back() {
        let chapters = vec![
            matroska(1, 0, Some(7)),
            matroska(2, 1, Some(7)),
            matroska(3, 2, Some(7)),
            matroska(4, 0, Some(7)),
            matroska(5, 0, Some(8)),
            matroska(6, 0, None),
        ];
        let plan = plan_chapters(&chapters).expect("the chapters nest");
        assert_eq!(plan.editions.len(), 3);
        assert_eq!(read_back(&plan), chapters);
    }

    #[test]
    fn chapters_from_mp4_get_uids_and_nanoseconds() {
        let chapters = vec![Chapter {
            start: 800,
            duration: Some(1200),
            timescale: 1000,
            title: "Closing".to_owned(),
            source: ChapterSource::Track(3),
        }];
        let plan = plan_chapters(&chapters).expect("the chapter maps");
        assert_ne!(plan.uids[0], 0);
        let read = read_back(&plan);
        assert_eq!(read[0].start, 800_000_000);
        assert_eq!(read[0].duration, Some(1_200_000_000));
        assert_eq!(read[0].title, "Closing");
    }

    #[test]
    fn bad_chapters_are_refused() {
        let mut orphan = matroska(1, 1, None);
        assert!(plan_chapters(std::slice::from_ref(&orphan)).is_err());
        orphan.source = ChapterSource::Chpl;
        orphan.timescale = 0;
        assert!(plan_chapters(std::slice::from_ref(&orphan)).is_err());
        orphan.timescale = 48_000;
        orphan.start = 1;
        assert!(plan_chapters(&[orphan]).is_err());
    }
}
