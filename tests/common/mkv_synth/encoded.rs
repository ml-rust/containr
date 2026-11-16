//! A one-track Opus file with `ContentEncodings`, so decode tests know the
//! plain and stored bytes of every frame and of `CodecPrivate`.

use miniz_oxide::deflate::compress_to_vec_zlib;

use super::bare::opus_head;
use super::blocks::{Laced, lace_header};
use super::elements::{cat, el, text, uint};

/// The `TrackNumber` of the one track.
pub const ENCODED_TRACK: u32 = 1;

/// One `ContentEncoding` to write.
#[derive(Clone, Debug)]
pub struct EncodingIn {
    pub order: u64,
    pub scope: u64,
    pub encoding_type: u64,
    /// `ContentCompAlgo`, for compression.
    pub algorithm: Option<u64>,
    /// `ContentCompSettings`, for compression.
    pub settings: Vec<u8>,
}

impl EncodingIn {
    /// Header stripping of `prefix`.
    pub fn strip(order: u64, scope: u64, prefix: &[u8]) -> Self {
        Self {
            settings: prefix.to_vec(),
            ..Self::compression(order, scope, 3)
        }
    }

    /// zlib compression.
    pub fn zlib(order: u64, scope: u64) -> Self {
        Self::compression(order, scope, 0)
    }

    /// Compression with `algorithm` and no settings.
    pub fn compression(order: u64, scope: u64, algorithm: u64) -> Self {
        Self {
            order,
            scope,
            encoding_type: 0,
            algorithm: Some(algorithm),
            settings: Vec::new(),
        }
    }

    /// AES encryption (`ContentEncAlgo` 5) of frames.
    pub fn encryption(order: u64) -> Self {
        Self {
            order,
            scope: 1,
            encoding_type: 1,
            algorithm: None,
            settings: Vec::new(),
        }
    }

    fn element(&self) -> Vec<u8> {
        let detail = match self.algorithm {
            Some(algorithm) => {
                let settings = if self.settings.is_empty() {
                    Vec::new()
                } else {
                    el(0x4255, &self.settings)
                };
                el(0x5034, &cat(&[uint(0x4254, algorithm), settings]))
            }
            None => el(0x5035, &uint(0x47E1, 5)),
        };
        el(
            0x6240,
            &cat(&[
                uint(0x5031, self.order),
                uint(0x5032, self.scope),
                uint(0x5033, self.encoding_type),
                detail,
            ]),
        )
    }

    /// Apply this encoding to `bytes` as a writer does. Encodings the
    /// demuxer leaves raw change nothing here, since their bytes are opaque.
    fn encode(&self, bytes: &[u8]) -> Vec<u8> {
        match (self.encoding_type, self.algorithm) {
            (0, Some(3)) => bytes
                .strip_prefix(self.settings.as_slice())
                .unwrap_or_else(|| panic!("{bytes:?} does not start with {:?}", self.settings))
                .to_vec(),
            (0, Some(0)) => compress_to_vec_zlib(bytes, 6),
            _ => bytes.to_vec(),
        }
    }
}

/// Apply `encodings` with scope bit `scope` to `plain`, lowest
/// `ContentEncodingOrder` first, as a writer does.
fn encode(encodings: &[EncodingIn], scope: u64, plain: &[u8]) -> Vec<u8> {
    let mut ordered: Vec<&EncodingIn> = encodings.iter().collect();
    ordered.sort_by_key(|encoding| encoding.order);
    ordered
        .into_iter()
        .filter(|encoding| encoding.scope & scope != 0)
        .fold(plain.to_vec(), |bytes, encoding| encoding.encode(&bytes))
}

/// A built file and what it stores.
pub struct EncodedFile {
    pub bytes: Vec<u8>,
    /// The stored bytes of each frame, in file order.
    pub stored: Vec<Vec<u8>>,
    /// The stored `CodecPrivate`.
    pub stored_private: Vec<u8>,
}

/// A file whose frames and `CodecPrivate` carry `encodings`. Each block
/// holds the plain frames given: one frame is an unlaced `SimpleBlock`, more
/// are Xiph-laced. The `CodecPrivate` is an `OpusHead`.
pub fn encoded_file(encodings: &[EncodingIn], blocks: &[Vec<Vec<u8>>]) -> EncodedFile {
    let stored_private = encode(encodings, 2, &opus_head());
    let stored_blocks: Vec<Vec<Vec<u8>>> = blocks
        .iter()
        .map(|frames| {
            frames
                .iter()
                .map(|frame| encode(encodings, 1, frame))
                .collect()
        })
        .collect();
    let bytes = stored_file(encodings, &stored_private, &stored_blocks);
    EncodedFile {
        bytes,
        stored: stored_blocks.into_iter().flatten().collect(),
        stored_private,
    }
}

/// A file that lists `encodings` and stores `private` and `blocks` as given.
pub fn stored_file(encodings: &[EncodingIn], private: &[u8], blocks: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let header = el(
        0x1A45_DFA3,
        &cat(&[
            uint(0x4286, 1),
            uint(0x42F2, 4),
            uint(0x42F3, 8),
            text(0x4282, "matroska"),
            uint(0x4287, 4),
            uint(0x4285, 2),
        ]),
    );
    let info = el(
        0x1549_A966,
        &cat(&[
            uint(0x2AD7B1, 1_000_000),
            text(0x4D80, "containr-test"),
            text(0x5741, "containr-test"),
        ]),
    );
    let audio = el(
        0xE1,
        &cat(&[
            el(0xB5, &48_000.0f32.to_bits().to_be_bytes()),
            uint(0x9F, 2),
        ]),
    );
    let list: Vec<Vec<u8>> = encodings.iter().map(EncodingIn::element).collect();
    let entry = el(
        0xAE,
        &cat(&[
            uint(0xD7, u64::from(ENCODED_TRACK)),
            uint(0x73C5, 0x4444),
            uint(0x83, 2),
            text(0x86, "A_OPUS"),
            el(0x63A2, private),
            uint(0x23E383, 20_000_000),
            audio,
            el(0x6D80, &cat(&list)),
        ]),
    );
    let tracks = el(0x1654_AE6B, &entry);
    let mut cluster = uint(0xE7, 0);
    let mut frames = 0i16;
    for block in blocks {
        let mut body = vec![0x80 | ENCODED_TRACK as u8];
        body.extend((frames * 20).to_be_bytes());
        let sizes: Vec<u32> = block.iter().map(|frame| frame.len() as u32).collect();
        if block.len() > 1 {
            // A keyframe, Xiph-laced.
            body.push(0x82);
            body.extend(lace_header(Laced::Xiph, &sizes));
        } else {
            body.push(0x80);
        }
        for frame in block {
            body.extend_from_slice(frame);
        }
        cluster.extend(el(0xA3, &body));
        frames += block.len() as i16;
    }
    let segment = cat(&[info, tracks, el(0x1F43_B675, &cluster)]);
    cat(&[header, el(0x1853_8067, &segment)])
}

/// Deflate bits, least significant bit first.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    count: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        for bit in 0..count {
            self.acc |= ((value >> bit) & 1) << self.count;
            self.count += 1;
            if self.count == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.count = 0;
            }
        }
    }

    /// A Huffman code, which deflate packs most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        for bit in (0..len).rev() {
            self.put((code >> bit) & 1, 1);
        }
    }
}

/// A small zlib stream that inflates to at least `len` zero bytes: one
/// literal, then fixed-Huffman matches of 258 bytes at distance 1. The
/// Adler-32 trailer is zero, since an inflate that stops at a limit never
/// reads it.
pub fn zlib_bomb(len: u64) -> Vec<u8> {
    let mut bits = Bits {
        out: vec![0x78, 0x01],
        acc: 0,
        count: 0,
    };
    // BFINAL, then BTYPE 01: fixed Huffman codes.
    bits.put(1, 1);
    bits.put(1, 2);
    // Literal 0.
    bits.code(0x30, 8);
    for _ in 0..len.div_ceil(258) {
        // Length 258 is code 285, distance 1 is distance code 0.
        bits.code(0xC5, 8);
        bits.code(0, 5);
    }
    // End of block.
    bits.code(0, 7);
    bits.put(0, 7);
    let mut out = bits.out;
    out.extend([0; 4]);
    out
}
