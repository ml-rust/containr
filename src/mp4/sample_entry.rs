//! Sample descriptions: `stsd` and its sample entries.
//!
//! Each entry keeps its raw bytes for stream copy. Visual and audio entries
//! also yield their fixed fields and the codec configuration box.

use crate::error::{Error, Result};
use crate::mp4::boxes::{Children, RawBox};
use crate::track::{AudioInfo, CodecConfig, PixelAspect, SampleEntry, TrackKind, VideoInfo};

/// Bytes of the SampleEntry base: 6 reserved, then `data_reference_index`.
const BASE_LEN: usize = 8;
/// Bytes before the children of a VisualSampleEntry.
const VISUAL_LEN: usize = BASE_LEN + 70;
/// Bytes before the children of a version 0 AudioSampleEntry.
const AUDIO_V0_LEN: usize = BASE_LEN + 20;
/// QuickTime sound description version 1 adds four 32-bit fields.
const AUDIO_V1_LEN: usize = AUDIO_V0_LEN + 16;
/// QuickTime sound description version 2 adds 36 bytes.
const AUDIO_V2_LEN: usize = AUDIO_V0_LEN + 36;

/// Parse every entry of a `stsd` box.
pub(crate) fn parse_stsd(
    stsd: &RawBox<'_>,
    path: &str,
    kind: TrackKind,
) -> Result<Vec<SampleEntry>> {
    let mut reader = stsd.reader(path);
    reader.skip(4, "version and flags")?;
    let declared = reader.u32("entry_count")?;
    let mut entries = Vec::new();
    for child in stsd.children(8, path)? {
        if entries.len() as u64 == u64::from(declared) {
            break;
        }
        let child = child?;
        let entry_path = format!("{path}/{}", child.kind);
        entries.push(parse_entry(&child, &entry_path, kind)?);
    }
    if entries.len() as u64 != u64::from(declared) {
        return Err(Error::malformed(
            path,
            stsd.offset,
            format!(
                "entry_count is {declared}, but the box holds {} entries",
                entries.len()
            ),
        ));
    }
    Ok(entries)
}

fn parse_entry(entry: &RawBox<'_>, path: &str, kind: TrackKind) -> Result<SampleEntry> {
    let mut reader = entry.reader(path);
    reader.skip(6, "reserved")?;
    let data_reference_index = reader.u16("data_reference_index")?;
    let mut parsed = SampleEntry {
        fourcc: entry.kind,
        raw: entry.full.to_vec(),
        data_reference_index,
        codec_config: None,
        video: None,
        audio: None,
    };
    match kind {
        TrackKind::Video => {
            reader.skip(16, "pre_defined and reserved")?;
            let width = reader.u16("width")?;
            let height = reader.u16("height")?;
            reader.skip(VISUAL_LEN - reader.position(), "visual sample entry fields")?;
            let mut video = VideoInfo {
                width,
                height,
                pixel_aspect: None,
            };
            for child in entry.children(VISUAL_LEN, path)? {
                let child = child?;
                if &child.kind.0 == b"pasp" {
                    let child_path = format!("{path}/pasp");
                    let mut pasp = child.reader(&child_path);
                    video.pixel_aspect = Some(PixelAspect {
                        h_spacing: pasp.u32("hSpacing")?,
                        v_spacing: pasp.u32("vSpacing")?,
                    });
                } else if parsed.codec_config.is_none() {
                    parsed.codec_config = codec_config(&child);
                }
            }
            parsed.video = Some(video);
        }
        TrackKind::Audio => {
            let (audio, children_at) = audio_fields(entry, path)?;
            parsed.audio = Some(audio);
            parsed.codec_config = audio_config(entry.children(children_at, path)?, path)?;
        }
        TrackKind::Subtitle | TrackKind::Data | TrackKind::Other => {}
    }
    Ok(parsed)
}

/// Read the fixed audio fields. Returns them and where the children start.
fn audio_fields(entry: &RawBox<'_>, path: &str) -> Result<(AudioInfo, usize)> {
    let mut reader = entry.reader(path);
    reader.skip(BASE_LEN, "sample entry base")?;
    let version = reader.u16("version")?;
    reader.skip(6, "revision and vendor")?;
    let channels = reader.u16("channelcount")?;
    let sample_size = reader.u16("samplesize")?;
    reader.skip(4, "compression_id and packet_size")?;
    let rate = reader.u32("samplerate")?;
    match version {
        0 | 1 => {
            let children_at = if version == 1 {
                reader.skip(16, "version 1 sound description fields")?;
                AUDIO_V1_LEN
            } else {
                AUDIO_V0_LEN
            };
            Ok((
                AudioInfo {
                    channels: u32::from(channels),
                    sample_size: u32::from(sample_size),
                    sample_rate: rate >> 16,
                },
                children_at,
            ))
        }
        2 => {
            reader.skip(4, "sizeOfStructOnly")?;
            let rate_bits = reader.u64("audioSampleRate")?;
            let channels = reader.u32("numAudioChannels")?;
            reader.skip(4, "always7F000000")?;
            let bits = reader.u32("constBitsPerChannel")?;
            reader.skip(12, "version 2 sound description flags and packet fields")?;
            let sample_rate = whole_hz(rate_bits).ok_or_else(|| {
                Error::malformed(
                    path,
                    entry.offset,
                    format!("audioSampleRate bits {rate_bits:#018x} are not a whole number of Hz below 2^32"),
                )
            })?;
            Ok((
                AudioInfo {
                    channels,
                    sample_size: bits,
                    sample_rate,
                },
                AUDIO_V2_LEN,
            ))
        }
        other => Err(Error::malformed(
            path,
            entry.offset,
            format!(
                "sound sample description version {other} is unknown. Versions 0, 1 and 2 exist"
            ),
        )),
    }
}

/// The integer value of an IEEE 754 double, when it is a whole number in
/// `0..2^32`. A sample rate is not a timestamp, and this reads the bits
/// without floating-point arithmetic.
fn whole_hz(bits: u64) -> Option<u32> {
    if bits == 0 {
        return Some(0);
    }
    if bits >> 63 != 0 {
        return None;
    }
    let exponent = ((bits >> 52) & 0x7FF) as i64 - 1023;
    if !(0..32).contains(&exponent) {
        return None;
    }
    let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let shift = 52 - exponent as u32;
    if mantissa & ((1u64 << shift) - 1) != 0 {
        return None;
    }
    u32::try_from(mantissa >> shift).ok()
}

/// Find the codec configuration of an audio entry, inside a QuickTime `wave`
/// box when the entry has one.
fn audio_config(children: Children<'_, '_>, path: &str) -> Result<Option<CodecConfig>> {
    for child in children {
        let child = child?;
        if &child.kind.0 == b"wave" {
            let wave_path = format!("{path}/wave");
            if let Some(config) = audio_config(child.children(0, &wave_path)?, &wave_path)? {
                return Ok(Some(config));
            }
        } else if let Some(config) = codec_config(&child) {
            return Ok(Some(config));
        }
    }
    Ok(None)
}

fn codec_config(child: &RawBox<'_>) -> Option<CodecConfig> {
    let wrap: fn(Vec<u8>) -> CodecConfig = match &child.kind.0 {
        b"avcC" => CodecConfig::Avc,
        b"hvcC" => CodecConfig::Hevc,
        b"av1C" => CodecConfig::Av1,
        b"vpcC" => CodecConfig::Vp,
        b"esds" => CodecConfig::Esds,
        b"dOps" => CodecConfig::Opus,
        _ => return None,
    };
    Some(wrap(child.body.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::FourCc;

    fn entry(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn stsd(entries: &[Vec<u8>], declared: u32) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&declared.to_be_bytes());
        for e in entries {
            body.extend_from_slice(e);
        }
        entry(b"stsd", &body)
    }

    fn visual(width: u16, height: u16, children: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; VISUAL_LEN];
        body[7] = 1;
        body[24..26].copy_from_slice(&width.to_be_bytes());
        body[26..28].copy_from_slice(&height.to_be_bytes());
        body.extend_from_slice(children);
        entry(b"avc1", &body)
    }

    fn parse(bytes: &[u8], kind: TrackKind) -> Result<Vec<SampleEntry>> {
        let raw = RawBox::whole(bytes, 0, "stsd")?;
        parse_stsd(&raw, "stsd", kind)
    }

    #[test]
    fn visual_entry_yields_size_pasp_and_config() {
        let mut children = entry(b"avcC", &[1, 2, 3]);
        children.extend(entry(b"pasp", &[0, 0, 0, 4, 0, 0, 0, 3]));
        let bytes = stsd(&[visual(96, 64, &children)], 1);
        let entries = parse(&bytes, TrackKind::Video).expect("the box is well formed");
        let first = &entries[0];
        assert_eq!(first.fourcc, FourCc(*b"avc1"));
        assert_eq!(first.data_reference_index, 1);
        assert_eq!(first.codec_config, Some(CodecConfig::Avc(vec![1, 2, 3])));
        let video = first.video.expect("a visual entry yields video facts");
        assert_eq!((video.width, video.height), (96, 64));
        assert_eq!(
            video.pixel_aspect,
            Some(PixelAspect {
                h_spacing: 4,
                v_spacing: 3
            })
        );
        assert_eq!(first.raw.len(), 8 + VISUAL_LEN + children.len());
    }

    #[test]
    fn quicktime_v1_audio_finds_esds_inside_wave() {
        let mut body = vec![0u8; AUDIO_V1_LEN];
        body[8..10].copy_from_slice(&1u16.to_be_bytes());
        body[16..18].copy_from_slice(&2u16.to_be_bytes());
        body[18..20].copy_from_slice(&16u16.to_be_bytes());
        body[24..28].copy_from_slice(&(48_000u32 << 16).to_be_bytes());
        let mut wave = entry(b"frma", b"mp4a");
        wave.extend(entry(b"esds", &[0, 0, 0, 0, 3]));
        wave.extend([0u8; 8]);
        body.extend(entry(b"wave", &wave));
        let bytes = stsd(&[entry(b"mp4a", &body)], 1);
        let entries = parse(&bytes, TrackKind::Audio).expect("the box is well formed");
        let audio = entries[0].audio.expect("an audio entry yields audio facts");
        assert_eq!(
            (audio.channels, audio.sample_size, audio.sample_rate),
            (2, 16, 48_000)
        );
        assert_eq!(
            entries[0].codec_config,
            Some(CodecConfig::Esds(vec![0, 0, 0, 0, 3]))
        );
    }

    #[test]
    fn stsd_rejects_a_missing_entry() {
        let bytes = stsd(&[visual(1, 1, &[])], 2);
        assert!(matches!(
            parse(&bytes, TrackKind::Video),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn visual_entry_rejects_a_short_body() {
        let bytes = stsd(&[entry(b"avc1", &[0; 20])], 1);
        assert!(matches!(
            parse(&bytes, TrackKind::Video),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn whole_hz_reads_exact_rates_only() {
        assert_eq!(whole_hz(48_000f64.to_bits()), Some(48_000));
        assert_eq!(whole_hz(1f64.to_bits()), Some(1));
        assert_eq!(whole_hz(0.5f64.to_bits()), None);
        assert_eq!(whole_hz((-1f64).to_bits()), None);
        assert_eq!(whole_hz(f64::NAN.to_bits()), None);
        assert_eq!(whole_hz(4_294_967_296f64.to_bits()), None);
    }
}
