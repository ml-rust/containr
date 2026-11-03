//! A small MP4 built in memory, so demux and corruption tests run without
//! FFmpeg. The tables are known exactly: a video track with signed `ctts`,
//! `stss` and an edit list, and an audio track with interleaved chunks.

use containr::Packet;

pub const VIDEO_ID: u32 = 1;
pub const AUDIO_ID: u32 = 2;
pub const VIDEO_TIMESCALE: u32 = 1000;
pub const AUDIO_TIMESCALE: u32 = 48_000;
pub const MOVIE_TIMESCALE: u32 = 1000;
pub const VIDEO_SIZES: [u32; 6] = [10, 3, 4, 5, 11, 6];
pub const VIDEO_OFFSETS: [i32; 6] = [0, 200, -100, 0, 200, -100];
pub const VIDEO_SYNC: [u32; 2] = [1, 5];
pub const AUDIO_SIZE: u32 = 7;
pub const AUDIO_SAMPLES: u32 = 4;
pub const AVCC: [u8; 7] = [1, 0x42, 0xC0, 0x1E, 0xFF, 0xE0, 0x00];
pub const ESDS: [u8; 9] = [0, 0, 0, 0, 3, 4, 0, 1, 0];
pub const EDIT: (u32, i32) = (600, 100);

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut head = flags.to_be_bytes();
    head[0] = version;
    let mut all = head.to_vec();
    all.extend_from_slice(body);
    bx(kind, &all)
}

fn be32(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

const MATRIX: [u32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

fn tkhd(id: u32, duration: u32, width: u32, height: u32) -> Vec<u8> {
    let mut body = be32(&[0, 0, id, 0, duration, 0, 0]);
    body.extend([0, 0, 0, 0, 0, 0, 0, 0]);
    body.extend(be32(&MATRIX));
    body.extend(be32(&[width << 16, height << 16]));
    full(b"tkhd", 0, 3, &body)
}

fn mdia(timescale: u32, duration: u32, handler: &[u8; 4], stbl: Vec<u8>) -> Vec<u8> {
    let mut mdhd = be32(&[0, 0, timescale, duration]);
    mdhd.extend([0x55, 0xC4, 0, 0]);
    let mut hdlr = vec![0, 0, 0, 0];
    hdlr.extend_from_slice(handler);
    hdlr.extend([0; 12]);
    hdlr.extend(b"synth\0");
    let mut body = full(b"mdhd", 0, 0, &mdhd);
    body.extend(full(b"hdlr", 0, 0, &hdlr));
    body.extend(bx(b"minf", &bx(b"stbl", &stbl)));
    bx(b"mdia", &body)
}

fn table(kind: &[u8; 4], version: u8, rows: &[Vec<u32>]) -> Vec<u8> {
    let mut body = (rows.len() as u32).to_be_bytes().to_vec();
    for row in rows {
        body.extend(be32(row));
    }
    full(kind, version, 0, &body)
}

fn stsz(sizes: &[u32]) -> Vec<u8> {
    let mut body = be32(&[0, sizes.len() as u32]);
    body.extend(be32(sizes));
    full(b"stsz", 0, 0, &body)
}

fn video_trak(chunks: &[u32]) -> Vec<u8> {
    let mut entry = vec![0, 0, 0, 0, 0, 0, 0, 1];
    entry.extend([0; 16]);
    entry.extend([0, 96, 0, 64]);
    entry.extend(be32(&[0x0048_0000, 0x0048_0000, 0]));
    entry.extend([0, 1]);
    entry.extend([0; 32]);
    entry.extend([0, 0x18, 0xFF, 0xFF]);
    entry.extend(bx(b"avcC", &AVCC));
    entry.extend(bx(b"pasp", &be32(&[4, 3])));
    let mut stsd = be32(&[1]);
    stsd.extend(bx(b"avc1", &entry));

    let mut stbl = full(b"stsd", 0, 0, &stsd);
    stbl.extend(table(b"stts", 0, &[vec![6, 100]]));
    let ctts: Vec<Vec<u32>> = VIDEO_OFFSETS.iter().map(|&o| vec![1, o as u32]).collect();
    stbl.extend(table(b"ctts", 1, &ctts));
    stbl.extend(table(b"stsc", 0, &[vec![1, 2, 1]]));
    stbl.extend(stsz(&VIDEO_SIZES));
    let offsets: Vec<Vec<u32>> = chunks.iter().map(|&c| vec![c]).collect();
    stbl.extend(table(b"stco", 0, &offsets));
    let sync: Vec<Vec<u32>> = VIDEO_SYNC.iter().map(|&s| vec![s]).collect();
    stbl.extend(table(b"stss", 0, &sync));

    let mut elst = be32(&[1, EDIT.0, EDIT.1 as u32]);
    elst.extend([0, 1, 0, 0]);
    let mut body = tkhd(VIDEO_ID, 600, 96, 64);
    body.extend(bx(b"edts", &full(b"elst", 0, 0, &elst)));
    body.extend(mdia(VIDEO_TIMESCALE, 600, b"vide", stbl));
    bx(b"trak", &body)
}

fn audio_trak(chunks: &[u32]) -> Vec<u8> {
    let mut entry = vec![0, 0, 0, 0, 0, 0, 0, 1];
    entry.extend([0; 8]);
    entry.extend([0, 2, 0, 16, 0, 0, 0, 0]);
    entry.extend(be32(&[AUDIO_TIMESCALE << 16]));
    entry.extend(bx(b"esds", &ESDS));
    let mut stsd = be32(&[1]);
    stsd.extend(bx(b"mp4a", &entry));

    let mut stbl = full(b"stsd", 0, 0, &stsd);
    stbl.extend(table(b"stts", 0, &[vec![AUDIO_SAMPLES, 1024]]));
    stbl.extend(table(b"stsc", 0, &[vec![1, 2, 1]]));
    stbl.extend(full(b"stsz", 0, 0, &be32(&[AUDIO_SIZE, AUDIO_SAMPLES])));
    let offsets: Vec<Vec<u32>> = chunks.iter().map(|&c| vec![c]).collect();
    stbl.extend(table(b"stco", 0, &offsets));

    let mut body = tkhd(AUDIO_ID, 85, 0, 0);
    body.extend(mdia(AUDIO_TIMESCALE, AUDIO_SAMPLES * 1024, b"soun", stbl));
    bx(b"trak", &body)
}

fn moov(video_chunks: &[u32], audio_chunks: &[u32]) -> Vec<u8> {
    let mut mvhd = be32(&[0, 0, MOVIE_TIMESCALE, 600, 0x10000]);
    mvhd.extend([1, 0, 0, 0]);
    mvhd.extend([0; 8]);
    mvhd.extend(be32(&MATRIX));
    mvhd.extend([0; 24]);
    mvhd.extend(be32(&[3]));
    let mut body = full(b"mvhd", 0, 0, &mvhd);
    body.extend(video_trak(video_chunks));
    body.extend(audio_trak(audio_chunks));
    let mut ilst_item = be32(&[1, 0]);
    ilst_item.extend(b"synth title");
    let ilst = bx(&[0xA9, b'n', b'a', b'm'], &bx(b"data", &ilst_item));
    let mut meta = full(
        b"hdlr",
        0,
        0,
        &[
            0, 0, 0, 0, b'm', b'd', b'i', b'r', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    );
    meta.extend(bx(b"ilst", &ilst));
    // Version 1 chpl: 4 reserved bytes, then the entry count.
    let mut chpl = vec![0, 0, 0, 0, 1];
    chpl.extend(0u64.to_be_bytes());
    chpl.push(5);
    chpl.extend(b"Start");
    let mut udta = full(b"meta", 0, 0, &meta);
    udta.extend(full(b"chpl", 1, 0, &chpl));
    body.extend(bx(b"udta", &udta));
    bx(b"moov", &body)
}

/// The sample byte pattern: track id, sample index, then filler.
fn sample_bytes(track: u32, index: u32, size: u32) -> Vec<u8> {
    (0..size)
        .map(|i| (track * 64 + index * 8 + i) as u8)
        .collect()
}

/// Build the movie. Returns the file bytes and the packets in file order.
pub fn movie(moov_first: bool) -> (Vec<u8>, Vec<Packet>) {
    let ftyp = bx(
        b"ftyp",
        &[
            b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o', b'm', b'm', b'p', b'4', b'1',
        ],
    );
    let moov_len = moov(&[0; 3], &[0; 2]).len();
    let mdat_start = ftyp.len() + if moov_first { moov_len } else { 0 };
    let mut data = Vec::new();
    let mut packets = Vec::new();
    let mut video_chunks = Vec::new();
    let mut audio_chunks = Vec::new();
    let mut dts = [0i64; 2];
    let layout: [(u32, &[u32]); 5] = [
        (VIDEO_ID, &[0, 1]),
        (AUDIO_ID, &[0, 1]),
        (VIDEO_ID, &[2, 3]),
        (AUDIO_ID, &[2, 3]),
        (VIDEO_ID, &[4, 5]),
    ];
    for (track, samples) in layout {
        let chunk_offset = (mdat_start + 8 + data.len()) as u32;
        if track == VIDEO_ID {
            video_chunks.push(chunk_offset);
        } else {
            audio_chunks.push(chunk_offset);
        }
        for &index in samples {
            let (size, duration, offset, sync) = if track == VIDEO_ID {
                let i = index as usize;
                (
                    VIDEO_SIZES[i],
                    100,
                    i64::from(VIDEO_OFFSETS[i]),
                    VIDEO_SYNC.contains(&(index + 1)),
                )
            } else {
                (AUDIO_SIZE, 1024, 0, true)
            };
            let slot = (track - 1) as usize;
            packets.push(Packet {
                track_id: track,
                index: u64::from(index),
                dts: dts[slot],
                pts: dts[slot] + offset,
                duration,
                sync,
                offset: (mdat_start + 8 + data.len()) as u64,
                size,
                description_index: 1,
            });
            dts[slot] += i64::from(duration);
            data.extend(sample_bytes(track, index, size));
        }
    }
    let moov = moov(&video_chunks, &audio_chunks);
    let mdat = bx(b"mdat", &data);
    let mut out = ftyp;
    if moov_first {
        out.extend(moov);
        out.extend(mdat);
    } else {
        out.extend(mdat);
        out.extend(moov);
    }
    (out, packets)
}

/// The bytes the synthetic movie stores for `packet`.
pub fn packet_bytes(packet: &Packet) -> Vec<u8> {
    sample_bytes(packet.track_id, packet.index as u32, packet.size)
}
