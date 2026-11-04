# Contributing to containr

## Before you start

Open an issue before you start anything non-trivial. Discuss design on [Discord](https://discord.gg/jBhFk9kHPg).

## Prerequisites

- Rust 1.95 or newer.
- `cargo-deny` and `shellcheck` for the checks.
- `python3`, `ffmpeg`, and `ffprobe` to generate fixtures.

## Build and test

Run these commands from the repository root.

```bash
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features mp4 -- -D warnings
cargo clippy --all-targets --features mkv -- -D warnings
cargo clippy --all-targets --features mp4,mkv -- -D warnings
python3 tests/fixtures/gen_mp4.py
python3 tests/fixtures/gen_mkv.py
cargo test --all-features --no-fail-fast
cargo deny --exclude-dev check
shellcheck scripts/ci/*.sh
```

CI also checks the library on the MSRV in `Cargo.toml`.

## Test fixtures

Media files never enter the repository. The two scripts above generate fixtures into `tests/fixtures/mp4/` and `tests/fixtures/mkv/`, which git ignores.

| Output | Contents |
| ------ | -------- |
| `<name>.mp4`, `<name>.mov`, `<name>.mkv`, `<name>.webm` | The fixture |
| `<name>.probe.json` | The ffprobe reference data the tests compare against |

Both scripts need `ffmpeg` and `ffprobe` on `PATH`, with libx264, libx265, and libopus. This table lists the other encoders:

| Missing | `gen_mp4.py` | `gen_mkv.py` |
| ------- | ------------ | ------------ |
| libvpx-vp9 | Skips its VP9 fixture | Required |
| An AV1 encoder (libsvtav1, libaom-av1, or librav1e) | Skips its AV1 fixture | Skips its AV1 fixture |
| A TrueType font | Not needed | Skips the attachment fixture |

A test whose fixture is missing prints a notice and skips. The synthetic and corruption tests build their input in memory and always run.

## Conventions

- Put boxes, EBML elements, sample tables, packets, track metadata, edit lists, and stream copy in containr. Put codecs, decode, and encode in [vidr](https://github.com/ml-rust/vidr) and [voxr](https://github.com/ml-rust/voxr).
- Make every container a Cargo feature. Keep the default build minimal.
- Keep timestamps as integers in the track timescale.
- Expose edit lists raw. Never apply one in the demuxer.
- Return an error on malformed input. Bound every size from the file before allocation.
- Keep the MSRV at the `rust-version` in `Cargo.toml`. CI checks it.

## Pull requests

- Add tests for every behavior change.
- Add a line to `CHANGELOG.md` for every user-visible change.
- Run the commands in [Build and test](#build-and-test) before you open the pull request.

## Releases

- Each release needs a non-empty `## [X.Y.Z]` section in `CHANGELOG.md`.
- A pushed `v*` tag runs the `Release Prepare` workflow. `Release Validate` refuses a tag whose version has no entry.
- The manual `Release` workflow publishes. The GitHub Release body is the changelog section for that version.
