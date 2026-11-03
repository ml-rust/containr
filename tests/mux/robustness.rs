//! A randomized sweep: any sequence of packets gives `Ok` or `Err`, never a
//! panic, and whatever the muxer accepts reads back as written.

use std::io::Cursor;

use containr::mp4::{
    AudioCodec, Brand, Demuxer, MuxOptions, Muxer, PacketFormat, PacketIn, SampleEntrySpec,
    TrackSpec, VisualCodec,
};
use containr::{Chapter, ChapterSource, Result, TrackKind};

/// A small xorshift generator. The sweep stays the same on every run.
struct Random(u64);

impl Random {
    fn step(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.step() % limit
    }

    /// A timestamp: mostly near `base`, sometimes an extreme value.
    fn timestamp(&mut self, base: i64) -> i64 {
        match self.below(16) {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => -1,
            3 => 0,
            4 => base.saturating_add(i64::from(u32::MAX)),
            5 => base.saturating_sub(self.below(1000) as i64),
            _ => base.saturating_add(self.below(1000) as i64),
        }
    }

    fn duration(&mut self) -> Option<u64> {
        match self.below(8) {
            0 => None,
            1 => Some(0),
            2 => Some(u64::MAX),
            3 => Some(u64::from(u32::MAX) + 1),
            _ => Some(self.below(1000)),
        }
    }
}

fn spec(kind: u64, random: &mut Random) -> TrackSpec {
    let timescale = [1u32, 1000, 48_000, 90_000, u32::MAX][random.below(5) as usize];
    match kind {
        0 => TrackSpec::new(
            TrackKind::Video,
            timescale,
            SampleEntrySpec::Visual {
                codec: VisualCodec::Hevc,
                config: vec![1],
                width: 16,
                height: 16,
            },
        ),
        1 => TrackSpec::new(
            TrackKind::Audio,
            timescale,
            SampleEntrySpec::Audio {
                codec: AudioCodec::Opus,
                config: vec![0; 11],
                channels: 2,
                sample_rate: 48_000,
            },
        ),
        _ => {
            let mut spec = TrackSpec::new(TrackKind::Subtitle, timescale, SampleEntrySpec::Tx3g);
            spec.packet_format = PacketFormat::PlainText;
            spec
        }
    }
}

/// One accepted packet of one track.
struct Written {
    pts: i64,
    dts: i64,
    size: u32,
    sync: bool,
}

fn run(seed: u64) -> Result<()> {
    let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let options = MuxOptions {
        brand: [Brand::Isom, Brand::Mp41, Brand::Mp42, Brand::QuickTime][random.below(4) as usize],
        faststart: random.below(2) == 0,
        chunk_duration_ms: [1, 500, 60_000][random.below(3) as usize],
        max_buffer: containr::mp4::MAX_CHUNK_BYTES,
        ..MuxOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options)?;
    let tracks = 1 + random.below(3);
    let mut ids = Vec::new();
    let mut kinds = Vec::new();
    for _ in 0..tracks {
        let kind = random.below(3);
        ids.push(muxer.add_track(spec(kind, &mut random))?);
        kinds.push(kind);
    }
    if random.below(4) == 0 {
        // The muxer can refuse chapters: the sweep only needs the outcome.
        let _ = muxer.set_chapters(vec![Chapter {
            start: random.step() >> 40,
            duration: None,
            timescale: 1 + random.below(1000) as u32,
            title: "c".to_owned(),
            source: ChapterSource::Chpl,
        }]);
    }
    let mut bases = vec![0i64; ids.len()];
    let mut written: Vec<Vec<Written>> = ids.iter().map(|_| Vec::new()).collect();
    let data = vec![5u8; 70_000];
    for _ in 0..random.below(60) {
        let slot = random.below(ids.len() as u64) as usize;
        let size = match random.below(4) {
            0 => 0,
            1 => 2,
            2 => random.below(70_000) as usize,
            _ => random.below(300) as usize,
        };
        let dts = random.timestamp(bases[slot]);
        let packet = PacketIn {
            data: &data[..size],
            pts: if random.below(2) == 0 {
                dts
            } else {
                random.timestamp(dts)
            },
            dts,
            duration: random.duration(),
            sync: random.below(2) == 0,
        };
        if muxer.write_packet(ids[slot], &packet).is_ok() {
            bases[slot] = packet.dts;
            written[slot].push(Written {
                pts: packet.pts,
                dts: packet.dts,
                size: size as u32,
                sync: packet.sync,
            });
        }
    }
    let bytes = muxer.finish()?.into_inner();
    // A file the muxer finished must read back, so these errors fail the test.
    let mut demuxer = Demuxer::open(Cursor::new(bytes))
        .unwrap_or_else(|err| panic!("seed {seed}: the output does not read: {err}"));
    for (slot, id) in ids.iter().enumerate() {
        let packets: Vec<_> = demuxer
            .track_packets(id.get())
            .and_then(|packets| packets.collect::<Result<Vec<_>>>())
            .unwrap_or_else(|err| panic!("seed {seed}: the packets do not read: {err}"));
        if kinds[slot] == 2 {
            // Text gaps add samples. Each cue keeps its timing and framing.
            for want in &written[slot] {
                let got = packets
                    .iter()
                    .find(|p| p.dts == Some(want.dts) && p.size == want.size + 2)
                    .unwrap_or_else(|| panic!("seed {seed}: cue at dts {} is lost", want.dts));
                assert_eq!(got.pts, want.pts, "seed {seed}");
            }
            continue;
        }
        assert_eq!(
            packets.len(),
            written[slot].len(),
            "seed {seed} slot {slot}"
        );
        for (got, want) in packets.iter().zip(&written[slot]) {
            assert_eq!(got.dts, Some(want.dts), "seed {seed}");
            assert_eq!(got.pts, want.pts, "seed {seed}");
            assert_eq!(got.size, want.size, "seed {seed}");
            assert_eq!(got.sync, want.sync, "seed {seed}");
        }
        for packet in &packets {
            demuxer
                .read_packet(packet)
                .unwrap_or_else(|err| panic!("seed {seed}: the bytes do not read: {err}"));
        }
    }
    Ok(())
}

#[test]
fn the_muxer_never_panics_and_reads_back_what_it_accepted() {
    let mut finished = 0;
    for seed in 0..2000 {
        if run(seed).is_ok() {
            finished += 1;
        }
    }
    assert!(finished > 100, "only {finished} random movies finished");
}

#[test]
fn every_option_error_names_its_field() {
    for (options, field) in [
        (
            MuxOptions {
                movie_timescale: 0,
                ..MuxOptions::default()
            },
            "movie_timescale",
        ),
        (
            MuxOptions {
                chunk_duration_ms: 0,
                ..MuxOptions::default()
            },
            "chunk_duration_ms",
        ),
        (
            MuxOptions {
                max_buffer: 1,
                ..MuxOptions::default()
            },
            "max_buffer",
        ),
    ] {
        match Muxer::new(Cursor::new(Vec::new()), options) {
            Err(err) => assert!(err.to_string().contains(field), "{err}"),
            Ok(_) => panic!("{field}: the options are not refused"),
        }
    }
}
