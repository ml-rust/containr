# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each release must have a non-empty section here before it can be tagged — `.github/workflows/release-validate.yml` refuses a tag whose version has no entry, and the GitHub Release body is this file's section for that version.

## [Unreleased]

### Added

- MP4 and QuickTime MOV demux behind the `mp4` feature: tracks, sample entries with codec configuration, packets with `dts`, `pts`, duration and sync flag, raw edit lists, `udta`/`meta` metadata, and `chpl` and chapter-track chapters. `moov` can sit before or after `mdat`.
- Fragmented MP4 is refused with `Error::Fragmented`.
- Matroska and WebM demux behind the `mkv` feature, with containr's own EBML reader: tracks with `CodecID`, `CodecPrivate`, language, `DefaultDuration`, `CodecDelay`, `SeekPreRoll`, video and audio facts, and `Projection`. Packets come from `SimpleBlock` and `BlockGroup` blocks, with Xiph, EBML and fixed lacing expanded, `DiscardPadding`, and keyframe and discardable flags. Clusters can have an unknown size. `Cues`, `SeekHead`, `Chapters`, `Tags` and `Attachments` are read.
- Shared types cover both formats. `Track::timebase` is an integer `Timebase` ratio, so a Matroska `TimestampScale` needs no float. `Packet::dts`, `Packet::duration`, `Track::handler` and `Track::duration` are `Option`s, since Matroska stores no dts, handler or track duration. `Packet` carries `discardable`, `discard_padding` and `lace`, `Track` carries `matroska`, and `MetadataScope::Tag` and `ChapterSource::Matroska` carry Matroska tags and chapters.
- `Error::Malformed`, `Inconsistent`, `TooLarge` and `MissingBox` name their `Container`.
- MP4 and QuickTime MOV mux behind the `mp4` feature: `containr::mp4::Muxer` writes `ftyp`, `mdat` and `moov`, with `MuxOptions::faststart` moving `moov` to the front in place. Chunks of bounded duration interleave across tracks by decode time, with a bounded buffer. It writes `stts`, `ctts` (signed with `cslg` when needed), `stsc`, `stsz`, `stco` or `co64`, `stss`, edit lists, `udta`/`meta`/`keys` metadata, and `chpl` plus a QuickTime chapter track.
- Stream copy into MP4 and MOV: `TrackSpec::copy_from` and `StreamCopy::plan`, from an MP4 track, or from a Matroska H.264, HEVC, AV1, AAC, Opus or `S_TEXT/UTF8` track. Every other Matroska codec is refused with an error that names Matroska. Two known limits: stream copy drops MP4 `sgpd` and `sbgp` sample-group boxes, and Matroska attachments have no MP4 form, so a caller copying a Matroska file to MP4 must check `Demuxer::attachments()` and refuse the copy, naming MKV as the container that holds attachments.
- Matroska and WebM mux behind the `mkv` feature, with containr's own EBML writer: `containr::mkv::Muxer` writes the EBML header, Segment, `SeekHead`, `Info`, `Tracks`, Clusters, `Cues`, `Chapters`, `Tags` and `Attachments`. Blocks are `SimpleBlock`, or `BlockGroup` where a duration differs from what `DefaultDuration` gives or a `DiscardPadding` applies. The last block of each track always carries `BlockDuration`. `MkvOptions::seekable` set to false needs no seek, with `NoSeek` for a target that has none. `DocType::WebM` refuses every codec but VP8, VP9, AV1, Opus, Vorbis and WebVTT, and refuses attachments, with an error that names Matroska.
- Stream copy into Matroska and WebM: `mkv::TrackSpec::copy_from` and `mkv::StreamCopy::plan`, from a Matroska track with every `MatroskaTrack` fact, or from an MP4 `avc1`, `avc3`, `hvc1`, `hev1`, `av01`, `vp09`, `mp4a`, `Opus` or `tx3g` track. `mkv::tags_from_mp4` maps MP4 text metadata to tags.
- The Matroska timestamp rule: a value converts when it is a whole number of nanoseconds and a whole multiple of `TimestampScale`. Nothing rounds. `StreamCopy::timestamp_scale` picks a scale that holds a set of tracks. A value that no scale holds, such as an AAC frame at 44.1 or 48 kHz in an MP4 timescale, is refused with an error that names MP4.
- An MP4 edit list maps to Matroska timing in `StreamCopy::plan`: an empty edit and a video `media_time` shift `pts`, an audio `media_time` becomes `CodecDelay`, and an audio end trim becomes `DiscardPadding`. An edit list with no Matroska form, and a frame an edit cuts from the start, are refused with an error that names MP4.
- `matroska::RawElement`, and `extra` on `MatroskaTrack`, `MatroskaVideo` and `MatroskaAudio`: the children containr does not model, such as `Colour`, kept raw for stream copy. `ContentEncoding::encryption_settings` keeps the raw `ContentEncryption` body.
- `Container::WebM` for WebM mux refusals.
- `Error::Unsupported`, `DtsOutOfOrder`, `CompositionOffset`, `SampleDescriptionChange`, `MissingDuration`, `InvalidPacket` and `InvalidMux` for mux refusals.

### Changed

- The Matroska demuxer reads the discardable bit on a `Block` in a `BlockGroup`, where containr's muxer keeps the flag. Other muxers leave that bit at 0.
