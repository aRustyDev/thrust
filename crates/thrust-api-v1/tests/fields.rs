//! DRF's field validation as the v1 routes use it (issue #58): what each kind accepts, what it refuses, and the message it refuses with.
//!
//! The kinds are DRF's `CharField`, `BooleanField` and `IntegerField` as the pinned serializers build them from the model columns; the
//! spellings and messages are read from DRF's source. Each table is the whole of a finite domain (every spelling DRF accepts as a
//! boolean; the boundaries of a bound and one step either side of each), so a changed comparison fails one of its rows.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use rstest::rstest;
use serde_json::{json, Value};
use thrust_api_v1::fields::{bool_field, char_field, int_field, no_such_pk, uuid_field};
use thrust_contracts::Uuid;

// --- BooleanField -------------------------------------------------------------------------------------------------------------

/// Every spelling DRF's `BooleanField` takes for true and for false (`TRUE_VALUES` and `FALSE_VALUES`), and the numbers 1 and 0 as
/// the JSON parser gives them (an integer, or a float with no fraction).
#[test]
fn every_spelling_drf_takes_is_read_as_the_boolean_it_means() {
    let truthy = ["t", "T", "y", "Y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON", "1"];
    let falsy = ["f", "F", "n", "N", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF", "0"];
    for s in truthy {
        assert_eq!(bool_field(&json!(s)), Ok(true), "{s}");
    }
    for s in falsy {
        assert_eq!(bool_field(&json!(s)), Ok(false), "{s}");
    }
    for v in [json!(true), json!(1), json!(1.0)] {
        assert_eq!(bool_field(&v), Ok(true), "{v}");
    }
    for v in [json!(false), json!(0), json!(0.0)] {
        assert_eq!(bool_field(&v), Ok(false), "{v}");
    }
}

/// Anything else is refused, with the message the pin gives (which does not quote the value), and null has its own.
#[rstest]
#[case(json!("maybe"))]
#[case(json!(""))]
#[case(json!("TRUe"))]
#[case(json!(" true"))]
#[case(json!(2))]
#[case(json!(-1))]
#[case(json!(0.5))]
#[case(json!([]))]
#[case(json!({}))]
#[case(json!([true]))]
fn anything_else_is_not_a_boolean(#[case] v: Value) {
    assert_eq!(bool_field(&v), Err("Must be a valid boolean.".to_string()), "{v}");
}

#[test]
fn a_null_is_not_a_boolean_and_says_so_in_its_own_words() {
    assert_eq!(bool_field(&Value::Null), Err("This field may not be null.".to_string()));
}

// --- IntegerField -------------------------------------------------------------------------------------------------------------

const REQUIRED: &str = "A valid integer is required.";

/// A number as `archive_in` and `close_in` take it (0 to 12): the bounds themselves, and a step either side of each; the integers a
/// client writes as text (spaces, a sign, a trailing `.0` of any length) and as a float with no fraction.
#[rstest]
#[case::the_least(json!(0), Ok(0))]
#[case::the_most(json!(12), Ok(12))]
#[case::in_between(json!(6), Ok(6))]
#[case::above_the_most(json!(13), Err("Ensure this value is less than or equal to 12.".to_string()))]
#[case::far_above(json!(10_000_000_000_i64), Err("Ensure this value is less than or equal to 12.".to_string()))]
#[case::below_the_least(json!(-1), Err("Ensure this value is greater than or equal to 0.".to_string()))]
#[case::text(json!("5"), Ok(5))]
#[case::text_with_spaces(json!(" 5 "), Ok(5))]
#[case::text_with_a_plus(json!("+5"), Ok(5))]
#[case::text_with_a_point_zero(json!("5.0"), Ok(5))]
#[case::text_with_point_zeros(json!("5.000"), Ok(5))]
#[case::text_with_a_fraction(json!("5.01"), Err(REQUIRED.to_string()))]
#[case::text_with_a_half(json!("5.5"), Err(REQUIRED.to_string()))]
#[case::text_with_a_trailing_point(json!("5."), Ok(5))]
#[case::text_above_the_most(json!("13"), Err("Ensure this value is less than or equal to 12.".to_string()))]
#[case::text_below_the_least(json!("-1"), Err("Ensure this value is greater than or equal to 0.".to_string()))]
#[case::a_float_without_a_fraction(json!(5.0), Ok(5))]
#[case::a_float_with_a_fraction(json!(5.5), Err(REQUIRED.to_string()))]
#[case::a_float_too_big_to_be_exact(json!(1e16), Err(REQUIRED.to_string()))]
#[case::a_float_far_too_big(json!(1e20), Err(REQUIRED.to_string()))]
#[case::words(json!("abc"), Err(REQUIRED.to_string()))]
#[case::empty(json!(""), Err(REQUIRED.to_string()))]
#[case::a_bool(json!(true), Err(REQUIRED.to_string()))]
#[case::a_list(json!([5]), Err(REQUIRED.to_string()))]
#[case::an_object(json!({"a": 5}), Err(REQUIRED.to_string()))]
#[case::null(Value::Null, Err("This field may not be null.".to_string()))]
fn an_integer_is_read_as_drf_reads_it(#[case] v: Value, #[case] want: Result<i64, String>) {
    assert_eq!(int_field(&v, 0, 12), want, "{v}");
}

/// The same bounds at another place: a lower bound that is not zero, so that neither bound is read as the other.
#[rstest]
#[case(-5, Ok(-5))]
#[case(5, Ok(5))]
#[case(-6, Err("Ensure this value is greater than or equal to -5.".to_string()))]
#[case(6, Err("Ensure this value is less than or equal to 5.".to_string()))]
fn bounds_are_each_their_own(#[case] n: i64, #[case] want: Result<i64, String>) {
    assert_eq!(int_field(&json!(n), -5, 5), want);
}

// --- CharField ----------------------------------------------------------------------------------------------------------------

/// A name as `name` takes it: trimmed, counted in characters (not bytes) after trimming, a number is its text, and null, blank,
/// and what is no text each have a message. The length bound is the bound: a name of exactly that many is allowed.
#[rstest]
#[case::exactly_the_most(json!("abc"), Ok(Some("abc".to_string())))]
#[case::one_too_many(json!("abcd"), Err("Ensure this field has no more than 3 characters.".to_string()))]
#[case::characters_not_bytes(json!("ééé"), Ok(Some("ééé".to_string())))]
#[case::trimmed_first(json!("  abc  "), Ok(Some("abc".to_string())))]
#[case::a_number_is_its_text(json!(123), Ok(Some("123".to_string())))]
#[case::a_float_is_its_text(json!(1.5), Ok(Some("1.5".to_string())))]
#[case::blank(json!("   "), Err("This field may not be blank.".to_string()))]
#[case::empty(json!(""), Err("This field may not be blank.".to_string()))]
#[case::null(Value::Null, Err("This field may not be null.".to_string()))]
#[case::a_bool(json!(true), Err("Not a valid string.".to_string()))]
#[case::a_list(json!(["a"]), Err("Not a valid string.".to_string()))]
#[case::an_object(json!({"a": "b"}), Err("Not a valid string.".to_string()))]
fn a_name_is_read_as_drf_reads_it(#[case] v: Value, #[case] want: Result<Option<String>, String>) {
    assert_eq!(char_field(&v, Some(3), false, false), want, "{v}");
}

/// A column that allows blank and null: a blank is kept as it is (trimmed), a null is none, and the bound still holds.
#[test]
fn a_column_that_allows_blank_and_null_keeps_both() {
    assert_eq!(char_field(&json!("  "), Some(3), true, true), Ok(Some(String::new())));
    assert_eq!(char_field(&Value::Null, Some(3), true, true), Ok(None));
    assert_eq!(
        char_field(&json!("abcd"), Some(3), true, true),
        Err("Ensure this field has no more than 3 characters.".to_string())
    );
    // and with no bound at all there is none
    assert_eq!(
        char_field(&json!("a".repeat(100_000)), None, true, true).map(|t| t.map(|s| s.len())),
        Ok(Some(100_000))
    );
}

// --- the key of a row ---------------------------------------------------------------------------------------------------------

#[test]
fn a_key_is_a_uuid_in_any_hyphenation_or_null() {
    let id = Uuid::from_u128(0xA1B2_C3D4_E5F6_0718_293A_4B5C_6D7E_8F90);
    for spelled in [
        id.hyphenated().to_string(),
        id.simple().to_string(),
        id.braced().to_string(),
        id.urn().to_string(),
        id.hyphenated().to_string().to_uppercase(),
        format!("uuid:{}", id.simple()),
    ] {
        assert_eq!(uuid_field(&json!(spelled)), Ok(Some(id)), "{spelled}");
    }
    assert_eq!(uuid_field(&Value::Null), Ok(None));
    for bad in [json!("nope"), json!(""), json!(5), json!(true), json!([]), json!("1234")] {
        assert_eq!(uuid_field(&bad), Err("Must be a valid UUID.".to_string()), "{bad}");
    }
}

#[test]
fn a_key_that_names_no_row_is_told_in_the_pins_words() {
    let id = Uuid::from_u128(0xBAD);
    assert_eq!(no_such_pk(id), format!("Invalid pk \"{id}\" - object does not exist."));
}
