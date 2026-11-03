//! Codec configuration and rotation in their MP4 form: `esds` from an
//! AudioSpecificConfig, `dOps` from an `OpusHead`, and the `tkhd` matrix
//! from a Matroska `ProjectionPoseRoll`.

use crate::error::Result;
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::spec::IDENTITY_MATRIX;

/// The largest `CodecPrivate` accepted for an `esds`, in bytes. The
/// descriptor length field holds 28 bits, and an AudioSpecificConfig spans
/// a few bytes.
const MAX_AUDIO_CONFIG: usize = 1 << 20;

/// The `tkhd` matrix for a `ProjectionPoseRoll`, in counter-clockwise
/// degrees. FFmpeg writes `-display_rotation 90` as roll 90 and matrix
/// `0 -1 0 1 0 0 0 0 1`. Multiples of 90 stay exact.
pub(crate) fn rotation_matrix(roll: f64) -> Result<[i32; 9]> {
    if !roll.is_finite() {
        return Err(invalid(format!(
            "`ProjectionPoseRoll` is {roll}. Use a finite angle"
        )));
    }
    let quarters = roll / 90.0;
    let (cos, sin) = if quarters.fract() == 0.0 {
        match (quarters as i64).rem_euclid(4) {
            0 => (0x10000, 0),
            1 => (0, 0x10000),
            2 => (-0x10000, 0),
            _ => (0, -0x10000),
        }
    } else {
        let radians = roll.to_radians();
        (
            (radians.cos() * 65536.0).round() as i32,
            (radians.sin() * 65536.0).round() as i32,
        )
    };
    let mut matrix = IDENTITY_MATRIX;
    matrix[0] = cos;
    matrix[1] = -sin;
    matrix[3] = sin;
    matrix[4] = cos;
    Ok(matrix)
}

/// One MPEG-4 descriptor with a 4-byte length.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len();
    let mut out = Vec::with_capacity(5 + len);
    out.extend_from_slice(&[
        tag,
        0x80 | ((len >> 21) & 0x7F) as u8,
        0x80 | ((len >> 14) & 0x7F) as u8,
        0x80 | ((len >> 7) & 0x7F) as u8,
        (len & 0x7F) as u8,
    ]);
    out.extend_from_slice(body);
    out
}

/// The `esds` body for an AudioSpecificConfig. Bit rates stay 0: stream
/// copy knows no bit rate up front, and readers take the rate from the
/// samples.
pub(crate) fn aac_esds(config: &[u8]) -> Result<Vec<u8>> {
    if config.len() > MAX_AUDIO_CONFIG {
        return Err(invalid(format!(
            "the AAC `CodecPrivate` holds {} bytes, above the limit of {MAX_AUDIO_CONFIG}",
            config.len()
        )));
    }
    let mut decoder = vec![0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    decoder.extend(descriptor(0x05, config));
    let mut es = vec![0, 0, 0];
    es.extend(descriptor(0x04, &decoder));
    es.extend(descriptor(0x06, &[0x02]));
    let mut body = vec![0, 0, 0, 0];
    body.extend(descriptor(0x03, &es));
    Ok(body)
}

/// The `dOps` body for an `OpusHead`. `OpusHead` is little-endian and
/// `dOps` is big-endian. The channel mapping fields carry over when the
/// mapping family is not 0.
pub(crate) fn opus_dops(head: &[u8]) -> Result<Vec<u8>> {
    let malformed = |reason: String| invalid(format!("the Opus `CodecPrivate` {reason}"));
    if head.len() < 19 || !head.starts_with(b"OpusHead") {
        return Err(malformed(format!(
            "holds {} bytes and does not start an `OpusHead` of 19 bytes or more",
            head.len()
        )));
    }
    let channels = head[9];
    let mut body = vec![0, channels];
    body.extend_from_slice(&[head[11], head[10]]);
    body.extend_from_slice(&[head[15], head[14], head[13], head[12]]);
    body.extend_from_slice(&[head[17], head[16]]);
    let family = head[18];
    body.push(family);
    if family != 0 {
        let mapping = head.get(19..21 + usize::from(channels)).ok_or_else(|| {
            malformed(format!(
                "holds {} bytes, too few for channel mapping family {family} with {channels} \
                 channels",
                head.len()
            ))
        })?;
        body.extend_from_slice(mapping);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matches_the_ffmpeg_matrix() {
        assert_eq!(rotation_matrix(0.0).ok(), Some(IDENTITY_MATRIX));
        assert_eq!(
            rotation_matrix(90.0).ok(),
            Some([0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x4000_0000])
        );
        assert_eq!(
            rotation_matrix(-90.0).ok(),
            Some([0, 0x10000, 0, -0x10000, 0, 0, 0, 0, 0x4000_0000])
        );
        assert!(rotation_matrix(f64::NAN).is_err());
    }

    #[test]
    fn esds_wraps_the_audio_specific_config() {
        let body = aac_esds(&[0x11, 0x90]).expect("the config fits");
        assert_eq!(&body[..4], &[0, 0, 0, 0]);
        assert_eq!(body[4], 0x03);
        assert_eq!(
            &body[body.len() - 6..],
            &[0x06, 0x80, 0x80, 0x80, 0x01, 0x02]
        );
        let at = body
            .windows(2)
            .position(|w| w == [0x11, 0x90])
            .expect("the config is present");
        assert_eq!(body[at - 5], 0x05);
    }

    #[test]
    fn dops_swaps_opus_head_to_big_endian() {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0xFE, 0xFF, 0]);
        assert_eq!(
            opus_dops(&head).expect("the head is whole"),
            vec![0, 2, 0x01, 0x38, 0, 0, 0xBB, 0x80, 0xFF, 0xFE, 0]
        );
        head.truncate(10);
        assert!(opus_dops(&head).is_err());
    }

    #[test]
    fn opus_mapping_family_1_keeps_the_mapping() {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2, 0, 0, 0x80, 0xBB, 0, 0, 0, 0, 1, 1, 1, 0, 1]);
        let dops = opus_dops(&head).expect("the head is whole");
        assert_eq!(&dops[10..], &[1, 1, 1, 0, 1]);
    }
}
