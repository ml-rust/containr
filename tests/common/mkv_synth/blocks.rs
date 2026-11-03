//! Blocks and Clusters, with the packets their frames give.

use containr::{Lace, Packet};

use super::elements::{el, frame_bytes, id_bytes, size_bytes, uint};
use super::file::VIDEO;

/// One block of a Cluster: its element bytes and the packets of its frames,
/// with offsets relative to the element start.
pub(super) struct Block {
    pub(super) bytes: Vec<u8>,
    pub(super) packets: Vec<Packet>,
}

#[derive(Clone, Copy)]
pub(super) enum Laced {
    No,
    Xiph,
    Ebml,
    Fixed,
}

/// Lace header bytes for `sizes`.
fn lace_header(kind: Laced, sizes: &[u32]) -> Vec<u8> {
    let mut out = vec![(sizes.len() - 1) as u8];
    let head = &sizes[..sizes.len() - 1];
    match kind {
        Laced::No => return Vec::new(),
        Laced::Fixed => {}
        Laced::Xiph => {
            for &size in head {
                out.extend(std::iter::repeat_n(0xFF, (size / 255) as usize));
                out.push((size % 255) as u8);
            }
        }
        Laced::Ebml => {
            let mut previous = 0i64;
            for (n, &size) in head.iter().enumerate() {
                if n == 0 {
                    out.extend(size_bytes(size as usize));
                } else {
                    let diff = i64::from(size) - previous;
                    assert!((-63..=63).contains(&diff), "1-byte signed lace size");
                    out.push((diff + 63) as u8 | 0x80);
                }
                previous = i64::from(size);
            }
        }
    }
    out
}

/// What one block stores: the element kind and the facts its packets get.
pub(super) struct Spec {
    pub(super) track: u32,
    pub(super) relative: i16,
    pub(super) flags: u8,
    pub(super) kind: Laced,
    pub(super) sizes: &'static [u32],
    pub(super) group: Option<Group>,
}

pub(super) struct Group {
    pub(super) duration: Option<u64>,
    pub(super) reference: bool,
    pub(super) discard_padding: Option<i64>,
}

pub(super) fn block(
    spec: &Spec,
    cluster_ts: i64,
    counts: &mut [u64; 2],
    default_ticks: u64,
) -> Block {
    let laced = !matches!(spec.kind, Laced::No);
    let lacing_bits = match spec.kind {
        Laced::No => 0,
        Laced::Xiph => 0x02,
        Laced::Fixed => 0x04,
        Laced::Ebml => 0x06,
    };
    let mut body = vec![0x80 | spec.track as u8];
    body.extend(spec.relative.to_be_bytes());
    body.push(spec.flags | lacing_bits);
    body.extend(lace_header(spec.kind, spec.sizes));
    let slot = (spec.track - 1) as usize;
    let count = spec.sizes.len() as u64;
    let duration = spec.group.as_ref().and_then(|g| g.duration);
    let mut frames = Vec::new();
    for (k, &size) in spec.sizes.iter().enumerate() {
        let k = k as u64;
        let (shift, ticks) = match duration {
            Some(d) => (k * d / count, (k + 1) * d / count - k * d / count),
            None => (k * default_ticks, default_ticks),
        };
        let index = counts[slot];
        counts[slot] += 1;
        let padding = spec
            .group
            .as_ref()
            .and_then(|g| g.discard_padding)
            .filter(|&p| if p < 0 { k == 0 } else { k + 1 == count });
        frames.push(Packet {
            track_id: spec.track,
            index,
            dts: None,
            pts: cluster_ts + i64::from(spec.relative) + shift as i64,
            duration: Some(ticks),
            sync: match &spec.group {
                Some(group) => !group.reference,
                None => spec.flags & 0x80 != 0,
            },
            discardable: spec.group.is_none() && spec.flags & 0x01 != 0,
            discard_padding: padding,
            lace: laced.then_some(Lace {
                index: k as u16,
                count: count as u16,
            }),
            offset: body.len() as u64,
            size,
            description_index: 1,
        });
        body.extend(frame_bytes(spec.track, index, size));
    }
    let (bytes, start) = match &spec.group {
        None => {
            let bytes = el(0xA3, &body);
            let start = bytes.len() - body.len();
            (bytes, start)
        }
        Some(group) => {
            let block = el(0xA1, &body);
            let start = block.len() - body.len();
            let mut children = block;
            if let Some(d) = group.duration {
                children.extend(uint(0x9B, d));
            }
            if group.reference {
                children.extend(el(0xFB, &[0xD8]));
            }
            if let Some(p) = group.discard_padding {
                children.extend(el(0x75A2, &p.to_be_bytes()));
            }
            let bytes = el(0xA0, &children);
            let start = bytes.len() - children.len() + start;
            (bytes, start)
        }
    };
    for frame in &mut frames {
        frame.offset += start as u64;
    }
    Block {
        bytes,
        packets: frames,
    }
}

/// Build a Cluster from `specs`. Returns the element and its packets with
/// offsets relative to the element start, and the Timestamp offset.
pub(super) fn cluster(
    timestamp: u64,
    specs: &[Spec],
    known_size: bool,
    counts: &mut [u64; 2],
) -> (Vec<u8>, Vec<Packet>, usize) {
    let mut body = uint(0xE7, timestamp);
    let mut packets = Vec::new();
    for spec in specs {
        let default_ticks = if spec.track == VIDEO { 40 } else { 20 };
        let built = block(spec, timestamp as i64, counts, default_ticks);
        for mut packet in built.packets {
            packet.offset += body.len() as u64;
            packets.push(packet);
        }
        body.extend(built.bytes);
    }
    let mut out = id_bytes(0x1F43_B675);
    if known_size {
        out.extend(size_bytes(body.len()));
    } else {
        out.extend([0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }
    let start = out.len();
    out.extend(body);
    for packet in &mut packets {
        packet.offset += start as u64;
    }
    (out, packets, start)
}
