#!/usr/bin/env python3
"""Generate the Matroska/WebM demux fixtures and their ffprobe reference data.

Usage: python3 tests/fixtures/gen_mkv.py

Requires `ffmpeg` and `ffprobe` on PATH, with libx264, libx265, libvpx-vp9,
libopus, and the native AAC, SubRip and ASS encoders. The AV1 fixture needs
libaom-av1, libsvtav1 or librav1e. Without one, the script skips it with a
notice. The attachment fixture needs a TrueType font on disk. Without one,
the script skips it with a notice.

Writes into tests/fixtures/mkv/, which git ignores:
- <name>.mkv or <name>.webm: the fixture.
- <name>.probe.json: the reference data the tests compare against.

The reference JSON holds `streams`, `packets`, `chapters` and `format`:
ffprobe output with the Adler-32 of each packet's bytes.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent / "mkv"

FFMPEG = ["ffmpeg", "-nostdin", "-y", "-hide_banner", "-loglevel", "error"]
VIDEO_SRC = ["-f", "lavfi", "-i", "testsrc=size=96x64:rate=25"]
AUDIO_SRC = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"]
H264 = ["-c:v", "libx264", "-preset", "veryfast", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p"]
HEVC = ["-c:v", "libx265", "-x265-params", "log-level=error:bframes=2"]
VP9 = ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "200k"]
AAC = ["-c:a", "aac", "-ac", "2", "-b:a", "64k"]
OPUS = ["-c:a", "libopus", "-b:a", "48k"]
DURATION = ["-t", "2"]

# Each AV1 encoder with the flags that keep it fast on a 96x64 source.
AV1_ENCODERS = [
    ("libsvtav1", ["-preset", "12"]),
    ("libaom-av1", ["-cpu-used", "8", "-row-mt", "1"]),
    ("librav1e", ["-speed", "10"]),
]

FONT_CANDIDATES = [
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
]

SRT = """1
00:00:00,200 --> 00:00:00,900
first line

2
00:00:01,100 --> 00:00:01,800
second line
"""

FFMETADATA = """;FFMETADATA1
title=containr fixture

[CHAPTER]
TIMEBASE=1/1000
START=0
END=800
title=Opening

[CHAPTER]
TIMEBASE=1/1000
START=800
END=2000
title=Closing
"""


def run(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"command failed ({result.returncode}): {' '.join(args)}\n{result.stderr}")
    return result


def encoders():
    listing = run(["ffmpeg", "-hide_banner", "-encoders"]).stdout
    return {line.split()[1] for line in listing.splitlines() if len(line.split()) > 1}


def probe(path):
    raw = run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_streams",
            "-show_packets",
            "-show_data_hash",
            "adler32",
            "-show_chapters",
            "-show_format",
            "-of",
            "json",
            str(path),
        ]
    ).stdout
    data = json.loads(raw)
    reference = {
        "streams": data.get("streams", []),
        "packets": data.get("packets", []),
        "chapters": data.get("chapters", []),
        "format": data.get("format", {}),
    }
    path.with_suffix(".probe.json").write_text(json.dumps(reference, indent=1))


def make(name, args):
    path = OUT / name
    run(FFMPEG + args + [str(path)])
    probe(path)
    print(f"wrote {path}")
    return path


def make_av1(available):
    for encoder, flags in AV1_ENCODERS:
        if encoder in available:
            make("av1.webm", VIDEO_SRC + DURATION + ["-c:v", encoder] + flags + ["-pix_fmt", "yuv420p"])
            return
    print("skipping av1.webm: this ffmpeg has no AV1 encoder (libsvtav1, libaom-av1 or librav1e)")


def vint_len(first):
    """The length of an EBML variable-size integer from its first byte."""
    for length in range(1, 9):
        if first & (0x80 >> (length - 1)):
            return length
    sys.exit(f"byte {first:#04x} starts no EBML variable-size integer")


def unknown_size_clusters(path):
    """Rewrite each Cluster size as the unknown-size marker of the same length.

    FFmpeg writes every Cluster with a known size, even with `-live 1`. A
    browser MediaRecorder or a streaming muxer writes them with unknown size.
    The rewrite keeps every byte offset, so ffprobe packets stay comparable.
    """
    data = bytearray(path.read_bytes())
    segment = data.find(bytes.fromhex("18538067"))
    if segment < 0:
        sys.exit(f"{path} has no Segment")
    pos = segment + 4
    size_len = vint_len(data[pos])
    pos += size_len
    clusters = 0
    while pos < len(data):
        id_len = vint_len(data[pos])
        element = bytes(data[pos : pos + id_len])
        size_at = pos + id_len
        size_len = vint_len(data[size_at])
        raw = int.from_bytes(data[size_at : size_at + size_len], "big")
        size = raw & ((1 << (7 * size_len)) - 1)
        if element == bytes.fromhex("1f43b675"):
            unknown = (1 << (7 * size_len)) - 1 | (1 << (7 * size_len))
            data[size_at : size_at + size_len] = unknown.to_bytes(size_len, "big")
            clusters += 1
        pos = size_at + size_len + size
    if clusters == 0:
        sys.exit(f"{path} has no Cluster to rewrite")
    path.write_bytes(data)


def make_live():
    path = OUT / "live.webm"
    run(FFMPEG + VIDEO_SRC + AUDIO_SRC + DURATION + VP9 + OPUS + ["-live", "1", "-f", "webm", str(path)])
    unknown_size_clusters(path)
    probe(path)
    print(f"wrote {path}")


def make_attachment(meta):
    font = next((Path(p) for p in FONT_CANDIDATES if Path(p).is_file()), None)
    if font is None:
        print(f"skipping attachment.mkv: no TrueType font at any of {FONT_CANDIDATES}")
        return
    make(
        "attachment.mkv",
        VIDEO_SRC + ["-i", str(meta)] + DURATION + H264
        + ["-map", "0:v", "-map_metadata", "1", "-attach", str(font)]
        + ["-metadata:s:t:0", "mimetype=font/ttf", "-metadata:s:t:0", "filename=fixture.ttf"],
    )


def main():
    for tool in ("ffmpeg", "ffprobe"):
        if shutil.which(tool) is None:
            sys.exit(f"{tool} is not on PATH. Install FFmpeg, then run this script again.")
    OUT.mkdir(parents=True, exist_ok=True)
    available = encoders()

    srt = OUT / "subtitle.srt"
    srt.write_text(SRT)
    meta = OUT / "chapters.ffmeta"
    meta.write_text(FFMETADATA)

    base = make("h264_aac.mkv", VIDEO_SRC + AUDIO_SRC + DURATION + H264 + AAC)
    make("hevc_opus.mkv", VIDEO_SRC + AUDIO_SRC + DURATION + HEVC + OPUS)
    make("vp9_opus.webm", VIDEO_SRC + AUDIO_SRC + DURATION + VP9 + OPUS)
    make_av1(available)
    make(
        "subtitles.mkv",
        VIDEO_SRC + ["-i", str(srt), "-i", str(srt)] + DURATION + H264
        + ["-map", "0:v", "-map", "1:s", "-map", "2:s", "-c:s:0", "srt", "-c:s:1", "ass"]
        + ["-metadata:s:s:0", "language=eng", "-metadata:s:s:1", "language=fre"],
    )
    make(
        "chapters.mkv",
        VIDEO_SRC + AUDIO_SRC + ["-i", str(meta)] + DURATION + H264 + AAC
        + ["-map", "0:v", "-map", "1:a", "-map_metadata", "2", "-map_chapters", "2"],
    )
    make_attachment(meta)
    make("rotate90.mkv", ["-display_rotation", "90", "-i", str(base), "-c", "copy"])
    # 1.013 s of 48 kHz audio is not a whole number of 20 ms Opus frames, so
    # the last block carries DiscardPadding.
    make("opus_trim.webm", AUDIO_SRC + ["-t", "1.013"] + OPUS)
    # A live WebM: a Segment and Clusters of unknown size.
    make_live()


if __name__ == "__main__":
    main()
