# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each release must have a non-empty section here before it can be tagged — `.github/workflows/release-validate.yml` refuses a tag whose version has no entry, and the GitHub Release body is this file's section for that version.

## [Unreleased]

### Added

- `ContainerColour`, `VideoInfo::colour`, `MatroskaVideo::colour` and `Track::colour`: the H.273 colour primaries, transfer characteristics, matrix coefficients and full-range flag that a container tags next to the bitstream. MP4 and MOV read `colr` `nclx` and QuickTime `nclc`. Matroska reads `Video/Colour`. A code point above 255 gives an error that names the box or element path. `prof` and `rICC` boxes give no values.
- Stream copy keeps colour tags. MP4 to MP4 and MKV to MKV keep the raw bytes. MP4 to MKV maps `colr` to `Colour`, and MKV to MP4 maps `Colour` to an `nclx` `colr` box. `mp4::TrackSpec::colour` and `MatroskaVideo::colour` set the tags on a hand-built spec.
- MP4 and QuickTime MOV demux behind the `mp4` feature: tracks, sample entries with codec configuration, packets with `dts`, `pts`, duration and sync flag, raw edit lists, `udta`/`meta` metadata, and `chpl` and chapter-track chapters. `moov` can sit before or after `mdat`.
- Fragmented MP4 is refused with `Error::Fragmented`.
- Matroska and WebM demux behind the `mkv` feature, with containr's own EBML reader: tracks with `CodecID`, `CodecPrivate`, language, `DefaultDuration`, `CodecDelay`, `SeekPreRoll`, video and audio facts, and `Projection`. Packets come from `SimpleBlock` and `BlockGroup` blocks, with Xiph, EBML and fixed lacing expanded, `DiscardPadding`, and keyframe and discardable flags. Clusters can have an unknown size. `Cues`, `SeekHead`, `Chapters`, `Tags` and `Attachments` are read.
- Shared types cover both formats. `Track::timebase` is an integer `Timebase` ratio, so a Matroska `TimestampScale` needs no float. `Packet::dts`, `Packet::duration`, `Track::handler` and `Track::duration` are `Option`s, since Matroska stores no dts, handler or track duration. `Packet` carries `discardable`, `discard_padding` and `lace`, `Track` carries `matroska`, and `MetadataScope::Tag` and `ChapterSource::Matroska` carry Matroska tags and chapters.
- `Error::Malformed`, `Inconsistent`, `TooLarge` and `MissingBox` name their `Container`.
- MP4 and QuickTime MOV mux behind the `mp4` feature: `containr::mp4::Muxer` writes `ftyp`, `mdat` and `moov`, with `MuxOptions::faststart` moving `moov` to the front in place. Chunks of bounded duration interleave across tracks by decode time, with a bounded buffer. It writes `stts`, `ctts` (signed with `cslg` when needed), `stsc`, `stsz`, `stco` or `co64`, `stss`, edit lists, `udta`/`meta`/`keys` metadata, and `chpl` plus a QuickTime chapter track.
- Stream copy into MP4 and MOV: `TrackSpec::copy_from` and `StreamCopy::plan`, from an MP4 track, or from a Matroska H.264, HEVC, AV1, AAC, Opus or `S_TEXT/UTF8` track. Every other Matroska codec is refused with an error that names Matroska. Two known limits: stream copy drops MP4 `sgpd` and `sbgp` sample-group boxes, and Matroska attachments have no MP4 form, so a caller copying a Matroska file to MP4 must check `Demuxer::attachments()` and refuse the copy, naming MKV as the container that holds attachments.
- Matroska and WebM mux behind the `mkv` feature, with containr's own EBML writer: `containr::mkv::Muxer` writes the EBML header, Segment, `SeekHead`, `Info`, `Tracks`, Clusters, `Cues`, `Chapters`, `Tags` and `Attachments`. Blocks are `SimpleBlock`, or `BlockGroup` where a duration differs from what `DefaultDuration` gives or a `DiscardPadding` applies. The last block of each track always carries `BlockDuration`. `MkvOptions::seekable` set to false needs no seek, with `NoSeek` for a target that has none. `DocType::WebM` refuses every codec but VP8, VP9, AV1, Opus, Vorbis and WebVTT, and refuses attachments, with an error that names Matroska.
- Stream copy into Matroska and WebM: `mkv::TrackSpec::copy_from` and `mkv::StreamCopy::plan`, from a Matroska track with every `MatroskaTrack` fact, or from an MP4 `avc1`, `avc3`, `hvc1`, `hev1`, `av01`, `vp09`, `mp4a`, `Opus` or `tx3g` track. `mkv::tags_from_mp4` maps MP4 text metadata to tags.
- The Matroska timestamp rule: a value converts when it is a whole number of nanoseconds and a whole multiple of `TimestampScale`. Nothing rounds. `StreamCopy::timestamp_scale` picks a scale that holds a set of tracks. A value that no scale holds, such as an AAC frame at 44.1 or 48 kHz or a 29.97 fps frame in a 30000 timescale, is refused with an error that names MP4 and `StreamCopy::plan_rounded`.
- `mkv::StreamCopy::plan_rounded(track, packets, scale)`, `StreamCopy::rounding` and `mkv::TimestampRounding`: the opt-in that rounds an MP4 track to a `TimestampScale` the caller names. Each timestamp rounds on its own to the nearest tick, ties to even, in integers, so its error stays at most half a tick and no error accumulates. Block durations come from adjacent rounded timestamps, so they sum to the rounded end. `CodecDelay`, `DiscardPadding` and `DefaultDuration` round to the nearest nanosecond. Two packets on one tick are refused, and the error names a finer scale. `StreamCopy::plan` still refuses.
- `Language::iso639_mapped`: the ISO 639-2/T code of a track, with QuickTime Macintosh language codes mapped through a table. `Language::iso639` keeps its contract.
- An MP4 edit list maps to Matroska timing in `StreamCopy::plan`: an empty edit and a video `media_time` shift `pts`, an audio `media_time` becomes `CodecDelay`, and an audio end trim becomes `DiscardPadding`. An edit list with no Matroska form, and a frame an edit cuts from the start, are refused with an error that names MP4.
- `matroska::RawElement`, and `extra` on `MatroskaTrack`, `MatroskaVideo` and `MatroskaAudio`: the children containr does not model, such as `Colour`, kept raw for stream copy. `ContentEncoding::encryption_settings` keeps the raw `ContentEncryption` body.
- `Container::WebM` for WebM mux refusals.
- `Error::Unsupported`, `DtsOutOfOrder`, `CompositionOffset`, `SampleDescriptionChange`, `MissingDuration`, `InvalidPacket` and `InvalidMux` for mux refusals.
- `mp4::Muxer::set_edit_list`: replaces a track's edit list at any point before `finish`, so a single-pass writer sets an edit whose duration it learns at the end.
- `mp4::Muxer::buffered_bytes` and `mkv::Muxer::buffered_bytes`: the packet bytes a muxer holds in memory, to check the `max_buffer` bound.
- `Language::matroska_code` and `Language::mdhd_form`: the written form of a language in each container, for a hand-built track spec.
- `mkv::tags_from_mp4` maps a track's `udta/name` box, the title FFmpeg writes as bare text, to a `TITLE` tag. It refused the box.

### Changed

- `VideoInfo` and `MatroskaVideo` gain a `colour` field, so a struct literal of either needs it. The `Colour` element stays in `MatroskaVideo::extra`.
- `StreamCopy::timestamp_scale` starts from the plans' own scales, so a rounded plan keeps the scale it names. An exact set of plans gets the value it got before.
- MP4 to Matroska stream copy maps a QuickTime Macintosh language code to its ISO 639-2/T code. It wrote `und` for every such code. `0x7FFF` and a code with no table entry still write `und`.
- `mkv::Muxer::set_chapters` refuses a repeated non-zero `ChapterUID`, and names the UID and both chapter positions.
- The Matroska demuxer reads the discardable bit on a `Block` in a `BlockGroup`, where containr's muxer keeps the flag. Other muxers leave that bit at 0.
