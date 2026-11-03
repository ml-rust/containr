//! The EBML header and the Segment `Info` element.

use crate::error::{Error, Result};
use crate::mkv::ebml::{Element, MAX_ID_LEN, MAX_SIZE_LEN, child_path};
use crate::mkv::ids;

/// The Matroska default `TimestampScale`: 1 000 000 ns, a millisecond.
pub const DEFAULT_TIMESTAMP_SCALE: u64 = 1_000_000;

/// Facts from the EBML header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EbmlHeader {
    /// `DocType`: `matroska` or `webm`.
    pub doc_type: String,
    /// `DocTypeVersion`.
    pub doc_type_version: u64,
    /// `DocTypeReadVersion`.
    pub doc_type_read_version: u64,
}

/// Facts from the Segment `Info` element.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentInfo {
    /// `TimestampScale`: nanoseconds per tick. Every Cluster and block
    /// timestamp counts in it.
    pub timestamp_scale: u64,
    /// `Duration` in ticks, as the float the file stores. The demuxer
    /// derives no timestamp from it.
    pub duration: Option<f64>,
    /// `Title`.
    pub title: Option<String>,
    /// `MuxingApp`: the library that wrote the file.
    pub muxing_app: Option<String>,
    /// `WritingApp`: the application that wrote the file.
    pub writing_app: Option<String>,
    /// `DateUTC`: nanoseconds since 2001-01-01T00:00:00 UTC.
    pub date_utc: Option<i64>,
    /// `SegmentUUID`.
    pub segment_uuid: Option<Vec<u8>>,
}

impl Default for SegmentInfo {
    /// The facts of an `Info` element with no children.
    fn default() -> Self {
        Self {
            timestamp_scale: DEFAULT_TIMESTAMP_SCALE,
            duration: None,
            title: None,
            muxing_app: None,
            writing_app: None,
            date_utc: None,
            segment_uuid: None,
        }
    }
}

/// Parse the EBML header body.
pub(crate) fn parse_ebml_header(ebml: &Element<'_>) -> Result<EbmlHeader> {
    let path = "EBML";
    let mut out = EbmlHeader {
        doc_type: "matroska".to_owned(),
        doc_type_version: 1,
        doc_type_read_version: 1,
    };
    for child in ebml.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::DOC_TYPE => out.doc_type = child.string(&at)?,
            ids::DOC_TYPE_VERSION => out.doc_type_version = child.uint(&at)?,
            ids::DOC_TYPE_READ_VERSION => out.doc_type_read_version = child.uint(&at)?,
            ids::EBML_MAX_ID_LENGTH => check_length(&child, &at, MAX_ID_LEN)?,
            ids::EBML_MAX_SIZE_LENGTH => check_length(&child, &at, MAX_SIZE_LEN)?,
            _ => {}
        }
    }
    if out.doc_type != "matroska" && out.doc_type != "webm" {
        return Err(Error::mkv_malformed(
            "EBML/DocType",
            ebml.offset,
            format!(
                "the DocType is {:?}, not \"matroska\" or \"webm\"",
                out.doc_type
            ),
        ));
    }
    Ok(out)
}

/// Refuse an `EBMLMaxIDLength` or `EBMLMaxSizeLength` above what this
/// reader decodes.
fn check_length(child: &Element<'_>, path: &str, limit: usize) -> Result<()> {
    let value = child.uint(path)?;
    if value > limit as u64 {
        return Err(Error::mkv_malformed(
            path,
            child.offset,
            format!("the file allows {value}-byte values; this reader decodes at most {limit}"),
        ));
    }
    Ok(())
}

/// Parse the `Info` element body.
pub(crate) fn parse_info(info: &Element<'_>) -> Result<SegmentInfo> {
    let path = "Segment/Info";
    let mut out = SegmentInfo::default();
    for child in info.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::TIMESTAMP_SCALE => out.timestamp_scale = child.uint(&at)?,
            ids::DURATION => out.duration = Some(child.float(&at)?),
            ids::TITLE => out.title = Some(child.string(&at)?),
            ids::MUXING_APP => out.muxing_app = Some(child.string(&at)?),
            ids::WRITING_APP => out.writing_app = Some(child.string(&at)?),
            ids::DATE_UTC => out.date_utc = Some(child.int(&at)?),
            ids::SEGMENT_UUID => out.segment_uuid = Some(child.binary()),
            _ => {}
        }
    }
    if out.timestamp_scale == 0 || out.timestamp_scale > u64::from(u32::MAX) {
        return Err(Error::mkv_malformed(
            "Segment/Info/TimestampScale",
            info.offset,
            format!(
                "the TimestampScale is {} ns; this demuxer accepts 1 to {}",
                out.timestamp_scale,
                u32::MAX
            ),
        ));
    }
    Ok(out)
}
