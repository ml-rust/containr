//! A randomized sweep of `StreamCopy::plan_rounded`: any MP4 track gives a plan
//! or a named refusal, never a panic. Each plan keeps every rounding promise,
//! and the muxer's output reads back as planned.

use std::io::Cursor;

use containr::mkv::{Demuxer, MkvOptions, Muxer, StreamCopy};
use containr::{EditList, Error, Packet, Result, Track, TrackKind};

use crate::mkvmux::mapping::{edit, entry, packet, track};
use crate::mkvmux::support::within_half_a_tick;

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

    fn pick<T: Copy>(&mut self, list: &[T]) -> T {
        list[self.below(list.len() as u64) as usize]
    }
}

/// A track with B-frame style reordering, a run of durations, and maybe edits.
fn source(random: &mut Random) -> (Track, Vec<Packet>) {
    let timescale = random.pick(&[1000u32, 12_800, 24_000, 30_000, 44_100, 48_000, 90_000]);
    let frame = random.pick(&[1u64, 512, 1001, 1024, 1500, 3003]);
    let mut source = track(TrackKind::Video, timescale, vec![entry(b"vp09", None)]);
    let count = 1 + random.below(60);
    let reorder = random.below(2) == 0;
    let packets = (0..count)
        .map(|n| {
            let swap = match (reorder, n % 3) {
                (true, 1) => 2,
                (true, 2) => -1,
                (true, 0) if n > 0 => -1,
                _ => 0,
            };
            let pts = (n as i64 + swap) * frame as i64;
            let duration = if random.below(16) == 0 {
                frame / 2
            } else {
                frame
            };
            let mut packet = packet(n, pts, duration, 1);
            packet.sync = random.below(2) == 0;
            packet
        })
        .collect();
    source.edit_list = match random.below(4) {
        0 => EditList {
            movie_timescale: random.pick(&[7u32, 1000, 600, 90_000]),
            entries: vec![edit(1 + random.below(1000), -1), edit(0, 0)],
        },
        1 => EditList {
            movie_timescale: 1000,
            entries: vec![edit(0, frame as i64)],
        },
        _ => EditList::default(),
    };
    (source, packets)
}

/// `value / per_second` seconds, against `ticks` of `scale` ns: the absolute
/// error at most half a tick.
fn assert_within_half_a_tick(seed: u64, ticks: i64, scale: u64, value: i128, per_second: i128) {
    assert!(
        within_half_a_tick(i128::from(ticks), scale, value, per_second),
        "seed {seed}: tick {ticks} at scale {scale} against {value} over {per_second}"
    );
}

/// The start of every packet, in units of 1 / `per_second` seconds.
fn exact_times(source: &Track, packets: &[Packet]) -> (Vec<i128>, i128) {
    let list = &source.edit_list;
    let movie = i128::from(list.movie_timescale.max(1));
    let timescale = i128::from(source.timebase.den);
    let mut shift = 0;
    for entry in &list.entries {
        if entry.is_empty() {
            shift += i128::from(entry.segment_duration) * timescale;
        } else {
            shift -= i128::from(entry.media_time) * movie;
        }
    }
    let times = packets
        .iter()
        .map(|packet| i128::from(packet.pts) * movie + shift)
        .collect();
    (times, timescale * movie)
}

fn check_plan(seed: u64, source: &Track, packets: &[Packet], plan: &StreamCopy, scale: u64) {
    let (times, per_second) = exact_times(source, packets);
    let blocks: Vec<_> = plan.timing().iter().map(|t| t.expect("a block")).collect();
    assert_eq!(blocks.len(), packets.len(), "seed {seed}: decode order");
    let rounding = plan.rounding().expect("a rounded plan");
    assert_eq!(rounding.scale, scale, "seed {seed}");
    assert!(
        rounding.max_error_ns <= scale / 2 + 1,
        "seed {seed}: {} ns at scale {scale}",
        rounding.max_error_ns
    );
    assert_eq!(
        StreamCopy::timestamp_scale(std::slice::from_ref(plan)),
        scale,
        "seed {seed}"
    );
    for (block, &time) in blocks.iter().zip(&times) {
        assert_within_half_a_tick(seed, block.pts, scale, time, per_second);
    }
    // In presentation order, each block ends where the next starts, and the
    // last ends within half a tick of its exact end.
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by_key(|&n| (times[n], n));
    for pair in order.windows(2) {
        let (this, next) = (blocks[pair[0]], blocks[pair[1]]);
        assert!(this.pts < next.pts, "seed {seed}: two packets on one tick");
        assert_eq!(
            this.pts + this.duration.expect("a duration") as i64,
            next.pts,
            "seed {seed}: no gap and no overlap"
        );
    }
    let last = *order.last().expect("a packet");
    let movie = per_second / i128::from(source.timebase.den);
    let span = i128::from(packets[last].duration.expect("a duration"));
    let end = blocks[last].pts + blocks[last].duration.expect("a duration") as i64;
    assert_within_half_a_tick(seed, end, scale, times[last] + span * movie, per_second);
}

/// Mux `plan`, read it back, and compare with the plan.
fn check_output(seed: u64, packets: &[Packet], plan: &StreamCopy, scale: u64) -> Result<()> {
    let options = MkvOptions {
        timestamp_scale: scale,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options)?;
    let id = muxer.add_track(plan.spec.clone())?;
    for packet in packets {
        if let Some(block) = plan.packet(packet, &[1, 2, 3])? {
            muxer.write_packet(id, &block)?;
        }
    }
    let bytes = muxer.finish()?.into_inner();
    let demuxer = Demuxer::open(Cursor::new(bytes))
        .unwrap_or_else(|err| panic!("seed {seed}: the output does not read: {err}"));
    let read: Vec<Packet> = demuxer
        .track_packets(id.get())?
        .collect::<Result<_>>()
        .unwrap_or_else(|err| panic!("seed {seed}: the packets do not read: {err}"));
    assert_eq!(read.len(), packets.len(), "seed {seed}");
    for ((got, want), source) in read.iter().zip(plan.timing().iter().flatten()).zip(packets) {
        assert_eq!(got.pts, want.pts, "seed {seed}: pts");
        assert_eq!(got.duration, want.duration, "seed {seed}: duration");
        assert_eq!(got.sync, source.sync, "seed {seed}: sync");
    }
    Ok(())
}

#[test]
fn rounding_keeps_its_promises_on_random_tracks() {
    let mut planned = 0;
    let mut collapsed = 0;
    for seed in 0..2000u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (source, packets) = source(&mut random);
        let scale = random.pick(&[1u64, 999, 1000, 333_333, 1_000_000, 10_000_000]);
        match StreamCopy::plan_rounded(&source, &packets, scale) {
            Ok(plan) => {
                check_plan(seed, &source, &packets, &plan, scale);
                check_output(seed, &packets, &plan, scale)
                    .unwrap_or_else(|err| panic!("seed {seed}: {err}"));
                planned += 1;
            }
            Err(err @ Error::Unsupported { .. }) => {
                let text = err.to_string();
                if text.contains("one tick") {
                    assert!(text.contains("scale of"), "seed {seed}: {text}");
                    collapsed += 1;
                } else {
                    assert!(
                        text.contains("never negative")
                            || text.contains("one source `pts`")
                            || text.contains("ends before"),
                        "seed {seed}: {text}"
                    );
                }
            }
            Err(err) => panic!("seed {seed}: {err}"),
        }
    }
    assert!(planned > 500, "only {planned} tracks planned");
    assert!(collapsed > 10, "only {collapsed} tracks collapsed");
}
