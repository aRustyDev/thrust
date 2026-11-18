//! `describe`, the one function a work item's description goes through before it is stored (issue #48, D-020): what is kept, the text
//! derived from it, and the policy it was cleaned under.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]
use proptest::prelude::*;
use thrust_html::{check_size, describe, strip_tags, validate_html_content, SanitizeError, MAX_SIZE, SCHEMA_VERSION};

/// Rule 9 (b): what the pin does is named by `research/0008` (§2.1, §5): script and handlers go, the text is what `strip_tags` makes of
/// the cleaned HTML (an entity stays as text), and an empty description stays empty, its text empty with it.
#[test]
fn a_description_is_cleaned_and_its_text_is_taken_from_the_cleaned_form() {
    let d = describe("<p>Fish &amp; <b>chips</b></p><script>steal()</script><img src=x onerror=steal()>").unwrap();
    assert_eq!(d.html, "<p>Fish &amp; <b>chips</b></p><img src=\"x\">");
    assert_eq!(d.stripped, "Fish &amp; chips");
    assert_eq!(d.schema, SCHEMA_VERSION);

    let empty = describe("").unwrap();
    assert_eq!((empty.html.as_str(), empty.stripped.as_str(), empty.schema), ("", "", SCHEMA_VERSION));

    // the pin's own default for a work item that was sent none
    let default = describe("<p></p>").unwrap();
    assert_eq!((default.html.as_str(), default.stripped.as_str()), ("<p></p>", ""));
}

/// The size is Plane's, to the byte, and `check_size` (what the request layers ask first) and `describe` agree on it.
#[test]
fn the_size_the_pin_refuses_is_refused_by_both() {
    let exactly = "a".repeat(MAX_SIZE);
    let over = "a".repeat(MAX_SIZE + 1);
    assert_eq!(check_size(&exactly), Ok(()));
    assert_eq!(check_size(&over), Err(SanitizeError::TooLarge));
    assert!(describe(&exactly).is_ok());
    assert_eq!(describe(&over), Err(SanitizeError::TooLarge));
}

proptest! {
    /// Whatever it is sent, what `describe` keeps is what `validate_html_content` makes of it (the empty text as it came), the text is
    /// `strip_tags` of that, and the policy is the current one: the three are derived from the one cleaned form, not from the input.
    #[test]
    fn prop_describe_is_the_cleaned_form_its_text_and_the_policy(sent in "(<[a-z]{1,6}( [a-z]{1,5}=\"[a-z&;]{0,6}\")?>|</[a-z]{1,6}>|[a-z &;<>\"']{0,8}){0,12}") {
        let d = describe(&sent).unwrap();
        let cleaned = validate_html_content(&sent).unwrap().unwrap_or_default();
        prop_assert_eq!(&d.html, &cleaned);
        prop_assert_eq!(&d.stripped, &strip_tags(&cleaned));
        prop_assert_eq!(d.schema, SCHEMA_VERSION);
    }

    /// A paragraph of plain text comes back as that text: the one thing a search over `description_stripped` must be able to rely on.
    #[test]
    fn prop_the_text_of_a_plain_paragraph_is_its_text(text in "[a-zA-Z0-9 ,.!?-]{0,40}") {
        let d = describe(&format!("<p>{text}</p>")).unwrap();
        prop_assert_eq!(d.stripped, text);
    }

    /// Markup in the text does not reach it: wrapping a paragraph in any number of the allowed inline tags leaves the same text.
    #[test]
    fn prop_inline_tags_do_not_change_the_text(text in "[a-zA-Z0-9 ]{1,30}", tags in prop::collection::vec(prop::sample::select(&["b", "i", "u", "em", "strong", "code"]), 0..5)) {
        let open: String = tags.iter().map(|t| format!("<{t}>")).collect();
        let close: String = tags.iter().rev().map(|t| format!("</{t}>")).collect();
        let d = describe(&format!("<p>{open}{text}{close}</p>")).unwrap();
        prop_assert_eq!(d.stripped, text);
    }
}
