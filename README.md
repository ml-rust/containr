<div align="center">

# containr

<h3>Media containers for Rust. Nothing is decoded.</h3>

<p>
containr reads MP4/MOV and Matroska/WebM containers: boxes and EBML elements, sample tables and blocks, packets with integer timestamps, track metadata, chapters, attachments, and raw edit lists. It writes MP4/MOV with stream copy. Matroska/WebM muxing is a roadmap item. It never decodes or encodes media. <a href="https://github.com/ml-rust/vidr">vidr</a> (video) and <a href="https://github.com/ml-rust/voxr">voxr</a> (audio) build on it.
</p>

<p>
  <a href="#status"><strong>Status</strong></a>
  ·
  <a href="#features"><strong>Features</strong></a>
  ·
  <a href="#usage"><strong>Usage</strong></a>
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

## Status

In development. No release exists yet.

| Capability                                                                                                                       | Status      |
| -------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| MP4/MOV demux (`mp4`): tracks, sample tables, packets, raw edit lists, `udta`/`meta` metadata, `chpl` and chapter-track chapters | Implemented |
| Fragmented MP4 (`moof`) demux                                                                                                    | Roadmap     |
| MP4/MOV mux (`mp4`): faststart, interleaved chunks, `co64`, edit lists, metadata, chapters, stream copy from MP4 and Matroska     | Implemented |
| MKV/WebM demux (`mkv`): tracks, Clusters of known and unknown size, laced blocks, `Cues`, `Chapters`, `Tags`, `Attachments`      | Implemented |
| MKV/WebM mux                                                                                                                     | Roadmap     |
| Stream copy into MP4/MOV (`mp4`), from MP4 and from Matroska H.264, HEVC, AV1, AAC, Opus and `S_TEXT/UTF8` tracks                | Implemented |
| Stream copy into MKV/WebM                                                                                                        | Roadmap     |

## Features

Each container is a Cargo feature. The default build holds the shared types only.

| Feature | What it adds                                                        |
| ------- | ------------------------------------------------------------------- |
| `mp4`   | `containr::mp4::Demuxer` and `Muxer` for MP4 and QuickTime MOV      |
| `mkv`   | `containr::mkv::Demuxer` for Matroska and WebM                      |

## MP4/MOV demux

- `moov` can sit before or after `mdat`.
- Each track carries its id, kind and handler type, timescale, duration, and language.
- Each sample entry keeps its raw bytes for stream copy, plus `avcC`, `hvcC`, `av1C`, `vpcC`, `esds` or `dOps`.
- Video tracks carry width, height, the `pasp` ratio, and the raw `tkhd` matrix. Audio tracks carry channels and sample rate.
- Packets carry `dts`, `pts` (signed `ctts` included), duration, sync flag, byte offset, and size.
- Sample tables: `stts`, `ctts`, `stsc`, `stsz`, `stz2`, `stco`, `co64`, `stss`. They expand one sample at a time, so memory tracks the table entries, not the sample count.
- Edit lists come raw, with the movie timescale. The caller applies them.
- Metadata: `udta` children and `meta/ilst` items, raw. Chapters come from `chpl` and from QuickTime chapter tracks.
- Fragmented MP4 is refused with `Error::Fragmented`.
- Malformed input returns an error that names the box path. Every size from the file is checked before allocation.

## MP4/MOV mux

- `Muxer::new`, `add_track`, `set_metadata`, `set_chapters`, `write_packet`, `finish`. The target implements `Write` and `Seek`, and `Read` for `finish`, since faststart reads `mdat` back.
- `TrackSpec` carries the sample entry as raw stream-copy bytes, or a build from `avcC`, `hvcC`, `av1C`, `esds`, or `dOps`, plus timescale, language, matrix, `pasp`, and an edit list.
- `TrackSpec::copy_from` builds a spec from an MP4 track, or from a Matroska H.264, HEVC, AV1, AAC, Opus or `S_TEXT/UTF8` track. `StreamCopy::plan` adds packet timing and, for Matroska, the decode times and edit list MP4 needs.
- Layout: `ftyp`, `mdat`, `moov`. `MuxOptions::faststart` moves `moov` to the front in place, in a second pass on the same target, and adds the shift to every chunk offset.
- Chunks last at most `MuxOptions::chunk_duration_ms` and interleave across tracks by decode time. `MuxOptions::max_buffer` bounds the bytes held back.
- Sample tables: `stts`, `ctts` (version 1 with `cslg` for a negative offset), `stsc`, `stsz`, `stss`, and `stco`, or `co64` past 4 GiB. The last sample's duration comes from its packet, so the track's total duration is exact.
- Refused, each by name: a codec MP4 cannot hold (the error names Matroska), `dts` out of order, a composition offset `ctts` cannot store, a second sample description, a last packet with no duration and no default.
- Composition offsets range from -2^31 to 2^31 - 1, since readers treat `ctts` offsets as signed.
- Known limits: stream copy drops MP4 `sgpd` and `sbgp` sample-group boxes. Matroska attachments have no MP4 form, so the muxer never sees them. A caller that copies a Matroska file to MP4 must check `Demuxer::attachments()` and refuse the copy, naming MKV as the container that holds attachments.

## MKV/WebM demux

- The EBML reader is containr's own. It checks every size before it allocates, and decodes unknown sizes of any length.
- Each track carries its `TrackNumber` as id, kind, `TimestampScale` timebase, and language. `Track::matroska` holds `TrackUID`, `CodecID`, raw `CodecPrivate`, `Language` and `LanguageBCP47`, `DefaultDuration`, `CodecDelay`, `SeekPreRoll`, flags, and `ContentEncodings`.
- Video tracks carry pixel and display size, `DisplayUnit`, crop, and `Projection` with `ProjectionPoseRoll`. Audio tracks carry sampling frequency, channels, and bit depth.
- Packets carry `pts` (Cluster `Timestamp` plus the block timestamp), duration, keyframe and discardable flags, `DiscardPadding`, byte offset, and size. `dts` is `None`: Matroska stores no decode timestamp.
- Xiph, EBML and fixed lacing expand into one packet per frame. Each laced packet records its place in the lace.
- Timestamps count in `TimestampScale` nanoseconds, as integers. `CodecDelay` and `DiscardPadding` come raw. The caller applies them.
- A Cluster of unknown size, as a live WebM writes it, ends at the next Segment child.
- `Cues`, `SeekHead` entries, chapters from every edition, and `Tags` come raw. A tag that targets a track's `TrackUID` sits in that track's metadata.
- Attachments expose name, media type, UID, and the byte range of their data. `Demuxer::read_attachment` reads it.
- A truncated file keeps every block before the cut. `Demuxer::truncated` reports the cut.
- Malformed input returns an error that names the element path, such as `Segment/Cluster[3]/SimpleBlock[7]`.

## Usage

```rust
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

```rust
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

## Minimum Rust version

1.95.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for conventions and checks.

Questions and design discussion happen on [Discord](https://discord.gg/jBhFk9kHPg).

## License

Licensed under Apache-2.0. See [LICENSE](LICENSE).
