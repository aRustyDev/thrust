//! `x-thrust-version` (issue #49): what the header says, and when a response does not carry it.
//!
//! The deploy's smoke compares the header on `/healthz` with the commit it just deployed. That check is only evidence if a
//! Worker with no usable version sends NO header, never an empty or a made-up one, so the cases that matter most here are
//! the ones where nothing is sent.

mod common;

use axum::http::{HeaderValue, Response};
use common::proptest_config;
use proptest::prelude::*;
use rstest::rstest;
use thrust_api::{set_version, version_label, VERSION_HEADER};

// The classes a tag or an id can fall in. A tag is a commit sha (`wrangler deploy --tag`) and an id is the version id Cloudflare
// assigns; both are visible ASCII without spaces. The rest are what a bad deploy or a local `wrangler dev` could hand over.
const NOTHING: &str = "";
const BLANK: &str = "  \t\n";
const SHA: &str = "9f3c2ab41d0e77c5a8b6e1f2d3c4b5a697887766";
const PADDED_SHA: &str = " 9f3c2ab41d0e77c5a8b6e1f2d3c4b5a697887766\n";
const ID: &str = "0b1f6d3e-5c7a-4e2b-9a1d-8c3f2e1d0b9a";
const PADDED_ID: &str = "\t0b1f6d3e-5c7a-4e2b-9a1d-8c3f2e1d0b9a ";
const SPACED: &str = "not a sha";
const NON_ASCII: &str = "vérsion";

/// The spec, written as a table and not as the code under test: a usable tag wins whatever the id is; a tag that is absent or
/// blank leaves the id to decide; and everything else, including a tag that is there but cannot go in a header, is nothing. That
/// last arm is deliberate: falling through to the id would let a mis-tagged deploy show a version id where the smoke expects a
/// sha, and "absent" fails the smoke more plainly than "different".
fn spec(tag: &str, id: &str) -> Option<&'static str> {
    match (tag, id) {
        (SHA | PADDED_SHA, _) => Some(SHA),
        (NOTHING | BLANK, ID | PADDED_ID) => Some(ID),
        _ => None,
    }
}

// Exhaustive over the classes above: 6 tags x 6 ids.
#[rstest]
fn version_label_follows_the_table(
    #[values(NOTHING, BLANK, SHA, PADDED_SHA, SPACED, NON_ASCII)] tag: &str,
    #[values(NOTHING, BLANK, ID, PADDED_ID, SPACED, NON_ASCII)] id: &str,
) {
    assert_eq!(version_label(tag, id).as_deref(), spec(tag, id), "tag {tag:?}, id {id:?}");
}

#[rstest]
fn set_version_puts_the_label_on_the_response(
    #[values(SHA, PADDED_SHA)] tag: &str,
    #[values(NOTHING, ID, SPACED)] id: &str,
) {
    let mut resp = Response::new(());
    set_version(&mut resp, tag, id);
    assert_eq!(resp.headers().get(&VERSION_HEADER), Some(&HeaderValue::from_static(SHA)));
}

// The control the smoke leans on: no usable version means the header is NOT there, which is different from an empty value.
#[rstest]
fn set_version_sends_nothing_when_there_is_no_usable_version(
    #[values(NOTHING, BLANK, SPACED, NON_ASCII)] tag: &str,
    #[values(NOTHING, BLANK, SPACED, NON_ASCII)] id: &str,
) {
    let mut resp = Response::new(());
    set_version(&mut resp, tag, id);
    assert!(resp.headers().get(&VERSION_HEADER).is_none(), "tag {tag:?}, id {id:?}");
}

#[test]
fn set_version_replaces_a_header_that_is_already_there() {
    // pinned: a handler that sets its own `x-thrust-version` must not be able to make the Worker report a different build
    let mut resp = Response::new(());
    resp.headers_mut().insert(VERSION_HEADER, HeaderValue::from_static("from-a-handler"));
    set_version(&mut resp, SHA, ID);
    assert_eq!(resp.headers().get(&VERSION_HEADER), Some(&HeaderValue::from_static(SHA)));
}

/// Strings drawn from the classes that matter, so the generator reaches the `Some` arms often: a hex sha, a UUID-shaped id,
/// whitespace, strings with a space or a non-ASCII character, and anything at all.
fn label_like() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[0-9a-f]{7,40}",
        3 => "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
        2 => "[ \t\r\n]{0,4}",
        2 => "[ \t\r\n]{0,2}[0-9a-f]{1,12}[ \t\r\n]{0,2}",
        1 => "[a-z]{1,5} [a-z]{1,5}",
        1 => "[a-z]{0,5}[é€\u{7f}\u{0}][a-z]{0,5}",
        1 => any::<String>(),
    ]
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Whatever a deploy or a runtime hands over, a label that comes out is something a header can carry, and it is one of the
    /// two inputs, trimmed. Nothing is invented.
    #[test]
    fn prop_version_label_is_a_header_safe_copy_of_an_input(tag in label_like(), id in label_like()) {
        if let Some(label) = version_label(&tag, &id) {
            prop_assert!(!label.is_empty());
            prop_assert!(label.bytes().all(|b| b.is_ascii_graphic()), "not visible ASCII: {label:?}");
            prop_assert!(HeaderValue::from_str(&label).is_ok());
            prop_assert!(label == tag.trim() || label == id.trim(), "{label:?} is neither input");
        }
    }

    /// A usable tag always wins, and a blank tag with a usable id always gives that id: the two arms the smoke depends on.
    #[test]
    fn prop_version_label_prefers_a_usable_tag_over_the_id(tag in label_like(), id in label_like()) {
        let usable = |s: &str| !s.trim().is_empty() && s.trim().bytes().all(|b| b.is_ascii_graphic());
        let got = version_label(&tag, &id);
        if usable(&tag) {
            prop_assert_eq!(got.as_deref(), Some(tag.trim()));
        } else if tag.trim().is_empty() && usable(&id) {
            prop_assert_eq!(got.as_deref(), Some(id.trim()));
        } else {
            prop_assert_eq!(got, None);
        }
    }
}
