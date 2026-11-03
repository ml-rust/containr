# containr — Agent Guidelines

Media containers for Rust: MP4/MOV and MKV/WebM demux, mux and edit lists. vidr (video) and voxr (audio) both build on it.

## Placement

- `containr`: boxes, EBML elements, sample tables, packets with timestamps, track metadata, edit lists, stream copy.
- vidr and voxr: codecs, decode, encode, frame math. NEVER decode or encode media here.
- NEVER parse or write a container in vidr or voxr. Add it here, then depend on it.

## Invariants

- Library only. No binary, no CLI.
- Each container is a Cargo feature: `mp4`, `mkv`. The default build is minimal.
- Timestamps are integers in the track timescale. NEVER convert a timestamp to floating point.
- Demux exposes raw edit lists. The caller applies them. NEVER apply an edit list silently.
- Stream copy keeps packet bytes, timestamps, durations and codec configuration unchanged.
- A mux refuses a stream the container cannot hold, and the error names the container that can.
- Demux on malformed input returns an error. NEVER panic, and NEVER allocate from a size the file declares without a bound.
- Every writer has a round-trip test: write, read back, compare packets and metadata. A test that reads only files we wrote proves nothing; also read files produced by another muxer.
- NEVER commit media. Tests generate fixtures, or read them from a directory at run time.
- A dependency must have a release or commit in the last year. Otherwise write our own.

## Implemented vs roadmap

This table is the single source of truth on what works. NEVER describe a roadmap item as working in code, docs, or replies.

| Capability | Status  |
| ---------- | ------- |
| Crate, CI, release workflows | Implemented |
| MP4/MOV demux (`mp4`): tracks, sample tables, packets, raw edit lists, `udta`/`meta` metadata, `chpl` and chapter-track chapters | Implemented |
| Fragmented MP4 (`moof`) demux | Roadmap |
| MP4/MOV mux | Roadmap |
| MKV/WebM demux and mux (`mkv`) | Roadmap |
| Stream copy | Roadmap |

Update this table in the same change that implements a capability.

## Commands

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo nextest run --all-features --no-fail-fast`
- `cargo fmt --all --check`
- `cargo deny --exclude-dev check`
