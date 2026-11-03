//! MP4/MOV mux tests: stream copy from FFmpeg-made MP4 and Matroska fixtures
//! read back with the demuxer and with ffprobe, faststart, `co64`, refusals
//! and a randomized sweep.

mod common;
mod mux;
