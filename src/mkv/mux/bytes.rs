//! Errors and derived UIDs the muxer shares.

use crate::error::{Container, Error};

/// An error for an invalid muxer call, option, spec or item.
pub(crate) fn invalid(reason: String) -> Error {
    Error::InvalidMux {
        container: Container::Matroska,
        reason,
    }
}

/// A refusal of what Matroska cannot hold and MP4 can.
pub(crate) fn refuse(what: String) -> Error {
    Error::Unsupported {
        container: Container::Matroska,
        what,
        use_instead: Container::Mp4,
    }
}

/// A non-zero UID from `salt` and `n` (FNV-1a), moved past any value `taken`
/// reports. The same inputs give the same UID, so output bytes repeat.
pub(crate) fn derive_uid(salt: &[u8], n: u64, taken: impl Fn(u64) -> bool) -> u64 {
    const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let index = n.to_be_bytes();
    let mut hash = OFFSET;
    for &byte in salt.iter().chain(index.iter()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    while hash == 0 || taken(hash) {
        hash = hash.wrapping_add(1);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_uids_repeat_and_avoid_taken_values() {
        let first = derive_uid(b"track", 1, |_| false);
        assert_ne!(first, 0);
        assert_eq!(first, derive_uid(b"track", 1, |_| false));
        assert_eq!(derive_uid(b"track", 1, |uid| uid == first), first + 1);
    }
}
