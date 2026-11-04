#!/usr/bin/env python3
"""Generate the MP4/MOV demux fixtures and their ffprobe reference data.

Usage: python3 tests/fixtures/gen_mp4.py

Requires `ffmpeg` and `ffprobe` on PATH, with libx264, libx265, libopus and
the native AAC and mov_text encoders. The VP9 and AV1 fixtures need libvpx-vp9
and an AV1 encoder (libsvtav1, libaom-av1 or librav1e). Without one, the script
skips that fixture with a notice.

Writes into tests/fixtures/mp4/, which git ignores:
- <name>.mp4 or <name>.mov: the fixture.
- <name>.probe.json: the reference data the tests compare against.

The reference JSON holds:
- streams, packets, chapters: ffprobe output read with `-ignore_editlist 1`,
  so timestamps are the raw sample-table values. Each packet carries the
  Adler-32 of its bytes.
- edit_lists: the `elst` entries of each trak in file order, read from the
  ffprobe trace log of a second pass that keeps the edit lists.
- top_level: the top-level box types in file order.

The colour fixtures (colour_*) carry `-color_primaries`, `-color_trc`,
`-colorspace` and `-color_range` tags. The reference streams hold ffprobe's
`color_primaries`, `color_transfer`, `color_space` and `color_range`.
"""

import json
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent / "mp4"

FFMPEG = ["ffmpeg", "-nostdin", "-y", "-hide_banner", "-loglevel", "error"]
VIDEO_SRC = ["-f", "lavfi", "-i", "testsrc=size=96x64:rate=25"]
AUDIO_SRC = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"]
H264 = ["-c:v", "libx264", "-preset", "veryfast", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p"]
AAC = ["-c:a", "aac", "-ac", "2", "-b:a", "64k"]
DURATION = ["-t", "2"]

# Container colour tags. `setparams` tags the frames, which FFmpeg 9 needs
# before it writes primaries and transfer. The `-color_*` options tag the
# encoder as well. A fixture tags all four fields, so ffprobe reports all four.


def colour_args(primaries, transfer, matrix, range_name):
    options = {"tv": "tv", "pc": "pc"}[range_name]
    return [
        "-vf",
        f"setparams=color_primaries={primaries}:color_trc={transfer}:colorspace={matrix}:range={options}",
        "-color_primaries", primaries,
        "-color_trc", transfer,
        "-colorspace", matrix,
        "-color_range", range_name,
    ]


BT709 = colour_args("bt709", "bt709", "bt709", "tv")
BT709_FULL = colour_args("bt709", "bt709", "bt709", "pc")
BT2020_PQ = colour_args("bt2020", "smpte2084", "bt2020nc", "tv")

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

# Each AV1 encoder with the flags that keep it fast on a 96x64 source.
AV1_ENCODERS = [
    ("libsvtav1", ["-preset", "12"]),
    ("libaom-av1", ["-cpu-used", "8", "-row-mt", "1"]),
    ("librav1e", ["-speed", "10"]),
]

ELST_ENTRY = re.compile(r"duration=(-?\d+) time=(-?\d+) rate=(-?[0-9.]+)")
TRAK_LINE = re.compile(r"type:'trak' parent:'moov'")


def run(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"command failed ({result.returncode}): {' '.join(args)}\n{result.stderr}")
    return result


def encoders():
    listing = run(["ffmpeg", "-hide_banner", "-encoders"]).stdout
    return {line.split()[1] for line in listing.splitlines() if len(line.split()) > 1}


def top_level_boxes(path):
    kinds = []
    with open(path, "rb") as f:
        while True:
            header = f.read(8)
            if len(header) < 8:
                break
            size, kind = struct.unpack(">I4s", header)
            header_len = 8
            if size == 1:
                size = struct.unpack(">Q", f.read(8))[0]
                header_len = 16
            kinds.append(kind.decode("latin-1"))
            if size == 0:
                break
            f.seek(size - header_len, 1)
    return kinds


def edit_lists(path):
    """Return the elst entries of each trak, in file order, from the trace log."""
    trace = run(["ffprobe", "-v", "trace", "-show_format", "-of", "json", str(path)]).stderr
    traks = []
    for line in trace.splitlines():
        if TRAK_LINE.search(line):
            traks.append([])
            continue
        match = ELST_ENTRY.search(line)
        if match and traks:
            duration, time, rate = match.groups()
            # The trace prints the rate as a float. The tests compare the raw
            # 16.16 value, so convert it once here.
            traks[-1].append(
                {
                    "segment_duration": int(duration),
                    "media_time": int(time),
                    "media_rate": round(float(rate) * 65536),
                }
            )
    return traks


def probe(path):
    raw = run(
        [
            "ffprobe",
            "-v",
            "error",
            "-ignore_editlist",
            "1",
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
        "edit_lists": edit_lists(path),
        "top_level": top_level_boxes(path),
    }
    reference_path = path.with_suffix(".probe.json")
    reference_path.write_text(json.dumps(reference, indent=1))


def make(name, args):
    path = OUT / name
    run(FFMPEG + args + [str(path)])
    probe(path)
    print(f"wrote {path}")
    return path


def make_colour():
    """Fixtures whose colour tags sit in the container: `colr` boxes."""
    OUT.mkdir(parents=True, exist_ok=True)
    make("colour_bt709.mp4", VIDEO_SRC + DURATION + H264 + BT709)
    make(
        "colour_bt2020_pq.mp4",
        VIDEO_SRC + DURATION
        + ["-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error", "-pix_fmt", "yuv420p"]
        + BT2020_PQ,
    )
    make("colour_bt709_full.mov", VIDEO_SRC + DURATION + H264 + BT709_FULL)
    make("colour_bt2020_pq.mov", VIDEO_SRC + DURATION + H264 + BT2020_PQ)


def main():
    for tool in ("ffmpeg", "ffprobe"):
        if shutil.which(tool) is None:
            sys.exit(f"{tool} is not on PATH. Install FFmpeg, then run this script again.")
    OUT.mkdir(parents=True, exist_ok=True)

    srt = OUT / "subtitle.srt"
    srt.write_text(SRT)
    meta = OUT / "chapters.ffmeta"
    meta.write_text(FFMETADATA)

    base = make("h264_aac_moov_end.mp4", VIDEO_SRC + AUDIO_SRC + DURATION + H264 + AAC)
    faststart = make(
        "h264_aac_faststart.mp4",
        VIDEO_SRC + AUDIO_SRC + DURATION + H264 + AAC + ["-movflags", "+faststart"],
    )
    make(
        "h264_negative_cts.mp4",
        VIDEO_SRC + DURATION + H264 + ["-movflags", "+negative_cts_offsets"],
    )
    make(
        "hevc_opus.mp4",
        VIDEO_SRC
        + AUDIO_SRC
        + DURATION
        + ["-vf", "setsar=4/3", "-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error:bframes=2"]
        + ["-c:a", "libopus", "-b:a", "48k"],
    )
    make(
        "mov_text.mp4",
        VIDEO_SRC + ["-i", str(srt)] + DURATION + H264 + ["-c:s", "mov_text", "-metadata:s:s:0", "language=eng"],
    )
    make(
        "fragmented.mp4",
        VIDEO_SRC + DURATION + H264 + ["-movflags", "+frag_keyframe+empty_moov"],
    )
    make("rotate90.mp4", ["-display_rotation", "90", "-i", str(faststart), "-c", "copy"])
    make("itsoffset.mp4", ["-itsoffset", "0.5", "-i", str(base), "-c", "copy"])
    make("ss_copy.mp4", ["-ss", "0.6", "-i", str(base), "-c", "copy"])
    make(
        "chapters_mp4.mp4",
        VIDEO_SRC + AUDIO_SRC + ["-i", str(meta)] + DURATION + H264 + AAC
        + ["-map", "0:v", "-map", "1:a", "-map_metadata", "2", "-map_chapters", "2"],
    )
    make(
        "chapters_mov.mov",
        VIDEO_SRC + AUDIO_SRC + ["-i", str(meta)] + DURATION + H264 + AAC
        + ["-map", "0:v", "-map", "1:a", "-map_metadata", "2", "-map_chapters", "2"],
    )
    # 1024 ticks at 32 kHz last a whole number of nanoseconds, unlike 44.1 and
    # 48 kHz, so this AAC track has exact Matroska timestamps.
    make(
        "aac_32k.mp4",
        ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000"] + DURATION + AAC + ["-ar", "32000"],
    )
    # 1001/30000 s and 1024/48000 s are no whole number of nanoseconds, so this
    # file copies to Matroska only through StreamCopy::plan_rounded.
    make(
        "ntsc_h264_aac.mp4",
        ["-f", "lavfi", "-i", "testsrc=size=96x64:rate=30000/1001"] + AUDIO_SRC + DURATION + H264 + AAC
        + ["-video_track_timescale", "30000"],
    )
    # A MOV stores these tags as Macintosh language codes. The last stream has
    # no tag, which is code 0x7FFF. 32 kHz keeps the AAC timestamps exact.
    aac_32k = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000"]
    make(
        "languages.mov",
        VIDEO_SRC + aac_32k + DURATION + H264 + AAC + ["-ar", "32000"]
        + ["-map", "0:v", "-map", "1:a", "-map", "1:a", "-map", "1:a", "-map", "1:a"]
        + ["-metadata:s:v:0", "language=jpn", "-metadata:s:a:0", "language=eng"]
        + ["-metadata:s:a:1", "language=fra", "-metadata:s:a:2", "language=spa"],
    )
    make_colour()
    available = encoders()
    if "libvpx-vp9" in available:
        make(
            "vp9_opus.mp4",
            VIDEO_SRC + AUDIO_SRC + DURATION
            + ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "200k"]
            + ["-c:a", "libopus", "-b:a", "48k"],
        )
    else:
        print("skipping vp9_opus.mp4: this ffmpeg has no libvpx-vp9 encoder")
    for encoder, flags in AV1_ENCODERS:
        if encoder in available:
            make("av1.mp4", VIDEO_SRC + DURATION + ["-c:v", encoder] + flags + ["-pix_fmt", "yuv420p"])
            break
    else:
        print("skipping av1.mp4: this ffmpeg has no AV1 encoder (libsvtav1, libaom-av1 or librav1e)")


if __name__ == "__main__":
    main()
