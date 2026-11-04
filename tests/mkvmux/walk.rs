//! A small EBML walk of a file the muxer writes, separate from the demuxer, so
//! a layout test does not trust the code it checks.

/// One block as the Cluster stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stored {
    pub track: u64,
    pub cluster_timestamp: u64,
    pub relative: i16,
    pub flags: u8,
    /// True for a `SimpleBlock`, false for a `BlockGroup`.
    pub simple: bool,
    pub duration: Option<u64>,
    pub has_reference: bool,
    pub padding: bool,
    /// The block's offset in the Cluster body.
    pub at: u64,
}

/// What the walk finds.
#[derive(Debug, Default)]
pub struct Layout {
    pub doc_type: String,
    pub segment_size_known: bool,
    /// The Segment children: ID and byte offset.
    pub children: Vec<(u32, u64)>,
    pub blocks: Vec<Stored>,
    /// Each Cluster's offset and timestamp.
    pub clusters: Vec<(u64, u64)>,
}

pub const SEEK_HEAD: u32 = 0x114D_9B74;
pub const VOID: u32 = 0xEC;
pub const INFO: u32 = 0x1549_A966;
pub const TRACKS: u32 = 0x1654_AE6B;
pub const CLUSTER: u32 = 0x1F43_B675;
pub const CUES: u32 = 0x1C53_BB6B;
pub const CHAPTERS: u32 = 0x1043_A770;
pub const TAGS: u32 = 0x1254_C367;
pub const ATTACHMENTS: u32 = 0x1941_A469;

/// The element at `pos`: ID, body offset and body size (`None` when unknown).
fn element(bytes: &[u8], pos: usize) -> (u32, usize, Option<usize>) {
    let id_len = bytes[pos].leading_zeros() as usize + 1;
    let id = bytes[pos..pos + id_len]
        .iter()
        .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
    let (size, size_len) = vint(&bytes[pos + id_len..]);
    let unknown = size == (1u64 << (7 * size_len)) - 1;
    (
        id,
        pos + id_len + size_len,
        (!unknown).then_some(size as usize),
    )
}

fn vint(bytes: &[u8]) -> (u64, usize) {
    let len = bytes[0].leading_zeros() as usize + 1;
    let mask = (0xFFu32 >> len) as u8;
    let value = bytes[1..len]
        .iter()
        .fold(u64::from(bytes[0] & mask), |acc, &b| {
            (acc << 8) | u64::from(b)
        });
    (value, len)
}

fn uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |acc, &b| (acc << 8) | u64::from(b))
}

fn block_fields(bytes: &[u8]) -> (u64, i16, u8) {
    let (track, len) = vint(bytes);
    (
        track,
        i16::from_be_bytes([bytes[len], bytes[len + 1]]),
        bytes[len + 2],
    )
}

fn walk_cluster(bytes: &[u8], base: usize, end: usize, out: &mut Layout, offset: u64) {
    let mut timestamp = 0;
    let mut pos = base;
    let mut seen_timestamp = false;
    while pos < end {
        let (id, body, size) = element(bytes, pos);
        let size = size.expect("a child of a Cluster has a known size");
        let content = &bytes[body..body + size];
        match id {
            0xE7 => {
                timestamp = uint(content);
                seen_timestamp = true;
                out.clusters.push((offset, timestamp));
            }
            0xA3 => {
                assert!(seen_timestamp, "the Timestamp precedes each block");
                let (track, relative, flags) = block_fields(content);
                out.blocks.push(Stored {
                    track,
                    cluster_timestamp: timestamp,
                    relative,
                    flags,
                    simple: true,
                    duration: None,
                    has_reference: false,
                    padding: false,
                    at: (pos - base) as u64,
                });
            }
            0xA0 => {
                assert!(seen_timestamp, "the Timestamp precedes each block");
                let mut block = None;
                let mut duration = None;
                let mut has_reference = false;
                let mut padding = false;
                let mut child = body;
                while child < body + size {
                    let (kind, at, len) = element(bytes, child);
                    let len = len.expect("a child of a BlockGroup has a known size");
                    match kind {
                        0xA1 => block = Some(block_fields(&bytes[at..at + len])),
                        0x9B => duration = Some(uint(&bytes[at..at + len])),
                        0xFB => has_reference = true,
                        0x75A2 => padding = true,
                        _ => {}
                    }
                    child = at + len;
                }
                let (track, relative, flags) = block.expect("a BlockGroup holds a Block");
                out.blocks.push(Stored {
                    track,
                    cluster_timestamp: timestamp,
                    relative,
                    flags,
                    simple: false,
                    duration,
                    has_reference,
                    padding,
                    at: (pos - base) as u64,
                });
            }
            other => panic!("element {other:#X} in a Cluster"),
        }
        pos = body + size;
    }
}

/// Walk a file the muxer wrote.
pub fn walk(bytes: &[u8]) -> Layout {
    let mut out = Layout::default();
    let (id, body, size) = element(bytes, 0);
    assert_eq!(id, 0x1A45_DFA3, "the file starts with the EBML header");
    let header_size = size.expect("the EBML header has a known size");
    let mut child = body;
    while child < body + header_size {
        let (kind, at, len) = element(bytes, child);
        let len = len.expect("a header child has a known size");
        if kind == 0x4282 {
            out.doc_type = String::from_utf8_lossy(&bytes[at..at + len]).into_owned();
        }
        child = at + len;
    }
    let (id, segment, size) = element(bytes, body + header_size);
    assert_eq!(id, 0x1853_8067, "a Segment follows the EBML header");
    out.segment_size_known = size.is_some();
    if let Some(size) = size {
        assert_eq!(
            segment + size,
            bytes.len(),
            "the Segment size covers the file"
        );
    }
    let mut pos = segment;
    while pos < bytes.len() {
        let (id, at, len) = element(bytes, pos);
        let len = len.expect("a Segment child has a known size");
        assert!(at + len <= bytes.len(), "element {id:#X} fits the file");
        out.children.push((id, pos as u64));
        if id == CLUSTER {
            walk_cluster(bytes, at, at + len, &mut out, pos as u64);
        }
        pos = at + len;
    }
    out
}

impl Layout {
    /// The Segment child IDs with Clusters merged and `Void` dropped.
    pub fn order(&self) -> Vec<u32> {
        let mut order: Vec<u32> = Vec::new();
        for &(id, _) in &self.children {
            if id != VOID && !(id == CLUSTER && order.last() == Some(&CLUSTER)) {
                order.push(id);
            }
        }
        order
    }

    /// The last block of `track` in file order.
    pub fn last_block(&self, track: u64) -> Option<&Stored> {
        self.blocks.iter().rev().find(|block| block.track == track)
    }
}
