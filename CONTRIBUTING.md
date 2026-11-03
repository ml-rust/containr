# Contributing to containr

## Before you start

Open an issue before you start anything non-trivial. Discuss design on [Discord](https://discord.gg/jBhFk9kHPg).

## Build and test

Run these commands from the repository root. They match `.github/workflows/test.yml`.

```bash
cargo fmt --all --check
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

## Test fixtures

Media files never enter the repository. The tests read fixtures that two scripts generate:

```bash
python3 tests/fixtures/gen_mp4.py
python3 tests/fixtures/gen_mkv.py
```

The scripts need `ffmpeg` and `ffprobe` on `PATH`, built with libx264, libx265 and libopus. gen_mkv.py also needs libvpx-vp9. It skips the AV1 fixture without an AV1 encoder, and the attachment fixture without a TrueType font. The scripts write into `tests/fixtures/mp4/` and `tests/fixtures/mkv/`, which git ignores:

- `<name>.mp4`, `<name>.mov`, `<name>.mkv` or `<name>.webm`: the fixture.
- `<name>.probe.json`: the ffprobe reference data the tests compare against.

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
