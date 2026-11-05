# containr — Agent Guidelines

Media containers for Rust: MP4/MOV and MKV/WebM demux, mux and edit lists. Shared by vidr (video) and voxr (audio).

## Placement

- `containr`: boxes, EBML elements, sample tables, packets with timestamps, track metadata, edit lists, stream copy.
- vidr and voxr: codecs, decode, encode, frame math. NEVER decode or encode media here.
- NEVER parse or write a container in vidr or voxr. Add it here, then depend on it.

## Invariants

- Library only. No binary, no CLI.
- Each container is a Cargo feature: `mp4`, `mkv`. The default build is minimal.
- Timestamps are integers in the track timescale. NEVER convert a timestamp to floating point.
- Demux exposes raw edit lists. The caller applies them. NEVER apply an edit list silently. `mkv::StreamCopy` maps an MP4 edit list into its plan, visible in `timing()`, since Matroska has no edit list.
- Stream copy keeps packet bytes, timestamps, durations and codec configuration unchanged. The one exception is `mkv::StreamCopy::plan_rounded`, an explicit opt-in. NEVER round by default. A last Matroska audio or video packet with no stored duration takes one from the last two timestamps, less its `DiscardPadding`. No other packet gains a duration.
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
| MP4/MOV demux (`mp4`): tracks, sample tables, packets, raw edit lists, `udta`/`meta` metadata, `chpl` and chapter-track chapters, `colr` colour tags (`nclx`, `nclc`) | Implemented |
| Fragmented MP4 (`moof`) demux | Roadmap |
| Track timing read with no sample tables (`mp4`): `mp4::first_track_timing`, the `mdhd` timescale and raw edit list of the first track of a kind. Reads only `moov`, so it accepts a fragmented file and yields no packets | Implemented |
| MP4/MOV mux (`mp4`): `ftyp`, `mdat`, `moov`, faststart, chunk interleave, `stts`, `ctts`, `cslg`, `stsc`, `stsz`, `stco`/`co64`, `stss`, edit lists, `udta`/`meta`/`keys` metadata, `chpl` and chapter-track chapters, `nclx` `colr` colour tags, named refusals | Implemented |
| MKV/WebM demux (`mkv`): EBML header, `SeekHead`, `Info`, `Tracks`, Clusters of known and unknown size, `SimpleBlock` and `BlockGroup`, Xiph, EBML and fixed lacing, `Cues`, `Chapters`, `Tags`, `Attachments`, `Video/Colour` tags | Implemented |
| Track trims read with no packet index (`mkv`): `mkv::first_track_timing`, the `CodecDelay` and summed positive `DiscardPadding` of the first track of a kind. Reads only the EBML header, `Tracks` and Clusters | Implemented |
| MKV/WebM mux (`mkv`): EBML header, `SeekHead`, `Info`, `Tracks`, Clusters with `SimpleBlock` and `BlockGroup`, explicit last-block `BlockDuration`, `DiscardPadding`, `Cues`, `Chapters`, `Tags`, `Attachments`, `Video/Colour` tags, WebM codec refusals, targets with no seek | Implemented |
| Stream copy into MP4/MOV (`mp4`): `TrackSpec::copy_from` and `StreamCopy`, from MP4 tracks, and from Matroska H.264, HEVC, AV1, AAC, Opus and `S_TEXT/UTF8` tracks | Implemented |
| Stream copy into MKV/WebM (`mkv`): `TrackSpec::copy_from` and `StreamCopy`, from Matroska tracks, and from MP4 `avc1`, `avc3`, `hvc1`, `hev1`, `av01`, `vp09`, `mp4a`, `Opus` and `tx3g` tracks. `StreamCopy::plan` takes whole nanoseconds only. `StreamCopy::plan_rounded` rounds to a named `TimestampScale` | Implemented |

Update this table in the same change that implements a capability.

## Commands

- `cargo clippy --all-targets --features <set> -- -D warnings`, for no features, `mp4`, `mkv`, and `mp4,mkv`
- `python3 tests/fixtures/gen_mp4.py` and `python3 tests/fixtures/gen_mkv.py`, before the tests
- `cargo nextest run --all-features --no-fail-fast`
- `cargo test --all-features --doc`, because nextest skips doctests
- `RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps`
- `cargo fmt --all --check`
- `cargo deny --exclude-dev check`
- `shellcheck scripts/ci/*.sh`
