//! Checks a track spec passes before the muxer takes it.

use crate::error::{Container, Error, Result};
use crate::metadata::{MetadataItem, MetadataScope, TagScope, TagTarget};
use crate::mkv::mux::bytes::invalid;
use crate::mkv::mux::options::{DocType, webm_allows};
use crate::mkv::mux::spec::TrackSpec;

/// Check a track spec against what the output holds.
pub(crate) fn check_spec(spec: &TrackSpec, doc_type: DocType) -> Result<()> {
    let mkv = &spec.matroska;
    if spec.timebase.num == 0 || spec.timebase.den == 0 {
        return Err(invalid(format!(
            "`TrackSpec::timebase` is {}/{}. Both parts must be above 0",
            spec.timebase.num, spec.timebase.den
        )));
    }
    if mkv.codec_id.is_empty() || !mkv.codec_id.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid(format!(
            "`CodecID` {:?} is not printable ASCII. Use a Matroska codec ID, such as `V_VP9`",
            mkv.codec_id
        )));
    }
    if mkv.track_type == 0 || mkv.track_type > 254 {
        return Err(invalid(format!(
            "`TrackType` is {}. Use 1 to 254",
            mkv.track_type
        )));
    }
    if mkv.track_type == 1
        && mkv
            .video
            .as_ref()
            .is_none_or(|video| video.pixel_width == 0 || video.pixel_height == 0)
    {
        return Err(invalid(format!(
            "video track with codec `{}` has no `PixelWidth` and `PixelHeight`. Set \
             `MatroskaTrack::video`",
            mkv.codec_id
        )));
    }
    if mkv.default_duration == Some(0) {
        return Err(invalid(format!(
            "track with codec `{}` has `DefaultDuration` 0. Use `None`, or a positive value",
            mkv.codec_id
        )));
    }
    if doc_type == DocType::WebM && !webm_allows(&mkv.codec_id) {
        return Err(Error::Unsupported {
            container: Container::WebM,
            what: format!(
                "codec `{}`: WebM holds VP8, VP9, AV1, Opus, Vorbis and WebVTT only",
                mkv.codec_id
            ),
            use_instead: Container::Matroska,
        });
    }
    Ok(())
}

/// Point a track's tags at its `TrackUID`.
pub(crate) fn retarget(items: &mut [MetadataItem], uid: u64) -> Result<()> {
    for item in items {
        let MetadataScope::Tag(TagScope { target, .. }) = &mut item.scope else {
            continue;
        };
        match *target {
            TagTarget::Segment | TagTarget::Track(_) => *target = TagTarget::Track(uid),
            other => {
                return Err(invalid(format!(
                    "a track tag targets {other:?}. Pass it to `Muxer::set_metadata`"
                )));
            }
        }
    }
    Ok(())
}
