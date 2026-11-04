//! MP4 codec configuration in its Matroska form: the AudioSpecificConfig
//! inside `esds`, and `OpusHead` from `dOps`.

use crate::error::Result;
use crate::mkv::mux::bytes::{invalid, refuse};

/// The MPEG-4 audio `objectTypeIndication`, and the MPEG-2 AAC profiles.
const AAC_OBJECT_TYPES: [u8; 4] = [0x40, 0x66, 0x67, 0x68];

/// One MPEG-4 descriptor: its tag, its body and the bytes after it.
fn descriptor(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    let mut len = 0usize;
    let mut pos = 0usize;
    loop {
        let &byte = rest.get(pos)?;
        pos += 1;
        len = (len << 7) | usize::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            break;
        }
        if pos == 4 {
            return None;
        }
    }
    let body = rest.get(pos..pos + len)?;
    Some((tag, body, rest.get(pos + len..)?))
}

/// The body of the first descriptor with `tag` in `bytes`.
fn find(mut bytes: &[u8], tag: u8) -> Option<&[u8]> {
    while let Some((found, body, rest)) = descriptor(bytes) {
        if found == tag {
            return Some(body);
        }
        bytes = rest;
    }
    None
}

/// The bytes of an ES descriptor after its fixed and optional fields.
fn after_es_fields(es: &[u8]) -> Option<&[u8]> {
    let &flags = es.get(2)?;
    let mut pos = 3usize;
    if flags & 0x80 != 0 {
        pos += 2;
    }
    if flags & 0x40 != 0 {
        pos += 1 + usize::from(*es.get(pos)?);
    }
    if flags & 0x20 != 0 {
        pos += 2;
    }
    es.get(pos..)
}

/// The AudioSpecificConfig in an `esds` body, version and flags first.
/// `track_id` names the track in errors. An object type other than AAC is
/// refused, and the error names MP4.
pub(crate) fn audio_specific_config(esds: &[u8], track_id: u32) -> Result<&[u8]> {
    let malformed = || {
        invalid(format!(
            "track {track_id}: the `esds` of {} bytes holds no whole ES descriptor, decoder \
             config and decoder specific info. Remux the source with `ffmpeg -c copy`",
            esds.len()
        ))
    };
    let body = esds.get(4..).ok_or_else(malformed)?;
    let es = find(body, 0x03).ok_or_else(malformed)?;
    let fields = after_es_fields(es).ok_or_else(malformed)?;
    let decoder = find(fields, 0x04).ok_or_else(malformed)?;
    let &object_type = decoder.first().ok_or_else(malformed)?;
    if !AAC_OBJECT_TYPES.contains(&object_type) {
        return Err(refuse(format!(
            "track {track_id}: `mp4a` with objectTypeIndication {object_type:#04x}. This muxer \
             maps AAC only, types 0x40 and 0x66 to 0x68"
        )));
    }
    let children = decoder.get(13..).ok_or_else(malformed)?;
    find(children, 0x05).ok_or_else(malformed)
}

/// The `OpusHead` for a `dOps` body. `dOps` is big-endian and `OpusHead` is
/// little-endian. The channel mapping fields carry over.
pub(crate) fn opus_head(dops: &[u8], track_id: u32) -> Result<Vec<u8>> {
    let &[
        _,
        channels,
        skip_high,
        skip_low,
        r0,
        r1,
        r2,
        r3,
        gain_high,
        gain_low,
        family,
        ref mapping @ ..,
    ] = dops
    else {
        return Err(invalid(format!(
            "track {track_id}: the `dOps` holds {} bytes, below the 11-byte minimum. Remux the \
             source with `ffmpeg -c copy`",
            dops.len()
        )));
    };
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[
        1, channels, skip_low, skip_high, r3, r2, r1, r0, gain_low, gain_high, family,
    ]);
    head.extend_from_slice(mapping);
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `esds` body with the given AudioSpecificConfig and object type.
    fn esds(object_type: u8, config: &[u8], long_lengths: bool) -> Vec<u8> {
        let wrap = |tag: u8, body: &[u8]| {
            let mut out = vec![tag];
            if long_lengths {
                out.extend_from_slice(&[0x80, 0x80, 0x80]);
            }
            out.push(body.len() as u8);
            out.extend_from_slice(body);
            out
        };
        let mut decoder = vec![object_type, 0x15];
        decoder.extend_from_slice(&[0; 11]);
        decoder.extend(wrap(0x05, config));
        let mut es = vec![0, 0, 0];
        es.extend(wrap(0x04, &decoder));
        es.extend(wrap(0x06, &[2]));
        let mut body = vec![0, 0, 0, 0];
        body.extend(wrap(0x03, &es));
        body
    }

    #[test]
    fn esds_gives_the_audio_specific_config_at_both_length_forms() {
        for long_lengths in [false, true] {
            let body = esds(0x40, &[0x11, 0x90], long_lengths);
            assert_eq!(
                audio_specific_config(&body, 1).ok(),
                Some(&[0x11, 0x90][..])
            );
        }
    }

    #[test]
    fn esds_with_optional_es_fields_is_read() {
        // Flags: a stream dependence, a URL of 2 bytes and an OCR stream.
        let mut es = vec![0, 0, 0xE0, 0, 0, 2, b'u', b'r', 0, 0];
        let mut decoder = vec![0x40];
        decoder.extend_from_slice(&[0; 12]);
        decoder.extend_from_slice(&[0x05, 1, 0x12]);
        es.extend_from_slice(&[0x04, decoder.len() as u8]);
        es.extend(decoder);
        let mut body = vec![0, 0, 0, 0, 0x03, es.len() as u8];
        body.extend(es);
        assert_eq!(audio_specific_config(&body, 1).ok(), Some(&[0x12][..]));
    }

    #[test]
    fn esds_refuses_other_object_types_and_short_bodies() {
        let mp3 = esds(0x6B, &[1], false);
        assert!(
            audio_specific_config(&mp3, 7)
                .err()
                .is_some_and(|err| err.to_string().contains("0x6b"))
        );
        let whole = esds(0x40, &[0x11, 0x90], false);
        for cut in 0..whole.len() - 1 {
            // A cut body never panics. A cut in the trailing SL descriptor
            // still finds the config.
            let _ = audio_specific_config(&whole[..cut], 1);
        }
        assert!(audio_specific_config(&[0, 0], 1).is_err());
    }

    #[test]
    fn opus_head_reverses_the_dops_conversion() {
        let dops = [0, 2, 0x01, 0x38, 0, 0, 0xBB, 0x80, 0xFF, 0xFE, 0];
        let mut want = b"OpusHead".to_vec();
        want.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0xFE, 0xFF, 0]);
        assert_eq!(opus_head(&dops, 1).ok(), Some(want.clone()));
        let mut family = dops.to_vec();
        family[10] = 1;
        family.extend_from_slice(&[1, 1, 0, 1]);
        let head = opus_head(&family, 1).expect("the body is whole");
        assert_eq!(&head[18..], &[1, 1, 1, 0, 1]);
        assert!(opus_head(&dops[..10], 1).is_err());
    }
}
