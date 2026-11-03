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
