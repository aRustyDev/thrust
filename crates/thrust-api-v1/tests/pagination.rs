//! Plane's cursor pagination (issue #58), pinned by the oracle's corpus (`pagination.json`) and read from the pin's `paginator.py`.
//!
//! What must hold: `per_page` and `cursor` are read as Python reads them and refused with the pin's words; the cursors a page gives
//! lead to the page before and after it, so that following them visits every row once and in order; the envelope always has its
//! twelve keys; and nothing a client sends can make the reading panic (a `per_page` of 0 or less is the pin's 500, handled).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use axum::response::IntoResponse;
use common::proptest_config;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_api_v1::fields::py_int;
use thrust_api_v1::pagination::{PageRequest, MAX_PER_PAGE};
use thrust_api_v1::V1Error;

/// What a refusal says: its status and body.
fn answer(e: V1Error) -> (u16, Value) {
    let resp = e.into_response();
    let status = resp.status().as_u16();
    let bytes = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn refused(per_page: Option<&str>, cursor: Option<&str>) -> (u16, Value) {
    match PageRequest::parse(per_page, cursor).and_then(PageRequest::plan) {
        Ok(plan) => panic!("{per_page:?} {cursor:?} should be refused, and is the window {plan:?}"),
        Err(e) => answer(e),
    }
}

// --- reading `per_page` and `cursor` ------------------------------------------------------------------------------------------

/// The refusals the corpus records (`pagination.json`: per-page-over-max, per-page-not-a-number, cursor-garbage, and the two 500s),
/// and the negative page that the pin's paginator raises "Error in parsing" for.
#[rstest]
#[case::not_a_number(Some("abc"), None, 400, json!({"detail": "Invalid per_page parameter."}))]
#[case::empty(Some(""), None, 400, json!({"detail": "Invalid per_page parameter."}))]
#[case::a_fraction(Some("5.0"), None, 400, json!({"detail": "Invalid per_page parameter."}))]
#[case::over_the_most(Some("1001"), None, 400, json!({"detail": "Invalid per_page value. Cannot exceed 1000."}))]
#[case::garbage_cursor(None, Some("garbage"), 400, json!({"detail": "Invalid cursor parameter."}))]
#[case::two_parts(None, Some("1000:0"), 400, json!({"detail": "Invalid cursor parameter."}))]
#[case::four_parts(None, Some("1000:0:0:0"), 400, json!({"detail": "Invalid cursor parameter."}))]
#[case::empty_cursor(None, Some(""), 400, json!({"detail": "Invalid cursor parameter."}))]
#[case::page_not_a_number(None, Some("1000:x:0"), 400, json!({"detail": "Invalid cursor parameter."}))]
#[case::a_negative_page(Some("10"), Some("10:-1:0"), 400, json!({"detail": "Error in parsing"}))]
#[case::zero_per_page(Some("0"), None, 500, json!({"error": "Something went wrong please try again later"}))]
#[case::negative_per_page(Some("-1"), None, 500, json!({"error": "Something went wrong please try again later"}))]
#[case::a_cursor_back_with_another_size(Some("10"), Some("5:1:1"), 500, json!({"error": "Something went wrong please try again later"}))]
fn a_request_the_pin_refuses_is_refused_with_its_words(
    #[case] per_page: Option<&str>,
    #[case] cursor: Option<&str>,
    #[case] status: u16,
    #[case] body: Value,
) {
    assert_eq!(refused(per_page, cursor), (status, body));
}

/// The control for the table: the neighbours of each refusal that the pin accepts are accepted.
#[rstest]
#[case::the_default(None, None)]
#[case::spaces_and_a_plus(Some(" +7 "), None)]
#[case::an_underscore(Some("1_0"), None)]
#[case::the_most(Some("1000"), None)]
#[case::a_float_value(None, Some("1000.0:0:0"))]
#[case::a_later_page(Some("10"), Some("10:3:0"))]
#[case::back_with_the_same_size(Some("10"), Some("10:1:1"))]
fn what_the_pin_accepts_is_accepted(#[case] per_page: Option<&str>, #[case] cursor: Option<&str>) {
    assert!(PageRequest::parse(per_page, cursor).and_then(PageRequest::plan).is_ok());
}

#[rstest]
#[case("7", Some(7))]
#[case(" 7 ", Some(7))]
#[case("+7", Some(7))]
#[case("-7", Some(-7))]
#[case("1_000", Some(1000))]
#[case("1__0", None)]
#[case("_1", None)]
#[case("1_", None)]
#[case("", None)]
#[case("-", None)]
#[case("1e3", None)]
#[case("0x10", None)]
#[case("9223372036854775808", None)]
fn python_reads_an_integer_this_way(#[case] text: &str, #[case] want: Option<i64>) {
    assert_eq!(py_int(text), want);
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Every integer is read back as itself.
    #[test]
    fn prop_an_integer_is_read_as_itself(n in any::<i64>()) {
        prop_assert_eq!(py_int(&n.to_string()), Some(n));
    }

    /// Nothing a client sends in either parameter makes the reading panic: it is a window or a refusal (the 500 included, which is a
    /// handled answer).
    #[test]
    fn prop_no_input_makes_the_reading_panic(per_page in any::<String>(), cursor in any::<String>()) {
        let _ = PageRequest::parse(Some(&per_page), Some(&cursor)).and_then(PageRequest::plan);
        let _ = PageRequest::parse(Some(&per_page), None).and_then(PageRequest::plan);
        let _ = PageRequest::parse(None, Some(&cursor)).and_then(PageRequest::plan);
    }

    /// Numbers that look like a request, including the extremes, the same way.
    #[test]
    fn prop_no_number_makes_the_reading_panic(per_page in any::<i64>(), value in any::<i64>(), page in any::<i64>(), prev in 0u8..3) {
        let cursor = format!("{value}:{page}:{prev}");
        let _ = PageRequest::parse(Some(&per_page.to_string()), Some(&cursor)).and_then(PageRequest::plan);
    }
}

// --- the pages ----------------------------------------------------------------------------------------------------------------

const KEYS: [&str; 12] = [
    "count",
    "extra_stats",
    "grouped_by",
    "next_cursor",
    "next_page_results",
    "prev_cursor",
    "prev_page_results",
    "results",
    "sub_grouped_by",
    "total_count",
    "total_pages",
    "total_results",
];

/// A page of `0..n` as a client gets it, for the query it sent.
fn page_of(n: usize, per_page: Option<&str>, cursor: Option<&str>) -> Value {
    let plan = PageRequest::parse(per_page, cursor).unwrap().plan().unwrap();
    let rows = plan.slice((0..n).collect::<Vec<_>>());
    plan.envelope(rows.into_iter().map(|i| json!(i)).collect(), n)
}

/// The corpus's list of two projects (`projects.json#list`): one page of 1000, and the cursors it gives, which lead nowhere.
#[test]
fn a_list_that_fits_one_page_has_the_cursors_of_the_corpus() {
    let body = page_of(2, None, None);
    assert_eq!(
        body,
        json!({
            "count": 2, "extra_stats": null, "grouped_by": null, "sub_grouped_by": null,
            "next_cursor": "1000:1:0", "next_page_results": false,
            "prev_cursor": "1000:-1:1", "prev_page_results": false,
            "total_count": 2, "total_pages": 1, "total_results": 2,
            "results": [0, 1],
        })
    );
}

#[test]
fn an_empty_list_has_no_pages() {
    let body = page_of(0, None, None);
    assert_eq!(
        (body["total_pages"].clone(), body["count"].clone(), body["results"].clone()),
        (json!(0), json!(0), json!([]))
    );
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// Following `next_cursor` from the first page visits every row once and in order, ends exactly where `next_page_results` says,
    /// and every page is full but the last. The envelope has its twelve keys throughout, `total_pages` is the ceiling of the rows over
    /// the page size, and a page after the first says there is one before it.
    #[test]
    fn prop_following_the_cursors_visits_every_row_once_and_in_order(n in 0usize..250, per_page in 1i64..=40) {
        let size = per_page.to_string();
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let body = page_of(n, Some(&size), cursor.as_deref());
            let mut keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort_unstable();
            prop_assert_eq!(&keys, &KEYS.to_vec());
            prop_assert_eq!(body["total_results"].as_u64(), Some(n as u64));
            prop_assert_eq!(body["total_pages"].as_u64(), Some(n.div_ceil(per_page as usize) as u64));
            prop_assert_eq!(body["prev_page_results"].as_bool(), Some(pages > 0));
            let rows: Vec<usize> = body["results"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
            prop_assert_eq!(body["count"].as_u64(), Some(rows.len() as u64));
            seen.extend(rows);
            pages += 1;
            if !body["next_page_results"].as_bool().unwrap() {
                break;
            }
            cursor = Some(body["next_cursor"].as_str().unwrap().to_string());
            prop_assert!(pages <= n + 1, "the cursors never end");
        }
        prop_assert_eq!(seen, (0..n).collect::<Vec<_>>());
        prop_assert_eq!(pages, n.div_ceil(per_page as usize).max(1));
    }

    /// The previous cursor of a page leads to the page before it, with the same rows the pin gives.
    #[test]
    fn prop_the_previous_cursor_leads_back_one_page(n in 1usize..120, per_page in 1i64..=30, page in 1usize..6) {
        let size = per_page.to_string();
        let here = page_of(n, Some(&size), Some(&format!("{per_page}:{page}:0")));
        let back_cursor = here["prev_cursor"].as_str().unwrap();
        prop_assert_eq!(back_cursor, format!("{per_page}:{}:1", page - 1));
        let back = page_of(n, Some(&size), Some(back_cursor));
        let direct = page_of(n, Some(&size), Some(&format!("{per_page}:{}:0", page - 1)));
        prop_assert_eq!(&back["results"], &direct["results"]);
    }

    /// A `per_page` above the most is refused wherever it is, and a page size is never more than the most.
    #[test]
    fn prop_a_page_is_never_more_than_the_most(n in 0usize..30, per_page in 1i64..=MAX_PER_PAGE) {
        let body = page_of(n, Some(&per_page.to_string()), None);
        prop_assert!(body["count"].as_i64().unwrap() <= per_page.min(MAX_PER_PAGE));
    }
}
