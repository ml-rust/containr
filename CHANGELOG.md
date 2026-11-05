# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-05

First release.

### Added

- `mp4` feature: read and write MP4 and QuickTime MOV, with sample tables, edit lists, metadata, chapters, and faststart. Fragmented MP4 demux returns `Error::Fragmented`.
- `mkv` feature: read and write Matroska and WebM, with lacing, `DiscardPadding`, `Cues`, chapters, tags, attachments, and Clusters of unknown size. WebM refuses codecs and attachments it cannot hold.
- `StreamCopy`: copy tracks within and between both families, with edit lists, codec delay, colour tags, languages, and titles. Stream copy drops MP4 `sgpd` and `sbgp` boxes. MP4 has no form for Matroska attachments, so a caller copying to MP4 must check `Demuxer::attachments()`. A last Matroska audio or video packet with no stored duration, as in FFmpeg 8 audio, takes the gap between the last two timestamps, less its `DiscardPadding`. A track with one timestamp or a subtitle track with no duration stays refused, so set `PacketIn::duration` for it.
- `mkv::StreamCopy::plan`: copy timestamps exactly. It refuses a value no `TimestampScale` holds.
- `mkv::StreamCopy::plan_rounded`: opt in to rounding. Each timestamp stays within half a tick.
- `ContainerColour`: read and write `colr` and `Colour` tags. The type is in the default build. Reading and writing need `mp4` or `mkv`.
- `mp4::first_track_timing`, `mkv::first_track_timing`: read trims with no sample tables or packet index. The MP4 function accepts a fragmented file.
- `buffered_bytes`: read the bytes a muxer holds back on `mp4::Muxer` and `mkv::Muxer`. `max_buffer` in each muxer's options bounds them.
- Mux errors: each error names the container that can hold the refused stream.
