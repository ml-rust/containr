//! Stream copy helpers: remux a fixture with the public API.

use std::io::Cursor;
use std::path::PathBuf;

use containr::mp4::{Brand, Demuxer, MuxOptions, Muxer, StreamCopy, TrackId};
use containr::{Error, FourCc, Packet, Result, Track};

use crate::common::{self, Fixture};

/// The MP4 demuxer over bytes in memory.
pub type Mp4Demuxer = Demuxer<Cursor<Vec<u8>>>;
/// The Matroska demuxer over bytes in memory.
pub type MkvDemuxer = containr::mkv::Demuxer<Cursor<Vec<u8>>>;

/// One track the remux copied.
pub struct Copied {
    pub source: Track,
    pub id: TrackId,
    pub plan: StreamCopy,
    pub packets: Vec<Packet>,
}

/// A remuxed fixture.
pub struct Remux {
    pub fixture: Fixture,
    pub bytes: Vec<u8>,
    pub copied: Vec<Copied>,
    /// The tracks refused by name, with their errors.
    pub refused: Vec<(u32, Error)>,
    pub movie_timescale: u32,
}

impl Remux {
    /// The output, opened with the demuxer.
    pub fn output(&self) -> Mp4Demuxer {
        open_mp4(&self.bytes)
    }
}

pub fn open_mp4(bytes: &[u8]) -> Mp4Demuxer {
    Demuxer::open(Cursor::new(bytes.to_vec())).unwrap_or_else(|err| panic!("{err}"))
}

pub fn open_mkv(bytes: &[u8]) -> MkvDemuxer {
    containr::mkv::Demuxer::open(Cursor::new(bytes.to_vec())).unwrap_or_else(|err| panic!("{err}"))
}

/// Unwrap a mux result, naming the fixture.
pub fn ok<T>(name: &str, result: Result<T>) -> T {
    result.unwrap_or_else(|err| panic!("{name}: {err}"))
}

pub fn collect(name: &str, packets: impl Iterator<Item = Result<Packet>>) -> Vec<Packet> {
    packets.map(|packet| ok(name, packet)).collect()
}

/// Remux an MP4 or MOV fixture: every track but the chapter track, the
/// movie metadata and the chapters. `None` when the generator has not run.
pub fn remux_mp4(name: &str, faststart: bool) -> Option<Remux> {
    let fixture = common::fixture(name)?;
    let mut input = open_mp4(&fixture.bytes);
    let brand = if input.major_brand() == Some(FourCc(*b"qt  ")) {
        Brand::QuickTime
    } else {
        Brand::Isom
    };
    let movie_timescale = input.movie_timescale();
    let options = MuxOptions {
        brand,
        faststart,
        movie_timescale,
        ..MuxOptions::default()
    };
    let mut muxer = ok(name, Muxer::new(Cursor::new(Vec::new()), options));
    let mut copied = Vec::new();
    for track in input.tracks().iter().filter(|track| !track.chapter_track) {
        let packets = collect(name, ok(name, input.track_packets(track.id)));
        let plan = ok(name, StreamCopy::plan(track, &packets, movie_timescale));
        let id = ok(name, muxer.add_track(plan.spec.clone()));
        copied.push(Copied {
            source: track.clone(),
            id,
            plan,
            packets,
        });
    }
    ok(name, muxer.set_metadata(input.metadata().to_vec()));
    ok(
        name,
        muxer.set_chapters(StreamCopy::chapters(input.chapters())),
    );
    for packet in input.packets() {
        let packet = ok(name, packet);
        let Some(track) = copied.iter().find(|c| c.source.id == packet.track_id) else {
            continue;
        };
        let data = ok(name, input.read_packet(&packet));
        let packet_in = ok(name, track.plan.packet(&packet, &data));
        ok(name, muxer.write_packet(track.id, &packet_in));
    }
    let bytes = ok(name, muxer.finish()).into_inner();
    Some(Remux {
        fixture,
        bytes,
        copied,
        refused: Vec::new(),
        movie_timescale,
    })
}

/// Remux a Matroska or WebM fixture: every track that maps, the segment tags
/// and the chapters. `None` when the generator has not run.
pub fn remux_mkv(name: &str, faststart: bool) -> Option<Remux> {
    let fixture = common::fixture(name)?;
    let mut input = open_mkv(&fixture.bytes);
    let options = MuxOptions {
        faststart,
        ..MuxOptions::default()
    };
    let movie_timescale = options.movie_timescale;
    let mut muxer = ok(name, Muxer::new(Cursor::new(Vec::new()), options));
    let mut copied = Vec::new();
    let mut refused = Vec::new();
    for track in input.tracks() {
        let packets = collect(name, ok(name, input.track_packets(track.id)));
        match StreamCopy::plan(track, &packets, movie_timescale) {
            Ok(plan) => {
                let id = ok(name, muxer.add_track(plan.spec.clone()));
                copied.push(Copied {
                    source: track.clone(),
                    id,
                    plan,
                    packets,
                });
            }
            Err(err @ Error::Unsupported { .. }) => refused.push((track.id, err)),
            Err(err) => panic!("{name}: track {}: {err}", track.id),
        }
    }
    ok(name, muxer.set_metadata(input.metadata().to_vec()));
    ok(name, muxer.set_chapters(input.chapters().to_vec()));
    for packet in input.packets() {
        let packet = ok(name, packet);
        let Some(track) = copied.iter().find(|c| c.source.id == packet.track_id) else {
            continue;
        };
        let data = ok(name, input.read_packet(&packet));
        let packet_in = ok(name, track.plan.packet(&packet, &data));
        ok(name, muxer.write_packet(track.id, &packet_in));
    }
    let bytes = ok(name, muxer.finish()).into_inner();
    Some(Remux {
        fixture,
        bytes,
        copied,
        refused,
        movie_timescale,
    })
}

/// Write `bytes` to `name` in the gitignored fixture directory.
pub fn write_output(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mp4/mux");
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("creating {}: {err}", dir.display()));
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|err| panic!("writing {}: {err}", path.display()));
    path
}
