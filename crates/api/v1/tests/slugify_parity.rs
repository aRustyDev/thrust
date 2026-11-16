//! `slugify` against the pinned Plane's, on every Unicode scalar value (issue #58, D-031 (2)).
//!
//! The fixture is recorded from the pin's own `slugify` (Django 5.2.17 on Python 3.12, `tools/compat-record/gen-slugify.py`), not
//! written by hand: what every one of the 1,112,064 scalar values does alone and between two letters, and the slugs of 3,000 names
//! drawn from a fixed seed. A character that Django folds under NFKD (`é`, `²`, `ﬃ`, a no-break space) and one that nothing folds
//! (`ß`, `Ø`, CJK) are both in it, so a fold that is missing, is wrong, or folds something the pin does not, fails a row.
//!
//! There is no difference to allow. The crate's Unicode data (18.0.0) is newer than the pin's (15.0.0), so the crate would fold
//! characters that the pin's Python has no character for, and so drops; `NEWER_THAN_THE_PINS_UNICODE` drops them here first (owner,
//! 2026-11-11: match the pin), and every row below is exact. `unassigned` in the fixture lists what the pin's data has no character
//! for, and the last test holds that list to exactly the characters the crate folds out of them: a crate that learns more fails it
//! and names them, and so does a pin that moves to a newer Python (the list is then emptied, D-031 (2)).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thrust_api_v1::{slugify, NEWER_THAN_THE_PINS_UNICODE};
use unicode_normalization::UnicodeNormalization;

const FIXTURE: &str = include_str!("parity/fixtures/slugify.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// A table of the fixture, keyed by the scalar value (the fixture writes it in hex, four digits or more).
fn table(fixture: &Value, key: &str) -> BTreeMap<u32, String> {
    fixture[key]
        .as_object()
        .unwrap()
        .iter()
        .map(|(hex, slug)| (u32::from_str_radix(hex, 16).unwrap(), slug.as_str().unwrap().to_string()))
        .collect()
}

/// What the two Unicode data sets are, for a failure to say.
fn versions(fixture: &Value) -> String {
    let (major, minor, update) = unicode_normalization::UNICODE_VERSION;
    format!(
        "the crate folds with Unicode {major}.{minor}.{update}, the pin with {}",
        fixture["unicode_version"].as_str().unwrap()
    )
}

/// Every scalar value but the surrogates (which are not characters, and no `str` holds one).
fn scalars() -> impl Iterator<Item = char> {
    (0..=0x10_FFFF_u32).filter_map(char::from_u32)
}

/// The runs of scalar values that the pin's Unicode data has no character for.
fn unassigned(fixture: &Value) -> Vec<(u32, u32)> {
    let hex = |v: &Value| u32::from_str_radix(v.as_str().unwrap(), 16).unwrap();
    fixture["unassigned"].as_array().unwrap().iter().map(|run| (hex(&run[0]), hex(&run[1]))).collect()
}

/// Every Unicode scalar value, as a name of its own: the slug is the pin's, and is empty for all that the fixture does not list.
#[test]
fn parity_slugify_of_every_scalar_value_is_the_pins() {
    let fixture = fixture();
    let alone = table(&fixture, "alone");
    // the fixture is not vacuous: it holds the folds the register names, and a good many more
    assert_eq!(alone.get(&0xE9).map(String::as_str), Some("e"), "é");
    assert_eq!(alone.get(&0xB3).map(String::as_str), Some("3"), "³ (EX-014's evidence)");
    assert_eq!(alone.get(&0xFB03).map(String::as_str), Some("ffi"), "the ffi ligature");
    assert!(alone.len() > 1_000, "{} entries", alone.len());

    let mut count = 0_u64;
    let mut wrong = Vec::new();
    for c in scalars() {
        count += 1;
        let want = alone.get(&u32::from(c)).map_or("", String::as_str);
        let got = slugify(&c.to_string());
        if got != want {
            wrong.push(format!("U+{:04X} gives {got:?}, the pin gives {want:?}", u32::from(c)));
        }
    }
    assert_eq!(
        count,
        fixture["scalar_values"].as_u64().unwrap(),
        "the fixture and this loop cover the same scalar values"
    );
    assert!(
        wrong.is_empty(),
        "{} of {count} differ ({}); the first twenty: {:#?}",
        wrong.len(),
        versions(&fixture),
        &wrong[..wrong.len().min(20)]
    );
}

/// Every scalar value between two letters: what a character does to its neighbours (a folded space or hyphen separates them, a
/// dropped character does not, a folded letter joins them).
#[test]
fn parity_slugify_of_every_scalar_value_between_two_letters_is_the_pins() {
    let fixture = fixture();
    let between = table(&fixture, "between");
    // U+00A0 is a no-break space that NFKD folds to a space, which is a separator
    assert_eq!(between.get(&0xA0).map(String::as_str), Some("a-b"), "no-break space");
    assert_eq!(between.get(&0xFF0D).map(String::as_str), Some("a-b"), "fullwidth hyphen-minus");
    assert!(between.len() > 1_000, "{} entries", between.len());

    let mut wrong = Vec::new();
    for c in scalars() {
        let want = between.get(&u32::from(c)).map_or("ab", String::as_str);
        let got = slugify(&format!("a{c}b"));
        if got != want {
            wrong.push(format!("a + U+{:04X} + b gives {got:?}, the pin gives {want:?}", u32::from(c)));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} differ ({}); the first twenty: {:#?}",
        wrong.len(),
        versions(&fixture),
        &wrong[..wrong.len().min(20)]
    );
}

/// Names of up to twelve characters out of a pool of ASCII, separators, folds and what nothing folds: the runs, the ends and the
/// interplay of the three, which no single character shows.
#[test]
fn parity_slugify_of_drawn_names_is_the_pins() {
    let fixture = fixture();
    let names = fixture["names"].as_array().unwrap();
    assert_eq!(names.len(), 3_000);
    let mut wrong = Vec::new();
    for pair in names {
        let (name, want) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        let got = slugify(name);
        if got != want {
            wrong.push(format!("{name:?} gives {got:?}, the pin gives {want:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} differ ({}); the first twenty: {:#?}",
        wrong.len(),
        names.len(),
        versions(&fixture),
        &wrong[..wrong.len().min(20)]
    );
}

/// What `slugify` drops before it folds is exactly the scalar values that the pin has no character for and that the crate would fold
/// to ASCII: not one fewer (a difference from the pin, which the tests above would show too, but not name), and not one more (a
/// character left out of the fold that the pin may know).
#[test]
fn parity_slugify_drops_exactly_what_the_pin_lacks_and_the_crate_would_fold() {
    let fixture = fixture();
    let unknown = unassigned(&fixture);
    let would_fold: BTreeSet<char> = unknown
        .iter()
        .flat_map(|&(first, last)| first..=last)
        .filter_map(char::from_u32)
        .filter(|c| c.to_string().nfkd().any(|d| d.is_ascii()))
        .collect();
    let dropped: BTreeSet<char> = NEWER_THAN_THE_PINS_UNICODE.iter().flat_map(Clone::clone).collect();
    let hex = |set: Vec<&char>| set.iter().map(|c| format!("U+{:04X}", u32::from(**c))).collect::<Vec<_>>().join(" ");
    assert!(
        would_fold == dropped,
        "NEWER_THAN_THE_PINS_UNICODE ({}) must be what the crate folds and the pin lacks; to add: [{}]; to remove: [{}]",
        versions(&fixture),
        hex(would_fold.difference(&dropped).collect()),
        hex(dropped.difference(&would_fold).collect())
    );
}
