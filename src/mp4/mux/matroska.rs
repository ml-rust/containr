//! A Matroska track as an MP4 track spec.
//!
//! H.264, HEVC and AV1 keep `CodecPrivate` as the `avcC`, `hvcC` or `av1C`
//! body. AAC wraps it in `esds`. Opus converts `OpusHead` to `dOps`.
//! `S_TEXT/UTF8` cues become `tx3g` samples. Every other codec is refused
//! and the error names Matroska.
//!
//! The media timescale keeps the Matroska tick exact: `TimestampScale`
//! reduced over one second. When the reduced tick is not one over an
//! integer, timestamps scale up by a whole factor.

use crate::error::{Container, Error, Result};
use crate::mac_language::terminology;
use crate::matroska::{DisplayUnit, MatroskaAudio, MatroskaTrack, MatroskaVideo};
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::codec_config::{aac_esds, opus_dops, rotation_matrix};
use crate::mp4::mux::spec::{AudioCodec, PacketFormat, SampleEntrySpec, TrackSpec, VisualCodec};
use crate::track::{Language, PixelAspect, Timebase, Track, TrackKind};

fn refuse(what: String) -> Error {
    Error::Unsupported {
        container: Container::Mp4,
        what,
        use_instead: Container::Matroska,
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The MP4 timescale for a Matroska timebase, and the whole factor that
/// turns a Matroska tick into MP4 ticks.
pub(crate) fn timescale_of(timebase: Timebase) -> Result<(u32, u64)> {
    if timebase.num == 0 || timebase.den == 0 {
        return Err(invalid(format!(
            "the track timebase is {}/{} seconds. Both parts must be above 0. Fix the track's `TimestampScale`",
            timebase.num, timebase.den
        )));
    }
    let divisor = gcd(u64::from(timebase.num), u64::from(timebase.den));
    let timescale = u64::from(timebase.den) / divisor;
    let timescale = u32::try_from(timescale).map_err(|_| {
        invalid(format!(
            "the timescale {timescale} exceeds 2^32 - 1. Use a coarser `TimestampScale`"
        ))
    })?;
    Ok((timescale, u64::from(timebase.num) / divisor))
}

/// The spec for a Matroska track, or the refusal that names Matroska.
pub(crate) fn spec_from_matroska(track: &Track, mkv: &MatroskaTrack) -> Result<TrackSpec> {
    if !mkv.content_encodings.is_empty() {
        return Err(refuse(format!(
            "track {} with `ContentEncodings` (compressed or encrypted frames)",
            track.id
        )));
    }
    let codec_id = mkv.codec_id.as_str();
    let (timescale, _) = timescale_of(track.timebase)?;
    let mut spec = match codec_id {
        "V_MPEG4/ISO/AVC" => video_spec(track, mkv, VisualCodec::Avc, timescale)?,
        "V_MPEGH/ISO/HEVC" => video_spec(track, mkv, VisualCodec::Hevc, timescale)?,
        "V_AV1" => video_spec(track, mkv, VisualCodec::Av1, timescale)?,
        "A_AAC" => {
            let config = aac_esds(private(track, mkv)?)?;
            audio_spec(mkv, AudioCodec::Aac, config, timescale)?
        }
        "A_OPUS" => {
            let config = opus_dops(private(track, mkv)?)?;
            audio_spec(mkv, AudioCodec::Opus, config, timescale)?
        }
        "S_TEXT/UTF8" => {
            let mut spec = TrackSpec::new(TrackKind::Subtitle, timescale, SampleEntrySpec::Tx3g);
            spec.packet_format = PacketFormat::PlainText;
            spec
        }
        "S_TEXT/ASS" | "S_TEXT/SSA" | "S_ASS" | "S_SSA" => {
            return Err(refuse(format!(
                "styled ASS/SSA subtitles (track {}, codec `{codec_id}`)",
                track.id
            )));
        }
        "S_HDMV/PGS" | "S_VOBSUB" | "S_DVBSUB" | "S_IMAGE/BMP" => {
            return Err(refuse(format!(
                "image subtitles (track {}, codec `{codec_id}`)",
                track.id
            )));
        }
        other => {
            return Err(refuse(format!(
                "track {} with codec `{other}`: this muxer maps H.264, HEVC, AV1, AAC, Opus \
                 and `S_TEXT/UTF8` from Matroska",
                track.id
            )));
        }
    };
    spec.language = mdhd_language(track.language);
    spec.enabled = mkv.enabled;
    spec.metadata = track.metadata.clone();
    spec.default_duration = default_duration(mkv, timescale)?;
    Ok(spec)
}

/// `DefaultDuration` in MP4 ticks, rounded down.
fn default_duration(mkv: &MatroskaTrack, timescale: u32) -> Result<Option<u32>> {
    let Some(nanoseconds) = mkv.default_duration else {
        return Ok(None);
    };
    let ticks = u128::from(nanoseconds) * u128::from(timescale) / 1_000_000_000;
    u32::try_from(ticks).map(Some).map_err(|_| {
        invalid(format!(
            "`DefaultDuration` {nanoseconds} ns gives {ticks} ticks, above 2^32 - 1. Drop the `DefaultDuration` or use a shorter one"
        ))
    })
}

fn private<'a>(track: &Track, mkv: &'a MatroskaTrack) -> Result<&'a [u8]> {
    match mkv.codec_private.as_deref() {
        Some(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err(refuse(format!(
            "track {} with codec `{}` and no `CodecPrivate`: MP4 stores the codec \
             configuration in the sample entry",
            track.id, mkv.codec_id
        ))),
    }
}

fn dimension(value: u64, field: &str) -> Result<u16> {
    u16::try_from(value).map_err(|_| {
        invalid(format!(
            "`{field}` is {value}, above the MP4 limit of 65 535. Use a smaller value"
        ))
    })
}

fn video_spec(
    track: &Track,
    mkv: &MatroskaTrack,
    codec: VisualCodec,
    timescale: u32,
) -> Result<TrackSpec> {
    let Some(video) = &mkv.video else {
        return Err(invalid(format!(
            "video track {} has no `Video` element, so its size is unknown. Add the `Video` element to the source track",
            track.id
        )));
    };
    if video.crop != [0; 4] {
        return Err(refuse(format!(
            "track {} with `PixelCrop` values {:?}: this muxer writes no `clap` box",
            track.id, video.crop
        )));
    }
    let width = dimension(video.pixel_width, "PixelWidth")?;
    let height = dimension(video.pixel_height, "PixelHeight")?;
    let mut spec = TrackSpec::new(
        TrackKind::Video,
        timescale,
        SampleEntrySpec::Visual {
            codec,
            config: private(track, mkv)?.to_vec(),
            width,
            height,
        },
    );
    if let Some(projection) = &video.projection {
        if projection.projection_type != 0
            || projection.pose_yaw != 0.0
            || projection.pose_pitch != 0.0
        {
            return Err(refuse(format!(
                "track {} with `Projection` type {}, yaw {} and pitch {}: this muxer maps \
                 `ProjectionPoseRoll` of a rectangular projection only",
                track.id, projection.projection_type, projection.pose_yaw, projection.pose_pitch
            )));
        }
        spec.matrix = rotation_matrix(projection.pose_roll)?;
    }
    let (display_width, display_height) = display_size(video);
    spec.display_width = u32::from(dimension(display_width, "DisplayWidth")?) << 16;
    spec.display_height = u32::from(dimension(display_height, "DisplayHeight")?) << 16;
    spec.pixel_aspect = pixel_aspect(video, display_width, display_height)?;
    Ok(spec)
}

/// The display size in pixels. A display size in centimeters or inches
/// says nothing about pixels, so it gives the coded size.
fn display_size(video: &MatroskaVideo) -> (u64, u64) {
    match (
        video.display_unit,
        video.display_width,
        video.display_height,
    ) {
        (DisplayUnit::Pixels, Some(width), Some(height)) if width > 0 && height > 0 => {
            (width, height)
        }
        (DisplayUnit::AspectRatio, Some(width), Some(height)) if width > 0 && height > 0 => {
            let scaled = u128::from(video.pixel_height) * u128::from(width) / u128::from(height);
            (
                u64::try_from(scaled).unwrap_or(u64::MAX),
                video.pixel_height,
            )
        }
        _ => (video.pixel_width, video.pixel_height),
    }
}

/// The `pasp` that turns the coded size into the display size. `None` for
/// square pixels.
fn pixel_aspect(video: &MatroskaVideo, width: u64, height: u64) -> Result<Option<PixelAspect>> {
    let h_spacing = u128::from(width) * u128::from(video.pixel_height);
    let v_spacing = u128::from(height) * u128::from(video.pixel_width);
    if h_spacing == v_spacing || h_spacing == 0 || v_spacing == 0 {
        return Ok(None);
    }
    let mut a = h_spacing;
    let mut b = v_spacing;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let too_large = |_: std::num::TryFromIntError| {
        invalid(format!(
            "the display size {width}x{height} of a {}x{} track gives a pixel aspect above \
             32 bits. Use a display size closer to the pixel size",
            video.pixel_width, video.pixel_height
        ))
    };
    Ok(Some(PixelAspect {
        h_spacing: u32::try_from(h_spacing / a).map_err(too_large)?,
        v_spacing: u32::try_from(v_spacing / a).map_err(too_large)?,
    }))
}

fn audio_spec(
    mkv: &MatroskaTrack,
    codec: AudioCodec,
    config: Vec<u8>,
    timescale: u32,
) -> Result<TrackSpec> {
    // Matroska's defaults, for a track with no `Audio` element.
    let defaults = MatroskaAudio {
        sampling_frequency: 8000.0,
        output_sampling_frequency: None,
        channels: 1,
        bit_depth: None,
        extra: Vec::new(),
    };
    let audio = mkv.audio.as_ref().unwrap_or(&defaults);
    let channels = u16::try_from(audio.channels).map_err(|_| {
        invalid(format!(
            "`Channels` is {}, above the MP4 limit of 65 535. Use 65 535 channels or fewer",
            audio.channels
        ))
    })?;
    let frequency = audio.sampling_frequency;
    let sample_rate = match codec {
        // Opus always decodes at 48 kHz, and the entry stores that rate.
        AudioCodec::Opus => 48_000,
        AudioCodec::Aac
            if frequency.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&frequency) =>
        {
            frequency.round() as u32
        }
        AudioCodec::Aac => {
            return Err(invalid(format!(
                "`SamplingFrequency` is {frequency}. Use a rate from 0 to 2^32 - 1 Hz"
            )));
        }
    };
    Ok(TrackSpec::new(
        TrackKind::Audio,
        timescale,
        SampleEntrySpec::Audio {
            codec,
            config,
            channels,
            sample_rate,
        },
    ))
}

/// `language` in the terminology form `mdhd` stores. Matroska holds the
/// bibliographic form.
fn mdhd_language(language: Language) -> Language {
    language
        .iso639()
        .map(terminology)
        .and_then(|code| <[u8; 3]>::try_from(code.as_bytes()).ok())
        .map_or(language, Language::Iso639)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timescale_reduces_the_matroska_tick() {
        assert_eq!(
            timescale_of(Timebase::from_nanoseconds(1_000_000)).ok(),
            Some((1000, 1))
        );
        assert_eq!(
            timescale_of(Timebase::from_nanoseconds(3)).ok(),
            Some((1_000_000_000, 3))
        );
        assert!(timescale_of(Timebase { num: 0, den: 1 }).is_err());
    }

    #[test]
    fn mdhd_language_uses_the_terminology_form() {
        assert_eq!(
            mdhd_language(Language::Iso639(*b"fre")),
            Language::Iso639(*b"fra")
        );
        assert_eq!(
            mdhd_language(Language::Iso639(*b"eng")),
            Language::Iso639(*b"eng")
        );
    }
}
