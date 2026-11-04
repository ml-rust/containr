//! The timestamp rule: exact conversion or refusal.
//!
//! A track's timestamps count in its [`Timebase`]. The output counts in
//! `TimestampScale`, a whole number of nanoseconds per tick. A value `v`
//! converts when `v * num * 10^9 / den` is a whole number of nanoseconds, and
//! that number is a whole multiple of the scale. Nothing rounds.
//!
//! Each failure has its own remedy:
//! - No whole number of nanoseconds: no `TimestampScale` can hold the value,
//!   such as 1024 ticks at 48 000 ticks per second. The mux refuses it, and
//!   the error names MP4.
//! - Not a multiple of the scale: a finer scale holds it, and the error names
//!   that scale.

use crate::track::Timebase;

/// Nanoseconds per second.
const NANOS: i128 = 1_000_000_000;

/// Why a value has no exact tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inexact {
    /// The value is no whole number of nanoseconds, or the timebase is zero.
    NoNanoseconds,
    /// The value is `nanos`, which the scale does not divide. `divisor`
    /// divides both.
    NotMultiple { nanos: i128, divisor: u64 },
}

/// The greatest common divisor.
pub(crate) fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `value` timebase ticks as whole nanoseconds. `value` spans 65 bits at most,
/// so the product of 127 bits at most fits.
pub(crate) fn nanos_of(value: i128, timebase: Timebase) -> Option<i128> {
    let den = i128::from(timebase.den);
    let total = value * i128::from(timebase.num) * NANOS;
    (den != 0 && total % den == 0).then(|| total / den)
}

/// `value` timebase ticks as output ticks of `scale` nanoseconds.
pub(crate) fn rescale(value: i128, timebase: Timebase, scale: u64) -> Result<i128, Inexact> {
    let nanos = nanos_of(value, timebase).ok_or(Inexact::NoNanoseconds)?;
    let wide = i128::from(scale.max(1));
    if nanos % wide == 0 {
        return Ok(nanos / wide);
    }
    let divisor = gcd(u128::from(scale), nanos.unsigned_abs());
    Err(Inexact::NotMultiple {
        nanos,
        divisor: u64::try_from(divisor).unwrap_or(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_convert() {
        let mp4 = Timebase::from_timescale(12_800);
        assert_eq!(rescale(512, mp4, 1_000_000), Ok(40));
        assert_eq!(rescale(-512, mp4, 1_000_000), Ok(-40));
        let mkv = Timebase::from_nanoseconds(1_000_000);
        assert_eq!(rescale(21, mkv, 500_000), Ok(42));
        assert_eq!(
            rescale(i128::from(u64::MAX), mkv, 1),
            Ok(i128::from(u64::MAX) * 1_000_000)
        );
    }

    #[test]
    fn aac_frames_at_48_khz_have_no_exact_tick_at_any_scale() {
        let audio = Timebase::from_timescale(48_000);
        for scale in [1, 1000, 1_000_000] {
            assert_eq!(rescale(1024, audio, scale), Err(Inexact::NoNanoseconds));
        }
        assert_eq!(rescale(960, audio, 1_000_000), Ok(20));
    }

    #[test]
    fn a_coarse_scale_names_the_divisor_that_holds_the_value() {
        let mkv = Timebase::from_nanoseconds(1_000_000);
        assert_eq!(
            rescale(21, mkv, 2_000_000),
            Err(Inexact::NotMultiple {
                nanos: 21_000_000,
                divisor: 1_000_000
            })
        );
    }

    #[test]
    fn a_zero_timebase_is_refused() {
        assert_eq!(
            rescale(1, Timebase { num: 1, den: 0 }, 1),
            Err(Inexact::NoNanoseconds)
        );
    }
}
