//! The generated list must satisfy what `is_reserved_slug` assumes of it.

use crate::reserved_slugs::RESERVED_WORKSPACE_SLUGS;

#[test]
fn the_reserved_slug_list_is_sorted_and_has_no_duplicates() {
    // `binary_search` is only right on a list that is sorted and strictly increasing
    assert!(RESERVED_WORKSPACE_SLUGS.windows(2).all(|w| w[0] < w[1]), "not strictly increasing");
}

#[test]
fn every_reserved_slug_is_reserved_and_a_near_miss_is_not() {
    for slug in RESERVED_WORKSPACE_SLUGS {
        assert!(crate::is_reserved_slug(slug), "{slug}");
        assert!(!crate::is_reserved_slug(&format!("{slug}x")), "{slug}x");
        assert!(!crate::is_reserved_slug(&format!("x{slug}")), "x{slug}");
        // matching is case sensitive, as the pin's is
        let upper = slug.to_uppercase();
        assert_eq!(crate::is_reserved_slug(&upper), upper == *slug, "{upper}");
    }
    assert!(!crate::is_reserved_slug(""));
}

#[test]
fn every_reserved_slug_is_itself_a_valid_slug() {
    // a reserved slug is refused for being reserved, not for its characters: the two refusals have different messages
    for slug in RESERVED_WORKSPACE_SLUGS {
        assert!(crate::slug_has_valid_characters(slug), "{slug}");
    }
}
