//! Stream copy helpers: remux a fixture with the public API.

use std::io::Cursor;
use std::path::PathBuf;

use containr::mkv::{
    AttachmentIn, Demuxer, DocType, MkvOptions, Muxer, PacketIn, StreamCopy, TrackId, tags_from_mp4,
};
use containr::{Chapter, ChapterSource, Error, Packet, Result, Track};

use crate::common::{self, Fixture};

/// The Matroska demuxer over bytes in memory.
pub type MkvDemuxer = Demuxer<Cursor<Vec<u8>>>;
/// The MP4 demuxer over bytes in memory.
pub type Mp4Demuxer = containr::mp4::Demuxer<Cursor<Vec<u8>>>;

/// One track the remux copied.
pub struct Copied {
    pub source: Track,
    pub id: TrackId,
    pub plan: StreamCopy,
    pub packets: Vec<Packet>,
}

/// A remuxed MP4 fixture.
pub struct Remux {
    pub fixture: Fixture,
    pub bytes: Vec<u8>,
    pub copied: Vec<Copied>,
    /// The tracks refused by name, with their errors.
    pub refused: Vec<(u32, Error)>,
    pub scale: u64,
}

pub fn open_mkv(bytes: &[u8]) -> MkvDemuxer {
    Demuxer::open(Cursor::new(bytes.to_vec())).unwrap_or_else(|err| panic!("{err}"))
}

pub fn open_mp4(bytes: &[u8]) -> Mp4Demuxer {
    containr::mp4::Demuxer::open(Cursor::new(bytes.to_vec())).unwrap_or_else(|err| panic!("{err}"))
}

/// Unwrap a result, naming the fixture.
pub fn ok<T>(name: &str, result: Result<T>) -> T {
    result.unwrap_or_else(|err| panic!("{name}: {err}"))
}

pub fn collect(name: &str, packets: impl Iterator<Item = Result<Packet>>) -> Vec<Packet> {
    packets.map(|packet| ok(name, packet)).collect()
}

/// A demuxed Matroska file copied whole, with the options `tweak` leaves.
pub fn copy_mkv(name: &str, input: &[u8], tweak: impl FnOnce(&mut MkvOptions)) -> Vec<u8> {
    let mut input = open_mkv(input);
    let mut options = MkvOptions {
        doc_type: if input.ebml_header().doc_type == "webm" {
            DocType::WebM
        } else {
            DocType::Matroska
        },
        timestamp_scale: input.info().timestamp_scale,
        title: input.info().title.clone(),
        ..MkvOptions::default()
    };
    tweak(&mut options);
    let mut muxer = ok(name, Muxer::new(Cursor::new(Vec::new()), options));
    let mut plans = Vec::new();
    for track in input.tracks() {
        let packets = collect(name, ok(name, input.track_packets(track.id)));
        let plan = ok(name, StreamCopy::plan(track, &packets));
        let id = ok(name, muxer.add_track(plan.spec.clone()));
        plans.push((track.id, id, plan));
    }
    ok(name, muxer.set_metadata(input.metadata().to_vec()));
    ok(name, muxer.set_chapters(input.chapters().to_vec()));
    for attachment in input.attachments().to_vec() {
        let data = ok(name, input.read_attachment(&attachment));
        let file = AttachmentIn {
            name: &attachment.name,
            mime: &attachment.mime,
            description: attachment.description.as_deref(),
            uid: Some(attachment.uid),
            data: &data,
        };
        ok(name, muxer.attach(&file));
    }
    for packet in input.packets() {
        let packet = ok(name, packet);
        let data = ok(name, input.read_packet(&packet));
        let (_, id, plan) = plans
            .iter()
            .find(|(source, _, _)| *source == packet.track_id)
            .unwrap_or_else(|| panic!("{name}: packet of unknown track {}", packet.track_id));
        let packet_in: Option<PacketIn<'_>> = ok(name, plan.packet(&packet, &data));
        let packet_in = packet_in.unwrap_or_else(|| panic!("{name}: a Matroska packet is skipped"));
        ok(name, muxer.write_packet(*id, &packet_in));
    }
    ok(name, muxer.finish()).into_inner()
}

/// Remux an MP4 or MOV fixture: every track that maps but the chapter track,
/// the movie tags and the chapters. `None` when the generator has not run.
pub fn remux_mp4(name: &str) -> Option<Remux> {
    let fixture = common::fixture(name)?;
    let mut input = open_mp4(&fixture.bytes);
    let mut planned = Vec::new();
    let mut refused = Vec::new();
    for track in input.tracks().iter().filter(|track| !track.chapter_track) {
        let packets = collect(name, ok(name, input.track_packets(track.id)));
        match StreamCopy::plan(track, &packets) {
            Ok(plan) => planned.push((track.clone(), plan, packets)),
            Err(err @ Error::Unsupported { .. }) => refused.push((track.id, err)),
            Err(err) => panic!("{name}: track {}: {err}", track.id),
        }
    }
    let plans: Vec<StreamCopy> = planned.iter().map(|(_, plan, _)| plan.clone()).collect();
    let scale = StreamCopy::timestamp_scale(&plans);
    let options = MkvOptions {
        timestamp_scale: scale,
        ..MkvOptions::default()
    };
    let mut muxer = ok(name, Muxer::new(Cursor::new(Vec::new()), options));
    let mut copied = Vec::new();
    for (source, plan, packets) in planned {
        let id = ok(name, muxer.add_track(plan.spec.clone()));
        copied.push(Copied {
            source,
            id,
            plan,
            packets,
        });
    }
    let tags = ok(name, tags_from_mp4(input.metadata()));
    ok(name, muxer.set_metadata(tags));
    ok(
        name,
        muxer.set_chapters(chapters_to_write(input.chapters())),
    );
    for packet in input.packets() {
        let packet = ok(name, packet);
        let Some(track) = copied.iter().find(|c| c.source.id == packet.track_id) else {
            continue;
        };
        let data = ok(name, input.read_packet(&packet));
        if let Some(packet_in) = ok(name, track.plan.packet(&packet, &data)) {
            ok(name, muxer.write_packet(track.id, &packet_in));
        }
    }
    let bytes = ok(name, muxer.finish()).into_inner();
    Some(Remux {
        fixture,
        bytes,
        copied,
        refused,
        scale,
    })
}

/// One set of chapters: an MP4 file repeats them in `chpl` and in a chapter
/// track. The track's set has durations, so it wins.
pub fn chapters_to_write(chapters: &[Chapter]) -> Vec<Chapter> {
    let from_track: Vec<Chapter> = chapters
        .iter()
        .filter(|chapter| matches!(chapter.source, ChapterSource::Track(_)))
        .cloned()
        .collect();
    if from_track.is_empty() {
        chapters.to_vec()
    } else {
        from_track
    }
}

/// Write `bytes` to `name` in the gitignored fixture directory.
pub fn write_output(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mkv/mux");
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("creating {}: {err}", dir.display()));
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|err| panic!("writing {}: {err}", path.display()));
    path
}
