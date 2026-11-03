<div align="center">

# containr

<h3>Media containers for Rust. Nothing is decoded.</h3>

<p>
containr reads MP4/MOV containers: boxes, sample tables, packets with integer timestamps, track metadata, and raw edit lists. MP4/MOV mux and MKV/WebM are roadmap items. It never decodes or encodes media. <a href="https://github.com/ml-rust/vidr">vidr</a> (video) and <a href="https://github.com/ml-rust/voxr">voxr</a> (audio) build on it.
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
| MP4/MOV mux                                                                                                                      | Roadmap     |
| MKV/WebM demux and mux (`mkv`)                                                                                                   | Roadmap     |
| Stream copy                                                                                                                      | Roadmap     |

## Features

Each container is a Cargo feature. The default build holds the shared types only.

| Feature | What it adds                                                        |
| ------- | ------------------------------------------------------------------- |
| `mp4`   | `containr::mp4::Demuxer` for MP4 and QuickTime MOV                  |
| `mkv`   | Matroska and WebM. Roadmap: the feature compiles nothing yet        |

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

## Minimum Rust version

1.95.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for conventions and checks.

Questions and design discussion happen on [Discord](https://discord.gg/jBhFk9kHPg).

## License

Licensed under Apache-2.0. See [LICENSE](LICENSE).
