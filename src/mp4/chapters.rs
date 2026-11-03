//! QuickTime chapter tracks: text samples named by a `tref` `chap`
//! reference.

use std::io::{Read, Seek};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::metadata::{Chapter, ChapterSource};
use crate::mp4::demuxer::read_range;
use crate::mp4::tables::{SampleCursor, SampleTables};
use crate::track::Track;

/// The largest chapter text sample read, in bytes.
pub const MAX_CHAPTER_SAMPLE: u32 = 64 * 1024;
/// The most chapters read from one chapter track.
pub const MAX_CHAPTERS: u64 = 65_536;

/// Read the chapters of one chapter track.
///
/// Each sample holds a 16-bit big-endian text length, then the text. The
/// text is UTF-8, or UTF-16 when it starts with a byte order mark.
pub(crate) fn read_chapter_track<R: Read + Seek>(
    reader: &mut R,
    input_len: u64,
    track: &Track,
    tables: &Arc<SampleTables>,
) -> Result<Vec<Chapter>> {
    if track.sample_count > MAX_CHAPTERS {
        return Err(Error::too_large(
            format!("{}/stsz", tables.path),
            track.sample_count,
            MAX_CHAPTERS,
        ));
    }
    let mut cursor = SampleCursor::new(Arc::clone(tables));
    let mut chapters = Vec::new();
    let mut buf = Vec::new();
    while let Some(packet) = cursor.next_packet()? {
        if packet.size > MAX_CHAPTER_SAMPLE {
            return Err(Error::too_large(
                format!("{} sample {}", tables.path, packet.index),
                u64::from(packet.size),
                u64::from(MAX_CHAPTER_SAMPLE),
            ));
        }
        read_range(reader, input_len, &packet, &mut buf)?;
        let start = u64::try_from(packet.pts).map_err(|_| {
            Error::inconsistent(
                format!("{}/ctts", tables.path),
                format!(
                    "chapter sample {} presents at {}, before 0",
                    packet.index, packet.pts
                ),
            )
        })?;
        chapters.push(Chapter {
            start,
            duration: Some(u64::from(packet.duration)),
            timescale: track.timescale,
            title: chapter_title(&buf),
            source: ChapterSource::Track(track.id),
        });
    }
    Ok(chapters)
}

/// Decode the title of one text sample. A length past the sample end keeps
/// the bytes the sample holds.
fn chapter_title(sample: &[u8]) -> String {
    let Some(len) = sample
        .get(..2)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
    else {
        return String::new();
    };
    let text = sample.get(2..).unwrap_or_default();
    let text = text.get(..len).unwrap_or(text);
    match text {
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        _ => String::from_utf8_lossy(text).into_owned(),
    }
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| unit(*pair))
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_title_reads_utf8_and_utf16() {
        assert_eq!(
            chapter_title(&[0, 5, b'O', b'p', b'e', b'n', b's']),
            "Opens"
        );
        assert_eq!(chapter_title(&[0, 6, 0xFE, 0xFF, 0, b'H', 0, b'i']), "Hi");
        assert_eq!(chapter_title(&[0, 6, 0xFF, 0xFE, b'H', 0, b'i', 0]), "Hi");
    }

    #[test]
    fn chapter_title_tolerates_short_samples() {
        assert_eq!(chapter_title(&[]), "");
        assert_eq!(chapter_title(&[0, 9, b'a']), "a");
    }
}
