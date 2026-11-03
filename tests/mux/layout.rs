//! Layout: `moov` placement, the faststart rewrite, and offsets past 4 GiB.

use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use containr::TrackKind;
use containr::mp4::{
    Demuxer, MAX_CHUNK_BYTES, MuxOptions, Muxer, PacketIn, SampleEntrySpec, TrackSpec, VisualCodec,
};

use crate::common::top_level;
use crate::mux::support::{collect, ok, open_mp4, remux_mp4};

fn kinds(bytes: &[u8]) -> Vec<[u8; 4]> {
    top_level(bytes)
        .into_iter()
        .map(|(kind, _, _)| kind)
        .collect()
}

#[test]
fn without_faststart_moov_follows_mdat() {
    let Some(remux) = remux_mp4("h264_aac_moov_end.mp4", false) else {
        return;
    };
    assert_eq!(
        kinds(&remux.bytes),
        [*b"ftyp", *b"free", *b"mdat", *b"moov"]
    );
}

#[test]
fn faststart_moves_moov_before_mdat_and_shifts_every_offset() {
    let name = "h264_aac_moov_end.mp4";
    let (Some(end), Some(fast)) = (remux_mp4(name, false), remux_mp4(name, true)) else {
        return;
    };
    assert_eq!(kinds(&fast.bytes), [*b"ftyp", *b"moov", *b"free", *b"mdat"]);
    let moov_len = crate::common::moov_range(&fast.bytes).len() as u64;
    assert_eq!(fast.bytes.len(), end.bytes.len());
    let end_out = open_mp4(&end.bytes);
    let mut fast_out = open_mp4(&fast.bytes);
    for track in end_out.tracks() {
        let plain = collect(name, ok(name, end_out.track_packets(track.id)));
        let moved = collect(name, ok(name, fast_out.track_packets(track.id)));
        assert_eq!(plain.len(), moved.len(), "track {}", track.id);
        for (plain, moved) in plain.iter().zip(&moved) {
            assert_eq!(moved.offset, plain.offset + moov_len, "track {}", track.id);
            let at = plain.offset as usize;
            let want = &end.bytes[at..at + plain.size as usize];
            assert_eq!(
                ok(name, fast_out.read_packet(moved)),
                want,
                "track {}",
                track.id
            );
        }
    }
}

/// A target that keeps only short writes. A write of one chunk or more holds
/// zeros in these tests, and reads back as zeros, so a file past 4 GiB costs
/// no disk and no memory.
#[derive(Debug, Default)]
struct SparseTarget {
    pos: u64,
    len: u64,
    stored: Vec<(u64, Vec<u8>)>,
}

impl Write for SparseTarget {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() < MAX_CHUNK_BYTES {
            self.stored.push((self.pos, buf.to_vec()));
        }
        self.pos += buf.len() as u64;
        self.len = self.len.max(self.pos);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for SparseTarget {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.pos = match from {
            SeekFrom::Start(at) => at,
            SeekFrom::End(delta) => self.len.wrapping_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.wrapping_add_signed(delta),
        };
        Ok(self.pos)
    }
}

impl Read for SparseTarget {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = (self.len.saturating_sub(self.pos)).min(buf.len() as u64) as usize;
        let out = &mut buf[..count];
        out.copy_from_slice(&vec![0u8; count]);
        for (at, bytes) in &self.stored {
            let start = self.pos.max(*at);
            let end = (self.pos + count as u64).min(*at + bytes.len() as u64);
            if start < end {
                out[(start - self.pos) as usize..(end - self.pos) as usize]
                    .copy_from_slice(&bytes[(start - at) as usize..(end - at) as usize]);
            }
        }
        self.pos += count as u64;
        Ok(count)
    }
}

/// Packets of one chunk each, past 4 GiB of `mdat`.
const PACKETS: u64 = (5 << 30) / MAX_CHUNK_BYTES as u64;

fn write_past_4_gib(faststart: bool) -> SparseTarget {
    let options = MuxOptions {
        faststart,
        ..MuxOptions::default()
    };
    let mut muxer = ok("co64", Muxer::new(SparseTarget::default(), options));
    let entry = SampleEntrySpec::Visual {
        codec: VisualCodec::Avc,
        config: vec![1, 2, 3],
        width: 96,
        height: 64,
    };
    let id = ok(
        "co64",
        muxer.add_track(TrackSpec::new(TrackKind::Video, 25, entry)),
    );
    let data = vec![0u8; MAX_CHUNK_BYTES];
    for n in 0..PACKETS as i64 {
        let packet = PacketIn {
            data: &data,
            pts: n,
            dts: n,
            duration: Some(1),
            sync: true,
        };
        ok("co64", muxer.write_packet(id, &packet));
    }
    ok("co64", muxer.finish())
}

fn check_past_4_gib(faststart: bool) {
    let mut demuxer = ok("co64", Demuxer::open(write_past_4_gib(faststart)));
    let id = demuxer.tracks()[0].id;
    let packets = collect("co64", ok("co64", demuxer.track_packets(id)));
    assert_eq!(packets.len() as u64, PACKETS);
    let first = packets[0].offset;
    for (n, packet) in packets.iter().enumerate() {
        assert_eq!(packet.offset, first + n as u64 * MAX_CHUNK_BYTES as u64);
    }
    let last = packets.last().expect("the track has packets");
    assert!(last.offset > u64::from(u32::MAX), "offset {}", last.offset);
    let bytes = ok("co64", demuxer.read_packet(last));
    assert_eq!(bytes.len(), MAX_CHUNK_BYTES);
    assert_eq!(demuxer.track(id).and_then(|t| t.duration), Some(PACKETS));
}

#[test]
fn offsets_past_4_gib_use_co64_and_a_large_mdat() {
    check_past_4_gib(false);
}

#[test]
fn faststart_past_4_gib_uses_co64_and_a_large_mdat() {
    check_past_4_gib(true);
}

/// A movie with no tracks still has a whole layout.
#[test]
fn an_empty_movie_is_valid() {
    let muxer = ok(
        "empty",
        Muxer::new(Cursor::new(Vec::new()), MuxOptions::default()),
    );
    let bytes = ok("empty", muxer.finish()).into_inner();
    assert_eq!(kinds(&bytes), [*b"ftyp", *b"free", *b"mdat", *b"moov"]);
    assert!(open_mp4(&bytes).tracks().is_empty());
}
