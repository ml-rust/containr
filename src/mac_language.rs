//! ISO 639-2 language codes: QuickTime Macintosh codes, and the two written forms.
//!
//! A QuickTime `mdhd` language below `0x400` is an index into this table. Two
//! codes can share a language, such as Traditional and Simplified Chinese. A
//! code with no entry has no ISO 639-2 form here, and the caller writes `und`.

/// The codes from 0 to 94.
const LOW: [&str; 95] = [
    "eng", "fra", "deu", "ita", "nld", "swe", "spa", "dan", "por", "nor", // 0
    "heb", "jpn", "ara", "fin", "ell", "isl", "mlt", "tur", "hrv", "zho", // 10
    "urd", "hin", "tha", "kor", "lit", "pol", "hun", "est", "lav", "smi", // 20
    "fao", "fas", "rus", "zho", "nld", "gle", "sqi", "ron", "ces", "slk", // 30
    "slv", "yid", "srp", "mkd", "bul", "ukr", "bel", "uzb", "kaz", "aze", // 40
    "aze", "hye", "kat", "ron", "kir", "tgk", "tuk", "mon", "mon", "pus", // 50
    "kur", "kas", "snd", "bod", "nep", "san", "mar", "ben", "asm", "guj", // 60
    "pan", "ori", "mal", "kan", "tam", "tel", "sin", "mya", "khm", "lao", // 70
    "vie", "ind", "tgl", "msa", "msa", "amh", "tir", "orm", "som", "swa", // 80
    "kin", "run", "nya", "mlg", "epo", // 90
];

/// The first code of [`HIGH`].
const HIGH_START: u16 = 128;

/// The codes from 128 to 138.
const HIGH: [&str; 11] = [
    "cym", "eus", "cat", "lat", "que", "grn", "aym", "tat", "uig", "dzo", "jav",
];

/// The ISO 639-2/T code of a Macintosh language code. `None` when the table
/// has no entry.
pub(crate) fn macintosh_iso639(code: u16) -> Option<&'static str> {
    let (table, index): (&[&'static str], u16) = if code < HIGH_START {
        (&LOW, code)
    } else {
        (&HIGH, code - HIGH_START)
    };
    table.get(usize::from(index)).copied()
}

/// The ISO 639-2 codes whose terminology form differs from the bibliographic
/// form, as `(terminology, bibliographic)`.
const TERMINOLOGY_BIBLIOGRAPHIC: [(&str, &str); 20] = [
    ("bod", "tib"),
    ("ces", "cze"),
    ("cym", "wel"),
    ("deu", "ger"),
    ("ell", "gre"),
    ("eus", "baq"),
    ("fas", "per"),
    ("fra", "fre"),
    ("hye", "arm"),
    ("isl", "ice"),
    ("kat", "geo"),
    ("mkd", "mac"),
    ("mri", "mao"),
    ("msa", "may"),
    ("mya", "bur"),
    ("nld", "dut"),
    ("ron", "rum"),
    ("slk", "slo"),
    ("sqi", "alb"),
    ("zho", "chi"),
];

/// `code` in the bibliographic form Matroska readers before version 4 expect.
/// A code with one form comes back unchanged.
#[cfg(feature = "mkv")]
pub(crate) fn bibliographic(code: &str) -> &str {
    TERMINOLOGY_BIBLIOGRAPHIC
        .iter()
        .find_map(|(terminology, bibliographic)| (*terminology == code).then_some(*bibliographic))
        .unwrap_or(code)
}

/// `code` in the terminology form the MP4 `mdhd` box stores. A code with one
/// form comes back unchanged.
#[cfg(feature = "mp4")]
pub(crate) fn terminology(code: &str) -> &str {
    TERMINOLOGY_BIBLIOGRAPHIC
        .iter()
        .find_map(|(terminology, bibliographic)| (*bibliographic == code).then_some(*terminology))
        .unwrap_or(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_map() {
        for (code, want) in [
            (0, "eng"),
            (1, "fra"),
            (2, "deu"),
            (5, "swe"),
            (11, "jpn"),
            (19, "zho"),
            (33, "zho"),
            (34, "nld"),
            (53, "ron"),
            (58, "mon"),
            (94, "epo"),
            (128, "cym"),
            (138, "jav"),
        ] {
            assert_eq!(macintosh_iso639(code), Some(want), "code {code}");
        }
    }

    #[test]
    fn codes_with_no_entry_give_none() {
        for code in [95, 127, 139, 0x3FF, 0x7FFF] {
            assert_eq!(macintosh_iso639(code), None, "code {code}");
        }
    }

    #[test]
    fn every_entry_is_three_lowercase_letters() {
        for entry in LOW.iter().chain(&HIGH) {
            assert_eq!(entry.len(), 3, "{entry}");
            assert!(
                entry.bytes().all(|byte| byte.is_ascii_lowercase()),
                "{entry}"
            );
        }
    }

    #[cfg(all(feature = "mkv", feature = "mp4"))]
    #[test]
    fn the_two_forms_convert_both_ways() {
        for (terminology_form, bibliographic_form) in TERMINOLOGY_BIBLIOGRAPHIC {
            assert_eq!(bibliographic(terminology_form), bibliographic_form);
            assert_eq!(terminology(bibliographic_form), terminology_form);
        }
        assert_eq!(bibliographic("eng"), "eng");
        assert_eq!(terminology("eng"), "eng");
    }
}
