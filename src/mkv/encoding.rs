//! `ContentEncoding` decode: the steps that turn stored frames and
//! `CodecPrivate` into plain bytes.
//!
//! Header stripping puts `ContentCompSettings` back in front of the bytes.
//! zlib inflates them. Encodings undo highest `ContentEncodingOrder` first.
//! A track with an encoding that [`ContentEncoding::undone_on_read`] rejects
//! gets no plan, and its bytes stay as stored.

use std::cmp::Reverse;

use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_zlib_with_limit};

use crate::error::{Error, Result};
use crate::matroska::{ContentEncoding, MatroskaTrack};
use crate::packet::Packet;
use crate::track::Track;

/// The largest frame or `CodecPrivate` a decode returns, in bytes. A zlib
/// stream that inflates past it, or a header-stripping prefix that grows a
/// frame past it, is an error.
pub const MAX_DECODED_SIZE: u64 = 64 << 20;

/// `ContentEncodingScope` bit 1: the encoding applies to frames.
const SCOPE_FRAMES: u64 = 1;
/// `ContentEncodingScope` bit 2: the encoding applies to `CodecPrivate`.
const SCOPE_PRIVATE: u64 = 2;

/// One decode step.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// Header stripping: put these bytes back in front.
    Prepend(Vec<u8>),
    /// zlib: inflate.
    Inflate,
}

/// The decode steps of one track, in the order they run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    frames: Vec<Step>,
    private: Vec<Step>,
}

impl Plan {
    /// The plan for `encodings`, or `None` when one of them is an encoding
    /// the demuxer does not undo.
    pub(crate) fn new(encodings: &[ContentEncoding]) -> Option<Self> {
        if !encodings.iter().all(ContentEncoding::undone_on_read) {
            return None;
        }
        let mut ordered: Vec<&ContentEncoding> = encodings.iter().collect();
        // A stable sort keeps file order for equal orders.
        ordered.sort_by_key(|encoding| Reverse(encoding.order));
        let mut plan = Self::default();
        for encoding in ordered {
            let step = match encoding.compression_algorithm {
                Some(0) => Step::Inflate,
                Some(3) => match &encoding.compression_settings {
                    Some(prefix) if !prefix.is_empty() => Step::Prepend(prefix.clone()),
                    _ => continue,
                },
                _ => return None,
            };
            if encoding.scope & SCOPE_FRAMES != 0 {
                plan.frames.push(step.clone());
            }
            if encoding.scope & SCOPE_PRIVATE != 0 {
                plan.private.push(step);
            }
        }
        Some(plan)
    }

    /// Undo the frame encodings on `buf`, the stored bytes of `packet`.
    pub(crate) fn decode_frame(&self, packet: &Packet, buf: &mut Vec<u8>) -> Result<()> {
        let at = Site {
            place: Place::Packet(packet.index),
            offset: packet.offset,
            track: packet.track_id,
        };
        apply(&self.frames, buf, &at, decoded_limit())
    }
}

/// Where the bytes a step decodes come from, for errors.
struct Site {
    place: Place,
    offset: u64,
    track: u32,
}

/// The element a [`Site`] names. A packet read builds no string unless an
/// error needs one.
enum Place {
    Packet(u64),
    Path(String),
}

impl Site {
    fn path(&self) -> String {
        match &self.place {
            Place::Packet(index) => format!("packet {index} of track {}", self.track),
            Place::Path(path) => path.clone(),
        }
    }
}

/// The decode bound as a `usize`.
fn decoded_limit() -> usize {
    usize::try_from(MAX_DECODED_SIZE).unwrap_or(usize::MAX)
}

/// Set `mkv.content_decoded`, and undo the `CodecPrivate` encodings when
/// the demuxer undoes every encoding of the track. `path` and `offset`
/// locate the `CodecPrivate` element of track `track`.
pub(crate) fn decode_track(
    mkv: &mut MatroskaTrack,
    track: u32,
    path: String,
    offset: u64,
) -> Result<()> {
    let Some(plan) = Plan::new(&mkv.content_encodings) else {
        mkv.content_decoded = false;
        return Ok(());
    };
    mkv.content_decoded = true;
    if let Some(private) = mkv.codec_private.as_mut() {
        let at = Site {
            place: Place::Path(path),
            offset,
            track,
        };
        apply(&plan.private, private, &at, decoded_limit())?;
    }
    Ok(())
}

/// The frame plan of every track whose frames carry an encoding the demuxer
/// undoes, by track id.
pub(crate) fn frame_plans(tracks: &[Track]) -> Vec<(u32, Plan)> {
    tracks
        .iter()
        .filter_map(|track| {
            let mkv = track.matroska.as_ref()?;
            if !mkv.content_decoded {
                return None;
            }
            let plan = Plan::new(&mkv.content_encodings)?;
            (!plan.frames.is_empty()).then_some((track.id, plan))
        })
        .collect()
}

/// Run `steps` on `buf`. No result grows past `limit` bytes.
fn apply(steps: &[Step], buf: &mut Vec<u8>, at: &Site, limit: usize) -> Result<()> {
    for step in steps {
        match step {
            Step::Prepend(prefix) => {
                let len = buf.len().saturating_add(prefix.len());
                if len > limit {
                    return Err(Error::mkv_too_large(
                        format!(
                            "{} with the header-stripping prefix of track {}",
                            at.path(),
                            at.track
                        ),
                        u64::try_from(len).unwrap_or(u64::MAX),
                        u64::try_from(limit).unwrap_or(u64::MAX),
                    ));
                }
                let stored = buf.len();
                buf.resize(len, 0);
                buf.copy_within(..stored, prefix.len());
                if let Some(head) = buf.get_mut(..prefix.len()) {
                    head.copy_from_slice(prefix);
                }
            }
            Step::Inflate => *buf = inflate(buf, at, limit)?,
        }
    }
    Ok(())
}

/// Inflate the zlib stream `data`, at most `limit` bytes of output.
fn inflate(data: &[u8], at: &Site, limit: usize) -> Result<Vec<u8>> {
    decompress_to_vec_zlib_with_limit(data, limit).map_err(|err| match err.status {
        TINFLStatus::HasMoreOutput => {
            let limit = u64::try_from(limit).unwrap_or(u64::MAX);
            Error::mkv_too_large(
                format!(
                    "{} inflated by the zlib `ContentEncoding` of track {}",
                    at.path(),
                    at.track
                ),
                limit.saturating_add(1),
                limit,
            )
        }
        status => Error::mkv_malformed(
            at.path(),
            at.offset,
            format!(
                "track {} declares a zlib `ContentEncoding`, and these bytes do not \
                 inflate ({status:?})",
                at.track
            ),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    fn compression(order: u64, scope: u64, algorithm: u64, settings: &[u8]) -> ContentEncoding {
        ContentEncoding {
            order,
            scope,
            encoding_type: 0,
            compression_algorithm: Some(algorithm),
            compression_settings: (!settings.is_empty()).then(|| settings.to_vec()),
            encryption_algorithm: None,
            encryption_settings: None,
        }
    }

    fn site() -> Site {
        Site {
            place: Place::Packet(0),
            offset: 7,
            track: 1,
        }
    }

    #[test]
    fn header_stripping_prepends_the_settings() {
        let plan = Plan::new(&[compression(0, 1, 3, &[0xAB, 0xCD])]).expect("a plan");
        let mut buf = vec![1, 2];
        apply(&plan.frames, &mut buf, &site(), 16).expect("it fits");
        assert_eq!(buf, [0xAB, 0xCD, 1, 2]);
        assert!(plan.private.is_empty());
    }

    #[test]
    fn zlib_inflates_frames_and_codec_private() {
        let plan = Plan::new(&[compression(0, 3, 0, &[])]).expect("a plan");
        let plain = b"plain bytes, plain bytes, plain bytes".to_vec();
        for steps in [&plan.frames, &plan.private] {
            let mut buf = compress_to_vec_zlib(&plain, 6);
            apply(steps, &mut buf, &site(), 1024).expect("it inflates");
            assert_eq!(buf, plain);
        }
    }

    #[test]
    fn the_highest_order_undoes_first() {
        // Order 1 was applied last when writing: zlib over a stripped frame.
        let encodings = [compression(0, 1, 3, &[0xAB]), compression(1, 1, 0, &[])];
        let plan = Plan::new(&encodings).expect("a plan");
        assert_eq!(plan.frames, [Step::Inflate, Step::Prepend(vec![0xAB])]);
        let mut buf = compress_to_vec_zlib(&[1, 2, 3], 6);
        apply(&plan.frames, &mut buf, &site(), 16).expect("it decodes");
        assert_eq!(buf, [0xAB, 1, 2, 3]);
    }

    #[test]
    fn output_past_the_limit_is_too_large() {
        let plan = Plan::new(&[compression(0, 1, 0, &[])]).expect("a plan");
        let mut buf = compress_to_vec_zlib(&[0; 101], 6);
        let err = apply(&plan.frames, &mut buf, &site(), 100).expect_err("past the limit");
        assert!(matches!(err, Error::TooLarge { limit: 100, .. }), "{err}");
        let plan = Plan::new(&[compression(0, 1, 3, &[0xAB])]).expect("a plan");
        let mut buf = vec![0; 100];
        let err = apply(&plan.frames, &mut buf, &site(), 100).expect_err("past the limit");
        assert!(
            matches!(err, Error::TooLarge { declared: 101, .. }),
            "{err}"
        );
    }

    #[test]
    fn a_corrupt_stream_is_malformed_and_names_the_track() {
        let plan = Plan::new(&[compression(0, 1, 0, &[])]).expect("a plan");
        let mut buf = vec![0x78, 0x9C, 0xFF, 0xFF, 0xFF];
        let err = apply(&plan.frames, &mut buf, &site(), 100).expect_err("corrupt");
        assert!(matches!(err, Error::Malformed { offset: 7, .. }), "{err}");
        let text = err.to_string();
        assert!(text.contains("track 1") && text.contains("zlib"), "{text}");
    }

    #[test]
    fn unsupported_encodings_give_no_plan() {
        let encryption = ContentEncoding {
            encoding_type: 1,
            compression_algorithm: None,
            encryption_algorithm: Some(5),
            ..compression(0, 1, 0, &[])
        };
        for encoding in [
            encryption,
            compression(0, 1, 1, &[]),
            compression(0, 1, 2, &[]),
            compression(0, 1, 9, &[]),
            compression(0, 4, 0, &[]),
            compression(0, 5, 3, &[0xAB]),
        ] {
            assert!(!encoding.undone_on_read(), "{}", encoding.label());
            let both = [compression(1, 1, 3, &[0xAB]), encoding];
            assert_eq!(Plan::new(&both), None, "{}", both[1].label());
        }
    }

    #[test]
    fn no_encodings_give_an_empty_plan() {
        assert_eq!(Plan::new(&[]), Some(Plan::default()));
    }
}
