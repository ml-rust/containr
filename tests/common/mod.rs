//! Shared helpers: fixture loading, ffprobe reference reads, checksums.

#![allow(dead_code)]

#[cfg(feature = "mkv")]
pub mod mkv_probe;
pub mod mkv_synth;
#[cfg(feature = "mp4")]
pub mod mp4_probe;
pub mod synth;

use std::path::PathBuf;

use serde_json::Value;

/// A generated fixture and its ffprobe reference data.
pub struct Fixture {
    pub name: String,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub probe: Value,
}

/// The fixture directory and generator for a fixture name: `mkv` and
/// `webm` files come from gen_mkv.py, the rest from gen_mp4.py.
pub fn fixture_source(name: &str) -> (PathBuf, &'static str) {
    let matroska = name.ends_with(".mkv") || name.ends_with(".webm");
    let (dir, generator) = if matroska {
        ("mkv", "gen_mkv.py")
    } else {
        ("mp4", "gen_mp4.py")
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    (root.join(dir), generator)
}

/// Load fixture `name`, or print a notice and return `None` when the
/// generator has not run.
pub fn fixture(name: &str) -> Option<Fixture> {
    let (dir, generator) = fixture_source(name);
    let path = dir.join(name);
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        panic!("fixture name {name} has no file stem");
    };
    let probe_path = path.with_file_name(format!("{stem}.probe.json"));
    let (Ok(bytes), Ok(probe)) = (std::fs::read(&path), std::fs::read_to_string(&probe_path))
    else {
        eprintln!(
            "skipping: fixture {} or {} is missing. Run `python3 tests/fixtures/{generator}` to generate it.",
            path.display(),
            probe_path.display()
        );
        return None;
    };
    let probe = serde_json::from_str(&probe).unwrap_or_else(|err| {
        panic!(
            "{} is not valid JSON: {err}. Run the generator again.",
            probe_path.display()
        )
    });
    Some(Fixture {
        name: name.to_owned(),
        path,
        bytes,
        probe,
    })
}

/// A JSON field as an integer. ffprobe writes some integers as strings.
pub fn int(value: &Value, field: &str) -> i64 {
    match &value[field] {
        Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| panic!("{field} = {n} is not an i64")),
        Value::String(s) => s
            .parse()
            .unwrap_or_else(|_| panic!("{field} = {s:?} is not an integer")),
        other => panic!("{field} is {other}, not an integer"),
    }
}

/// A JSON string field.
pub fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} is {}, not a string", value[field]))
}

/// The JSON array at `field`, empty when absent.
pub fn array<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// A rational such as `1/12800`, as (numerator, denominator).
pub fn ratio(value: &str, separator: char) -> (u64, u64) {
    let (num, den) = value
        .split_once(separator)
        .unwrap_or_else(|| panic!("{value:?} is not a ratio"));
    (
        num.parse()
            .unwrap_or_else(|_| panic!("{value:?} has a bad numerator")),
        den.parse()
            .unwrap_or_else(|_| panic!("{value:?} has a bad denominator")),
    )
}

/// The track id in an ffprobe stream `id` such as `0x1`.
pub fn stream_track_id(stream: &Value) -> u32 {
    let id = text(stream, "id");
    let hex = id
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("stream id {id:?} is not hex"));
    u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("stream id {id:?} is not hex"))
}

/// The Adler-32 checksum, as ffprobe `-show_data_hash adler32` prints it.
pub fn adler32(bytes: &[u8]) -> String {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    format!("adler32:{:08x}", (b << 16) | a)
}

/// The top-level boxes of a file: type, offset and length.
pub fn top_level(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        let kind: [u8; 4] = bytes[pos + 4..pos + 8].try_into().expect("4 bytes");
        let len = match size {
            0 => bytes.len() - pos,
            1 => u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().expect("8 bytes")) as usize,
            n => n,
        };
        out.push((kind, pos, len));
        pos += len.max(8);
    }
    out
}

/// The byte range of the top-level `moov` box.
pub fn moov_range(bytes: &[u8]) -> std::ops::Range<usize> {
    top_level(bytes)
        .into_iter()
        .find(|(kind, _, _)| kind == b"moov")
        .map(|(_, pos, len)| pos..pos + len)
        .expect("the fixture has a moov box")
}
