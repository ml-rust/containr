//! A randomized sweep: any sequence of packets gives `Ok` or `Err`, never a
//! panic, and whatever the muxer accepts reads back as written.

use std::io::Cursor;

use containr::matroska::{DisplayUnit, MatroskaVideo};
use containr::mkv::{Demuxer, DocType, MkvOptions, Muxer, PacketIn, TrackSpec};
use containr::{Chapter, ChapterSource, Result, Timebase, TrackKind};

use crate::mkvmux::walk::walk;

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
            4 => base.saturating_add(70_000),
            5 => base.saturating_sub(self.below(1000) as i64),
            _ => base.saturating_add(self.below(1000) as i64),
        }
    }

    fn duration(&mut self) -> Option<u64> {
        match self.below(8) {
            0 | 1 => None,
            2 => Some(0),
            3 => Some(u64::MAX),
            _ => Some(self.below(1000)),
        }
    }
}

fn spec(random: &mut Random) -> TrackSpec {
    let timebase = [
        Timebase::from_nanoseconds(1_000_000),
        Timebase::from_nanoseconds(1),
        Timebase::from_timescale(1000),
        Timebase::from_timescale(12_800),
        Timebase::from_timescale(48_000),
        Timebase {
            num: 3,
            den: 1_000_000_000,
        },
    ][random.below(6) as usize];
    let mut spec = match random.below(3) {
        0 => {
            let mut spec = TrackSpec::new(TrackKind::Video, "V_VP9", timebase);
            spec.matroska.video = Some(MatroskaVideo {
                pixel_width: 16,
                pixel_height: 16,
                display_width: None,
                display_height: None,
                display_unit: DisplayUnit::Pixels,
                crop: [0; 4],
                interlaced: 0,
                projection: None,
                extra: Vec::new(),
            });
            spec
        }
        1 => TrackSpec::new(TrackKind::Audio, "A_OPUS", timebase),
        _ => TrackSpec::new(TrackKind::Subtitle, "S_TEXT/WEBVTT", timebase),
    };
    if random.below(2) == 0 {
        spec.matroska.default_duration = Some(1 + random.below(50_000_000));
    }
    spec
}

/// One accepted packet of one track, in output ticks.
struct Written {
    pts: i64,
    duration: Option<u64>,
    size: u32,
    sync: bool,
    discardable: bool,
    padding: Option<i64>,
}

/// `value` timebase ticks in output ticks. The muxer accepted it, so it is
/// exact.
fn ticks(value: i128, timebase: Timebase, scale: u64) -> i128 {
    value * i128::from(timebase.num) * 1_000_000_000 / i128::from(timebase.den) / i128::from(scale)
}

fn run(seed: u64) -> Result<()> {
    let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let scale = [1u64, 1000, 500_000, 1_000_000, u64::from(u32::MAX)][random.below(5) as usize];
    let cluster_bytes = [containr::mkv::MIN_CLUSTER_BYTES, 1 << 16][random.below(2) as usize];
    let options = MkvOptions {
        doc_type: if random.below(2) == 0 {
            DocType::Matroska
        } else {
            DocType::WebM
        },
        timestamp_scale: scale,
        cluster_duration_ms: [1, 500, 60_000][random.below(3) as usize],
        cluster_bytes,
        max_buffer: cluster_bytes,
        cues: random.below(2) == 0,
        seekable: true,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options)?;
    let mut ids = Vec::new();
    let mut specs = Vec::new();
    for _ in 0..1 + random.below(3) {
        let spec = spec(&mut random);
        ids.push(muxer.add_track(spec.clone())?);
        specs.push(spec);
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
        let packet = PacketIn {
            data: &data[..size],
            pts: random.timestamp(bases[slot]),
            duration: random.duration(),
            sync: random.below(2) == 0,
            discardable: random.below(4) == 0,
            discard_padding: match random.below(6) {
                0 => Some(-5),
                1 => Some(5),
                _ => None,
            },
        };
        if muxer.write_packet(ids[slot], &packet).is_ok() {
            bases[slot] = packet.pts;
            let timebase = specs[slot].timebase;
            written[slot].push(Written {
                pts: ticks(i128::from(packet.pts), timebase, scale) as i64,
                duration: packet
                    .duration
                    .map(|duration| ticks(i128::from(duration), timebase, scale) as u64),
                size: size as u32,
                sync: packet.sync,
                discardable: packet.discardable,
                padding: packet.discard_padding,
            });
        }
    }
    let bytes = muxer.finish()?.into_inner();
    // A file the muxer finished must read back, so these errors fail the test.
    let layout = walk(&bytes);
    let mut demuxer = Demuxer::open(Cursor::new(bytes))
        .unwrap_or_else(|err| panic!("seed {seed}: the output does not read: {err}"));
    assert!(!demuxer.truncated(), "seed {seed}");
    for (slot, id) in ids.iter().enumerate() {
        let default_ticks = specs[slot]
            .matroska
            .default_duration
            .map(|nanoseconds| nanoseconds / scale);
        let packets: Vec<_> = demuxer
            .track_packets(id.get())
            .and_then(|packets| packets.collect::<Result<Vec<_>>>())
            .unwrap_or_else(|err| panic!("seed {seed}: the packets do not read: {err}"));
        assert_eq!(
            packets.len(),
            written[slot].len(),
            "seed {seed} slot {slot}"
        );
        for (got, want) in packets.iter().zip(&written[slot]) {
            assert_eq!(got.pts, want.pts, "seed {seed}");
            assert_eq!(got.duration, want.duration.or(default_ticks), "seed {seed}");
            assert_eq!(got.size, want.size, "seed {seed}");
            assert_eq!(got.discardable, want.discardable, "seed {seed}");
            assert_eq!(got.discard_padding, want.padding, "seed {seed}");
            assert_eq!(got.sync, want.sync, "seed {seed}");
        }
        for packet in &packets {
            demuxer
                .read_packet(packet)
                .unwrap_or_else(|err| panic!("seed {seed}: the bytes do not read: {err}"));
        }
        if let Some(last) = packets.last() {
            let stored = layout
                .last_block(u64::from(id.get()))
                .unwrap_or_else(|| panic!("seed {seed}: no block for slot {slot}"));
            assert_eq!(stored.duration, last.duration, "seed {seed}: last block");
            assert!(stored.duration.is_some(), "seed {seed}: explicit last");
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
    assert!(finished > 100, "only {finished} random files finished");
}
