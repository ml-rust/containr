//! Sample entry bytes: a raw entry checked for stream copy, or an entry
//! built from a codec configuration.

use crate::error::Result;
use crate::mp4::mux::bytes::BoxBuf;
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::spec::SampleEntrySpec;
use crate::track::{FourCc, PixelAspect};

/// The QuickTime `text` sample entry of a chapter track, as FFmpeg writes
/// it: display flags 1, default text box, and a one-font `ftab`.
pub(crate) const CHAPTER_TEXT_ENTRY: [u8; 59] = [
    0x00, 0x00, 0x00, 0x3B, b't', b'e', b'x', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x0D, b'f', b't', b'a', b'b', 0x00, 0x01, 0x00, 0x01, 0x00,
];

/// The `gmhd` media header of a QuickTime text track: `gmin` and `text`, as
/// FFmpeg writes them.
pub(crate) const TEXT_GMHD: [u8; 76] = [
    0x00, 0x00, 0x00, 0x4C, b'g', b'm', b'h', b'd', 0x00, 0x00, 0x00, 0x18, b'g', b'm', b'i', b'n',
    0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x2C, b't', b'e', b'x', b't', 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
];

/// The bytes before `data_reference_index` in every sample entry, header
/// included.
const DREF_AT: usize = 14;

/// The sample entry box for `spec`, and its type.
pub(crate) fn build_entry(
    spec: &SampleEntrySpec,
    pixel_aspect: Option<PixelAspect>,
) -> Result<(FourCc, Vec<u8>)> {
    match spec {
        SampleEntrySpec::Raw(raw) => raw_entry(raw),
        SampleEntrySpec::Visual {
            codec,
            config,
            width,
            height,
        } => {
            let (kind, config_kind) = codec.boxes();
            if config.is_empty() {
                return Err(invalid(format!(
                    "the `{}` configuration of a `{}` entry is empty. Pass the codec \
                     configuration record",
                    FourCc(*config_kind),
                    FourCc(*kind)
                )));
            }
            let path = FourCc(*kind).to_string();
            let mut buf = BoxBuf::new();
            let start = buf.open(kind);
            buf.zeros(6);
            buf.u16(1);
            buf.zeros(16);
            buf.u16(*width);
            buf.u16(*height);
            buf.u32(0x0048_0000);
            buf.u32(0x0048_0000);
            buf.u32(0);
            buf.u16(1);
            buf.zeros(32);
            buf.u16(0x0018);
            buf.u16(0xFFFF);
            buf.boxed(
                config_kind,
                config,
                &format!("{path}/{}", FourCc(*config_kind)),
            )?;
            if let Some(aspect) = pixel_aspect {
                let pasp_start = buf.open(b"pasp");
                buf.u32(aspect.h_spacing);
                buf.u32(aspect.v_spacing);
                buf.close(pasp_start, &format!("{path}/pasp"))?;
            }
            buf.close(start, &path)?;
            Ok((FourCc(*kind), buf.into_bytes()))
        }
        SampleEntrySpec::Audio {
            codec,
            config,
            channels,
            sample_rate,
        } => {
            let (kind, config_kind) = codec.boxes();
            let path = FourCc(*kind).to_string();
            let mut buf = BoxBuf::new();
            let start = buf.open(kind);
            buf.zeros(6);
            buf.u16(1);
            buf.zeros(8);
            buf.u16(*channels);
            buf.u16(16);
            buf.zeros(4);
            buf.u32(if *sample_rate <= 0xFFFF {
                sample_rate << 16
            } else {
                0
            });
            buf.boxed(
                config_kind,
                config,
                &format!("{path}/{}", FourCc(*config_kind)),
            )?;
            buf.close(start, &path)?;
            Ok((FourCc(*kind), buf.into_bytes()))
        }
        SampleEntrySpec::Tx3g => {
            let mut buf = BoxBuf::new();
            let start = buf.open(b"tx3g");
            buf.zeros(6);
            buf.u16(1);
            buf.u32(0);
            buf.bytes(&[0x01, 0xFF, 0x00, 0x00, 0x00, 0xFF]);
            buf.zeros(8);
            buf.bytes(&[0, 0, 0, 0, 0, 1, 0, 0x10, 0xFF, 0xFF, 0xFF, 0xFF]);
            let ftab = buf.open(b"ftab");
            buf.u16(1);
            buf.u16(1);
            buf.u8(5);
            buf.bytes(b"Arial");
            buf.close(ftab, "tx3g/ftab")?;
            buf.close(start, "tx3g")?;
            Ok((FourCc(*b"tx3g"), buf.into_bytes()))
        }
    }
}

/// Check a raw sample entry and point its data reference at the file.
fn raw_entry(raw: &[u8]) -> Result<(FourCc, Vec<u8>)> {
    let (Some(size), Some(kind)) = (raw.get(..4), raw.get(4..8)) else {
        return Err(invalid(format!(
            "the raw sample entry holds {} bytes, too few for a box header",
            raw.len()
        )));
    };
    let mut kind_bytes = [0u8; 4];
    kind_bytes.copy_from_slice(kind);
    let kind = FourCc(kind_bytes);
    let declared = u32::from_be_bytes([size[0], size[1], size[2], size[3]]);
    if declared as usize != raw.len() || raw.len() < DREF_AT + 2 {
        return Err(invalid(format!(
            "the raw `{kind}` sample entry declares {declared} bytes and holds {}. \
             Pass the whole box, as `SampleEntry::raw` holds it",
            raw.len()
        )));
    }
    let mut out = raw.to_vec();
    if let Some(field) = out.get_mut(DREF_AT..DREF_AT + 2) {
        field.copy_from_slice(&[0, 1]);
    }
    Ok((kind, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::mux::spec::{AudioCodec, VisualCodec};

    #[test]
    fn visual_entry_holds_size_config_and_pasp() {
        let spec = SampleEntrySpec::Visual {
            codec: VisualCodec::Avc,
            config: vec![1, 2, 3],
            width: 96,
            height: 64,
        };
        let aspect = PixelAspect {
            h_spacing: 4,
            v_spacing: 3,
        };
        let (kind, bytes) = build_entry(&spec, Some(aspect)).expect("the entry builds");
        assert_eq!(kind, FourCc(*b"avc1"));
        assert_eq!(bytes.len(), 8 + 78 + 11 + 16);
        assert_eq!(&bytes[32..36], &[0, 96, 0, 64]);
        assert_eq!(
            &bytes[86..97],
            &[0, 0, 0, 11, b'a', b'v', b'c', b'C', 1, 2, 3]
        );
        assert_eq!(&bytes[101..105], b"pasp");
    }

    #[test]
    fn audio_entry_stores_the_rate_as_16_16() {
        let spec = SampleEntrySpec::Audio {
            codec: AudioCodec::Opus,
            config: vec![0; 11],
            channels: 2,
            sample_rate: 48_000,
        };
        let (kind, bytes) = build_entry(&spec, None).expect("the entry builds");
        assert_eq!(kind, FourCc(*b"Opus"));
        assert_eq!(&bytes[24..26], &[0, 2]);
        assert_eq!(&bytes[32..36], &(48_000u32 << 16).to_be_bytes());
        assert_eq!(&bytes[40..44], b"dOps");
    }

    #[test]
    fn raw_entry_resets_the_data_reference() {
        let mut raw = vec![0, 0, 0, 16, b'm', b'p', b'4', b'a', 0, 0, 0, 0, 0, 0, 0, 7];
        let (kind, out) =
            build_entry(&SampleEntrySpec::Raw(raw.clone()), None).expect("the entry is whole");
        assert_eq!(kind, FourCc(*b"mp4a"));
        assert_eq!(&out[14..16], &[0, 1]);
        raw[3] = 40;
        assert!(build_entry(&SampleEntrySpec::Raw(raw), None).is_err());
    }

    #[test]
    fn text_constants_declare_their_length() {
        assert_eq!(CHAPTER_TEXT_ENTRY[3] as usize, CHAPTER_TEXT_ENTRY.len());
        assert_eq!(TEXT_GMHD[3] as usize, TEXT_GMHD.len());
    }
}
