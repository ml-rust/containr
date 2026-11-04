<div align="center">

# containr

<h3>Media containers in Rust. Packets in, packets out, nothing decoded.</h3>

<p>
Read and write MP4, MOV, Matroska, and WebM, with stream copy between them. Timestamps stay integers and edit lists stay raw.
</p>

<p>
  <a href="#status"><strong>Status</strong></a>
  ·
  <a href="#features"><strong>Features</strong></a>
  ·
  <a href="#usage"><strong>Usage</strong></a>
  ·
  <a href="#format-details"><strong>Format details</strong></a>
  ·
  <a href="CONTRIBUTING.md"><strong>Contributing</strong></a>
</p>

<p>
  <a href="https://discord.gg/jBhFk9kHPg">
    <img src="https://img.shields.io/discord/1453357769720594543?label=Discord&logo=discord&logoColor=white&color=5865F2" alt="Join the Discord">
  </a>
</p>

<p>
  <a href="https://github.com/ml-rust/containr/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/ml-rust/containr/ci.yml?branch=main&label=ci" alt="CI status">
  </a>
  <a href="LICENSE">
    <img src="https://img.shields.io/github/license/ml-rust/containr" alt="License">
  </a>
  <a href="https://github.com/ml-rust/containr/stargazers">
    <img src="https://img.shields.io/github/stars/ml-rust/containr?style=social" alt="GitHub stars">
  </a>
</p>

</div>

containr holds the container layer of the ml-rust media stack. It parses boxes and EBML elements into tracks, packets, metadata, chapters, and attachments, and it writes them back. [vidr](https://github.com/ml-rust/vidr) (video) and [voxr](https://github.com/ml-rust/voxr) (audio) depend on it. Codecs live in those crates, so containr never decodes or encodes media.

```text
file → demux → packets with integer timestamps → your codec, or mux → file
```

## Status

In development. No release exists yet.

| Format | Demux | Mux | Feature |
| ------ | ----- | --- | ------- |
| MP4, QuickTime MOV | Yes. Fragmented MP4 (`moof`) is planned, and refused today | Yes | `mp4` |
| Matroska, WebM | Yes | Yes. WebM refuses codecs it cannot hold | `mkv` |

### Stream copy

| Source track | Into MP4/MOV | Into MKV/WebM |
| ------------ | ------------ | ------------- |
| From MP4/MOV | Any track, by raw sample entry | `avc1`, `avc3`, `hvc1`, `hev1`, `av01`, `vp09`, `mp4a`, `Opus`, `tx3g` |
| From MKV/WebM | H.264, HEVC, AV1, AAC, Opus, `S_TEXT/UTF8` | Any track |

`mkv::StreamCopy::plan` refuses a timestamp Matroska cannot hold exactly. See [what `TimestampScale` holds](#limits-what-timestampscale-holds).

## Features

Each container is a Cargo feature. The default build holds the shared types only.

| Feature | What it adds                                                        |
| ------- | ------------------------------------------------------------------- |
| `mp4`   | `containr::mp4::Demuxer` and `Muxer` for MP4 and QuickTime MOV      |
| `mkv`   | `containr::mkv::Demuxer` and `Muxer` for Matroska and WebM          |

## Usage

The examples need the `mp4` and `mkv` features.

Read an MP4 or MOV file:

```rust,no_run
use containr::mp4::Demuxer;

fn main() -> Result<(), containr::Error> {
    let mut demuxer = Demuxer::open_path("input.mp4")?;
    for track in demuxer.tracks() {
        println!("track {}: {:?} {:?}", track.id, track.kind, track.codec());
    }
    for packet in demuxer.packets() {
        let packet = packet?;
        let bytes = demuxer.read_packet(&packet)?;
        println!("track {} pts {} size {}", packet.track_id, packet.pts, bytes.len());
    }
    Ok(())
}
```

`containr::mkv::Demuxer` has the same shape for Matroska and WebM:

```rust,no_run
use containr::mkv::Demuxer;

fn main() -> Result<(), containr::Error> {
    let mut demuxer = Demuxer::open_path("input.webm")?;
    for track in demuxer.tracks() {
        println!("track {}: {:?} {:?}", track.id, track.kind, track.codec_id());
    }
    for packet in demuxer.packets() {
        let packet = packet?;
        let bytes = demuxer.read_packet(&packet)?;
        println!("track {} pts {} size {}", packet.track_id, packet.pts, bytes.len());
    }
    Ok(())
}
```

Copy an MP4 file into a new one, with `moov` at the front. The muxer rebuilds the chapter track from `set_chapters`, so the copy skips it:

```rust,no_run
use std::collections::HashMap;
use std::fs::File;

use containr::mp4::{Demuxer, MuxOptions, Muxer, StreamCopy};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Demuxer::open_path("input.mp4")?;
    let target = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("output.mp4")?;
    let options = MuxOptions {
        faststart: true,
        ..MuxOptions::default()
    };
    let movie_timescale = options.movie_timescale;
    let mut output = Muxer::new(target, options)?;

    let mut plans = HashMap::new();
    for track in input.tracks().iter().filter(|track| !track.chapter_track) {
        let packets = input.track_packets(track.id)?.collect::<Result<Vec<_>, _>>()?;
        let plan = StreamCopy::plan(track, &packets, movie_timescale)?;
        let id = output.add_track(plan.spec.clone())?;
        plans.insert(track.id, (plan, id));
    }
    output.set_metadata(input.metadata().to_vec())?;
    output.set_chapters(StreamCopy::chapters(input.chapters()))?;
    for packet in input.packets() {
        let packet = packet?;
        let Some((plan, id)) = plans.get(&packet.track_id) else {
            continue;
        };
        let data = input.read_packet(&packet)?;
        output.write_packet(*id, &plan.packet(&packet, &data)?)?;
    }
    output.finish()?;
    Ok(())
}
```

Copy a Matroska or WebM file into a new one, with one plan per track:

```rust,no_run
use std::fs::File;

use containr::mkv::{Demuxer, MkvOptions, Muxer, StreamCopy};

fn main() -> Result<(), containr::Error> {
    let mut input = Demuxer::open_path("input.mkv")?;
    let mut plans = Vec::new();
    for track in input.tracks() {
        let packets = input
            .track_packets(track.id)?
            .collect::<Result<Vec<_>, _>>()?;
        plans.push((track.id, StreamCopy::plan(track, &packets)?));
    }
    let options = MkvOptions {
        timestamp_scale: input.info().timestamp_scale,
        title: input.info().title.clone(),
        ..MkvOptions::default()
    };
    let output = File::create("output.mkv")
        .map_err(|err| containr::Error::Io {
            what: "output.mkv".to_owned(),
            offset: 0,
            source: err,
        })?;
    let mut muxer = Muxer::new(output, options)?;
    let mut ids = Vec::new();
    for (source, plan) in &plans {
        ids.push((*source, muxer.add_track(plan.spec.clone())?));
    }
    muxer.set_metadata(input.metadata().to_vec())?;
    muxer.set_chapters(input.chapters().to_vec())?;
    for packet in input.packets() {
        let packet = packet?;
        let bytes = input.read_packet(&packet)?;
        let slot = ids.iter().position(|(source, _)| *source == packet.track_id);
        if let Some(slot) = slot
            && let Some(block) = plans[slot].1.packet(&packet, &bytes)?
        {
            muxer.write_packet(ids[slot].1, &block)?;
        }
    }
    muxer.finish()?;
    Ok(())
}
```

Copy an MP4 file with 29.97 fps video or 48 kHz AAC into Matroska, rounded to 1 ms ticks:

```rust,no_run
use std::fs::File;

use containr::mkv::{MkvOptions, Muxer, StreamCopy};
use containr::mp4::Demuxer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Demuxer::open_path("input.mp4")?;
    let mut plans = Vec::new();
    for track in input.tracks().iter().filter(|track| !track.chapter_track) {
        let packets = input.track_packets(track.id)?.collect::<Result<Vec<_>, _>>()?;
        let plan = StreamCopy::plan_rounded(track, &packets, 1_000_000)?;
        if let Some(rounding) = plan.rounding() {
            println!("track {}: at most {} ns off", track.id, rounding.max_error_ns);
        }
        plans.push((track.id, plan));
    }
    let scales: Vec<StreamCopy> = plans.iter().map(|(_, plan)| plan.clone()).collect();
    let options = MkvOptions {
        timestamp_scale: StreamCopy::timestamp_scale(&scales),
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(File::create("output.mkv")?, options)?;
    let mut ids = Vec::new();
    for (source, plan) in &plans {
        ids.push((*source, muxer.add_track(plan.spec.clone())?));
    }
    for packet in input.packets() {
        let packet = packet?;
        let slot = ids.iter().position(|(source, _)| *source == packet.track_id);
        let Some(slot) = slot else {
            continue;
        };
        let bytes = input.read_packet(&packet)?;
        if let Some(block) = plans[slot].1.packet(&packet, &bytes)? {
            muxer.write_packet(ids[slot].1, &block)?;
        }
    }
    muxer.finish()?;
    Ok(())
}
```

## Format details

### MP4/MOV demux

- `moov` can sit before or after `mdat`.
- Each track carries its id, kind and handler type, timescale, duration, and language.
- Each sample entry keeps its raw bytes for stream copy, plus `avcC`, `hvcC`, `av1C`, `vpcC`, `esds` or `dOps`.
- Video tracks carry width, height, the `pasp` ratio, and the raw `tkhd` matrix. Audio tracks carry channels and sample rate.
- `VideoInfo::colour` holds the H.273 tags of a `colr` box. `Track::colour` gives the value for either container.
  - `nclx`: tags and the full-range bit.
  - `nclc` (QuickTime): tags with no range.
  - `prof`, `rICC`: no H.273 values. `colour` stays `None` unless another `colr` box gives `nclx`.
  - A code point above 255 is an error.
- Packets carry `dts`, `pts` (signed `ctts` included), duration, sync flag, byte offset, and size.
- Sample tables: `stts`, `ctts`, `stsc`, `stsz`, `stz2`, `stco`, `co64`, `stss`. They expand one sample at a time, so memory tracks the table entries, not the sample count.
- Edit lists come raw, with the movie timescale. The caller applies them.
- Metadata: `udta` children and `meta/ilst` items, raw. Chapters come from `chpl` and from QuickTime chapter tracks.
- Fragmented MP4 is refused with `Error::Fragmented`.
- `mp4::first_track_timing` reads the `mdhd` timescale and raw edit list of the first track of a kind, from `moov` alone, with no sample tables. It accepts a fragmented file.
- Malformed input returns an error that names the box path. Every size from the file is checked before allocation.

### MP4/MOV mux

- `Muxer::new`, `add_track`, `set_metadata`, `set_chapters`, `write_packet`, `finish`. The target implements `Write` and `Seek`, and `Read` for `finish`, since faststart reads `mdat` back.
- `TrackSpec` carries the sample entry as raw stream-copy bytes, or builds it from `avcC`, `hvcC`, `av1C`, `esds`, or `dOps`.
- `TrackSpec` also carries timescale, language, matrix, `pasp`, `colour`, and an edit list. `colour` writes an `nclx` `colr` box into a built visual entry. A raw entry keeps its own `colr` box.
- `TrackSpec::copy_from` builds a spec from an MP4 track.
- `TrackSpec::copy_from` also takes a Matroska H.264, HEVC, AV1, AAC, Opus or `S_TEXT/UTF8` track. `StreamCopy::plan` adds packet timing and, for Matroska, the decode times and edit list MP4 needs.
- Matroska timing into MP4:

  | Matroska fact | MP4 result |
  | ------------- | ---------- |
  | Negative timestamps | The edit list drops them |
  | Padding of the last block | The edit list drops it |
  | AAC `CodecDelay` | Becomes the edit's `media_time`. The segment lasts to the end of the last block less that delay |
  | A delay the Matroska tick cannot hold | The track timescale rises to a multiple that holds it, so no timestamp rounds |
  | A delay no tick count matches within 1 ns | Refused. The error names Matroska |
  | Opus `CodecDelay` | Adds no edit. `dOps` `pre_skip` already carries it, and a second skip drops it twice |

- Layout: `ftyp`, `mdat`, `moov`. `MuxOptions::faststart` moves `moov` to the front in a second pass on the same target. That pass adds the shift to every chunk offset.
- Chunks last at most `MuxOptions::chunk_duration_ms` and interleave across tracks by decode time. `MuxOptions::max_buffer` bounds the bytes held back.
- Sample tables: `stts`, `ctts`, `stsc`, `stsz`, `stss`, and `stco`.
- `co64` replaces `stco` past 4 GiB. Version 1 `ctts` with `cslg` holds a negative offset. The last sample's duration comes from its packet, so the track's total duration is exact.
- Refused, each by name:
  - a codec MP4 cannot hold (the error names Matroska)
  - `dts` out of order
  - a composition offset `ctts` cannot store
  - a second sample description
  - a last packet with no duration and no default
- Composition offsets range from -2^31 to 2^31 - 1, since readers treat `ctts` offsets as signed.
- Known limits:
  - Stream copy drops MP4 `sgpd` and `sbgp` sample-group boxes.
  - Matroska attachments have no MP4 form, so the muxer never sees them.
  - A caller that copies Matroska to MP4 must check `Demuxer::attachments()`. When it lists any, refuse the copy and name MKV as the container that holds them.

### MKV/WebM demux

- The EBML reader is containr's own. It checks every size before it allocates, and decodes unknown sizes of any length.
- Each track carries its `TrackNumber` as id, kind, `TimestampScale` timebase, and language. `Track::matroska` holds `TrackUID`, `CodecID`, raw `CodecPrivate`, `Language` and `LanguageBCP47`, `DefaultDuration`, `CodecDelay`, `SeekPreRoll`, flags, and `ContentEncodings`.
- Video tracks carry pixel and display size, `DisplayUnit`, crop, and `Projection` with `ProjectionPoseRoll`. Audio tracks carry sampling frequency, channels, and bit depth.
- `MatroskaVideo::colour` holds `MatrixCoefficients`, `TransferCharacteristics`, `Primaries`, and `Range`. The raw `Colour` element also stays in `MatroskaVideo::extra`.
  - `Range` 1 gives limited, 2 gives full, and 0 or 3 give `None`.
  - A code point above 255 is an error that names the element path.
- Packets carry `pts` (Cluster `Timestamp` plus the block timestamp), duration, keyframe and discardable flags, `DiscardPadding`, byte offset, and size. `dts` is `None`: Matroska stores no decode timestamp.
- Xiph, EBML and fixed lacing expand into one packet per frame. Each laced packet records its place in the lace.
- Timestamps count in `TimestampScale` nanoseconds, as integers. `CodecDelay` and `DiscardPadding` come raw. The caller applies them.
- A QuickTime Macintosh language code stays `Language::Other`.
  - `Language::iso639` gives `None` for it.
  - `Language::iso639_mapped` maps it to ISO 639-2/T.
  - Codes with no table entry give `None`: `0x7FFF`, 95 to 127, and above 138.
- A Cluster of unknown size, as a live WebM writes it, ends at the next Segment child.
- `Cues`, `SeekHead` entries, chapters from every edition, and `Tags` come raw. A tag that targets a track's `TrackUID` sits in that track's metadata.
- Attachments expose name, media type, UID, and the byte range of their data. `Demuxer::read_attachment` reads it.
- A truncated file keeps every block before the cut. `Demuxer::truncated` reports the cut.
- `mkv::first_track_timing` reads the `CodecDelay` and summed positive `DiscardPadding` of the first track of a kind, with no packet index.
- Malformed input returns an error that names the element path, such as `Segment/Cluster[3]/SimpleBlock[7]`.

### MKV/WebM mux

- `Muxer::new`, `add_track`, `set_metadata`, `set_chapters`, `attach`, `write_packet`, `finish`. A target with no seek works through `NoSeek` with `MkvOptions::seekable` set to false.
- `TrackSpec::copy_from` and `StreamCopy::plan` copy a Matroska track with every `MatroskaTrack` fact, or map an MP4 `avc1`, `avc3`, `hvc1`, `hev1`, `av01`, `vp09`, `mp4a`, `Opus` or `tx3g` track.
- Colour tags survive every copy:

  | Copy | Result |
  | ---- | ------ |
  | MP4 to MKV | A `colr` `nclx` or `nclc` box maps to `Colour` |
  | MKV to MP4 | `Colour` maps to an `nclx` `colr` box. An unset tag writes 2 (unspecified). An unset range writes limited |
  | Hand-built spec | Sets `MatroskaVideo::colour` |
  | Raw `Colour` element that matches `colour` | The muxer writes it unchanged |
  | Raw `Colour` element that differs | `colour` replaces the four tags. Other children, such as `MasteringMetadata`, stay |

- Languages:

  | Output | Form | Example |
  | ------ | ---- | ------- |
  | Matroska | Bibliographic, because version 2 readers expect it | `ger` |
  | MP4 | Terminology, as `mdhd` defines it | `deu` |

  An MP4 track's language maps through `Language::iso639_mapped`. A code with no table entry writes `und`.

- An edit list maps to timing: an empty edit and a video `media_time` shift `pts`, an audio `media_time` becomes `CodecDelay`, and an audio end trim becomes `DiscardPadding`.
- The default refuses. `StreamCopy::plan` and `Muxer::write_packet` take a value only when it is a whole number of nanoseconds and a whole multiple of `TimestampScale`. `StreamCopy::timestamp_scale` picks a scale that holds a set of tracks.
- A repeated `TrackUID`, attachment UID or non-zero `ChapterUID` is refused.

#### Limits: what `TimestampScale` holds

Matroska stores integer ticks of `TimestampScale`. It cannot hold 1024/48000 s or 1001/30000 s exactly, at any scale. So `StreamCopy::plan` refuses AAC at 44.1 or 48 kHz and 29.97 fps video in a 30000 timescale. The error names MP4, which holds them, and `StreamCopy::plan_rounded`.

`StreamCopy::plan_rounded(track, packets, scale)` is the opt-in for MP4 tracks. `scale` is the `TimestampScale` in nanoseconds:

- Each timestamp rounds on its own, from its exact source value, to the nearest tick, ties to even, in integer arithmetic. Its error stays at most half a tick, and no error accumulates across packets.
- A block's duration is the distance to the next packet's rounded timestamp, in presentation order. The last packet in that order rounds from its exact end. So durations sum to the track's rounded end.
- `CodecDelay`, `DiscardPadding` and `DefaultDuration` count in nanoseconds, so each rounds to the nearest nanosecond, ties to even. A block's duration can differ from `DefaultDuration` by a tick, and then the block states its own.
- Decode order and `sync` flags stay as the source has them.
- Two packets of one track on one tick are refused, and the error names a finer scale.
- `StreamCopy::rounding` reports the scale and the largest absolute error, in nanoseconds, rounded up.
- `StreamCopy::timestamp_scale` gives the largest value that divides every plan's scale, so rounded and exact plans share one output.

A chapter time stays exact or refused. `plan_rounded` does not round chapters.

## Minimum Rust version

1.95.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for conventions and checks.

Questions and design discussion happen on [Discord](https://discord.gg/jBhFk9kHPg).

## License

Licensed under Apache-2.0. See [LICENSE](LICENSE).
