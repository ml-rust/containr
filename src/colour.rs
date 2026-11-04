//! Colour tags a container carries next to the bitstream.

/// The colour tags of a video track, as the container stores them.
///
/// MP4 and MOV store them in a `colr` box of the visual sample entry.
/// Matroska stores them in `Video/Colour`. The numbers are the ITU-T H.273
/// code points, the same ones a codec writes in its bitstream. A field is
/// `None` when the container does not tag it. Value 2 (unspecified) stays
/// `Some(2)`, since the container did write it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContainerColour {
    /// H.273 `ColourPrimaries`.
    pub primaries: Option<u8>,
    /// H.273 `TransferCharacteristics`.
    pub transfer: Option<u8>,
    /// H.273 `MatrixCoefficients`.
    pub matrix: Option<u8>,
    /// True for full range, false for limited range. `None` when the
    /// container gives no range: a QuickTime `nclc` box, or a Matroska
    /// `Range` of 0 (unspecified) or 3 (defined by matrix and transfer).
    pub full_range: Option<bool>,
}

impl ContainerColour {
    /// True when no field is set.
    pub const fn is_empty(&self) -> bool {
        self.primaries.is_none()
            && self.transfer.is_none()
            && self.matrix.is_none()
            && self.full_range.is_none()
    }

    /// `None` when no field is set, else `self`.
    #[cfg_attr(not(feature = "mkv"), allow(dead_code))]
    pub(crate) const fn non_empty(self) -> Option<Self> {
        if self.is_empty() { None } else { Some(self) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty_and_filtered() {
        assert!(ContainerColour::default().is_empty());
        assert_eq!(ContainerColour::default().non_empty(), None);
        let tagged = ContainerColour {
            full_range: Some(false),
            ..ContainerColour::default()
        };
        assert!(!tagged.is_empty());
        assert_eq!(tagged.non_empty(), Some(tagged));
    }
}
