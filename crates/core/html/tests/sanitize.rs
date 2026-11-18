//! The sanitiser's own tests: the ones that need no editor schema, moved here from `thrust-editor-doc` with the code (issue #48).
//! The tests that parse the sanitised HTML with the editor's schema, or compare it with nh3 through the real JS conversion, stay in
//! `crates/editor/doc/tests/sanitize.rs`.
use thrust_html as sanitize;

/// Plane's `validate_html_content` and `validate_binary_data`, as the pin's `content_validator.py` states them (the example is the
/// spec's own: empty is `None`, over 10 MB is refused, a script in the first 200 bytes of a binary is refused).
#[test]
fn validate_html_content_semantics() {
    assert_eq!(sanitize::validate_html_content(""), Ok(None));
    let big = "a".repeat(sanitize::MAX_SIZE + 1);
    assert_eq!(sanitize::validate_html_content(&big), Err(sanitize::SanitizeError::TooLarge));
    assert!(sanitize::validate_html_content("<p>x</p>").unwrap().unwrap().contains("<p>x</p>"));
    assert!(sanitize::validate_binary_data(b"<script>alert(1)</script> padding padding").is_err());
    assert!(sanitize::validate_binary_data(&[1, 2]).is_err());
    assert!(sanitize::validate_binary_data(&[1, 2, 3, 4, 5]).is_ok());
    assert!(sanitize::validate_binary_data(&[]).is_ok());
}

/// Plane's limits are exact: `MAX_SIZE` is 10 MB, a payload of exactly that size is accepted and one byte more is refused, and a
/// binary of four bytes is long enough while three are not (`content_validator.py`, found by mutation testing: the tests above were
/// written relative to the constant, so a changed constant or a limit moved by one passed them).
#[test]
fn the_size_limits_are_plane_s_to_the_byte() {
    assert_eq!(sanitize::MAX_SIZE, 10 * 1024 * 1024);
    assert!(sanitize::validate_html_content(&"a".repeat(sanitize::MAX_SIZE)).unwrap().is_some());
    assert_eq!(
        sanitize::validate_html_content(&"a".repeat(sanitize::MAX_SIZE + 1)),
        Err(sanitize::SanitizeError::TooLarge)
    );
    assert!(sanitize::validate_binary_data(&vec![1u8; sanitize::MAX_SIZE]).is_ok());
    assert!(sanitize::validate_binary_data(&vec![1u8; sanitize::MAX_SIZE + 1]).is_err());
    assert!(sanitize::validate_binary_data(&[1, 2, 3, 4]).is_ok());
    assert!(sanitize::validate_binary_data(&[1, 2, 3]).is_err());
}

/// must-fail control for the harness itself: an unsanitised payload is not allowed to survive the check
#[test]
fn unsanitised_payload_is_caught() {
    let s = sanitize::Sanitizer::default();
    for p in ["<img src=x onerror=alert(1)>", "<script>alert(1)</script>", "<a href=\"javascript:alert(1)\">x</a>"] {
        let out = s.clean(p);
        assert!(
            !out.contains("onerror") && !out.contains("<script") && !out.contains("javascript:"),
            "sanitiser let through: {out}"
        );
        assert!(
            p.contains("onerror") || p.contains("<script") || p.contains("javascript:"),
            "control payload must be dangerous as given"
        );
    }
}
