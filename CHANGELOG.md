# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each release must have a non-empty section here before it can be tagged — `.github/workflows/release-validate.yml` refuses a tag whose version has no entry, and the GitHub Release body is this file's section for that version.

## [Unreleased]

### Added

- MP4 and QuickTime MOV demux behind the `mp4` feature: tracks, sample entries with codec configuration, packets with `dts`, `pts`, duration and sync flag, raw edit lists, `udta`/`meta` metadata, and `chpl` and chapter-track chapters. `moov` can sit before or after `mdat`.
- Fragmented MP4 is refused with `Error::Fragmented`.
