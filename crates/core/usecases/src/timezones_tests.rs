//! The generated list must satisfy what `is_known_timezone` assumes of it.

use crate::timezones::COMMON_TIMEZONES;

#[test]
fn the_time_zone_list_is_sorted_and_has_no_duplicates() {
    // `binary_search` is only right on a list that is sorted and strictly increasing
    assert!(COMMON_TIMEZONES.windows(2).all(|w| w[0] < w[1]), "not strictly increasing");
}

#[test]
fn every_name_in_the_list_is_known_and_a_near_miss_is_not() {
    for tz in COMMON_TIMEZONES {
        assert!(crate::is_known_timezone(tz), "{tz}");
        // control: a name one character off, in case or by a suffix, is a different name
        assert!(!crate::is_known_timezone(&tz.to_lowercase()) || tz.to_lowercase() == *tz, "{tz} lower-cased");
        assert!(!crate::is_known_timezone(&format!("{tz}x")), "{tz}x");
    }
}
