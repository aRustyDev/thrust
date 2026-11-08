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

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Map, Value};
use thrust_api_v1::fields::{
    bool_field, char_field, choice_field, float_field, int_field, name_was_padded, no_such_pk, no_such_pk_shown,
    not_a_dictionary, pk_field, read, uuid_field, FieldErrors, PkRead,
};
use thrust_api_v1::{slugify, V1Error};
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

// --- ChoiceField --------------------------------------------------------------------------------------------------------------

/// A choice is its text, and what is not one is refused with the text DRF quotes: Python's `str()` of the value (so a bool is
/// `True` and `False`, and a number is its digits). Null has its own message, and the corpus records the first (`"nope" is not a valid
/// choice.`).
#[rstest]
#[case(json!("backlog"), Ok("backlog"))]
#[case(json!("started"), Ok("started"))]
#[case(json!("nope"), Err("\"nope\" is not a valid choice.".to_string()))]
#[case(json!(""), Err("\"\" is not a valid choice.".to_string()))]
#[case(json!("Backlog"), Err("\"Backlog\" is not a valid choice.".to_string()))]
#[case(json!(true), Err("\"True\" is not a valid choice.".to_string()))]
#[case(json!(false), Err("\"False\" is not a valid choice.".to_string()))]
#[case(json!(5), Err("\"5\" is not a valid choice.".to_string()))]
#[case(json!(1.5), Err("\"1.5\" is not a valid choice.".to_string()))]
#[case(Value::Null, Err("This field may not be null.".to_string()))]
fn a_choice_is_one_of_the_choices(#[case] v: Value, #[case] want: Result<&str, String>) {
    assert_eq!(choice_field(&v, &["backlog", "started"]), want, "{v}");
}

/// A choice whose text is a number or a bool in Python is found by that text.
#[test]
fn a_choice_that_reads_as_a_python_value_is_found_by_its_text() {
    assert_eq!(choice_field(&json!(true), &["True"]), Ok("True"));
    assert_eq!(choice_field(&json!(false), &["False", "True"]), Ok("False"));
    assert_eq!(choice_field(&json!(7), &["7"]), Ok("7"));
}

// --- FloatField ---------------------------------------------------------------------------------------------------------------

const NUMBER: &str = "A valid number is required.";

/// A place as `sequence` takes it: Python's `float()`: a number, a string that reads as one (spaces and exponents included), a bool
/// as 0 or 1; anything else is no number, and so is a value that is not finite.
#[rstest]
#[case(json!(1), Ok(1.0))]
#[case(json!(90000.0), Ok(90000.0))]
#[case(json!(-1.5), Ok(-1.5))]
#[case(json!("2.5"), Ok(2.5))]
#[case(json!(" 2.5 "), Ok(2.5))]
#[case(json!("1e3"), Ok(1000.0))]
#[case(json!(true), Ok(1.0))]
#[case(json!(false), Ok(0.0))]
#[case(json!("abc"), Err(NUMBER.to_string()))]
#[case(json!(""), Err(NUMBER.to_string()))]
#[case(json!("nan"), Err(NUMBER.to_string()))]
#[case(json!("inf"), Err(NUMBER.to_string()))]
#[case(json!("-infinity"), Err(NUMBER.to_string()))]
#[case(json!([1]), Err(NUMBER.to_string()))]
#[case(json!({"a": 1}), Err(NUMBER.to_string()))]
#[case(Value::Null, Err("This field may not be null.".to_string()))]
fn a_number_is_read_as_python_reads_a_float(#[case] v: Value, #[case] want: Result<f64, String>) {
    assert_eq!(float_field(&v), want, "{v}");
}

// --- reading a field of a body ------------------------------------------------------------------------------------------------

#[test]
fn a_field_is_read_when_the_body_names_it_and_what_is_wrong_is_filed_under_its_name() {
    let body: Map<String, Value> = json!({"good": 5, "bad": "x"}).as_object().unwrap().clone();
    let mut errors = FieldErrors::new();
    let int = |v: &Value| int_field(v, 0, 12);
    assert_eq!(read(&mut errors, &body, "good", int), Some(5));
    assert!(errors.is_empty(), "a field that reads files nothing");
    assert_eq!(read(&mut errors, &body, "absent", int), None);
    assert!(errors.is_empty(), "a field the body does not name is not an error here");
    assert_eq!(read(&mut errors, &body, "bad", int), None);
    assert_eq!(Value::Object(errors), json!({"bad": ["A valid integer is required."]}));
}

#[test]
fn a_body_that_is_a_list_is_not_a_dictionary() {
    match not_a_dictionary() {
        V1Error::Fields(errors) => {
            assert_eq!(
                Value::Object(errors),
                json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]})
            );
        }
        other => panic!("{other:?}"),
    }
}

// --- a name that DRF trimmed --------------------------------------------------------------------------------------------------

/// A name is "padded" when the body's text differs from what the serializer kept, which is what the pin's clash lookup (by the text as
/// sent) cannot find. Only text can be padded: a number is kept as its own text, and a name that was not sent or was refused is not.
#[rstest]
#[case(Some(json!("Todo")), Some("Todo"), false)]
#[case(Some(json!(" Todo")), Some("Todo"), true)]
#[case(Some(json!("Todo ")), Some("Todo"), true)]
#[case(Some(json!("\tTodo\n")), Some("Todo"), true)]
#[case(Some(json!(" ")), Some(""), true)]
#[case(Some(json!("To do")), Some("To do"), false)]
#[case(Some(json!(5)), Some("5"), false)]
#[case(Some(json!("Todo")), None, false)]
#[case(None, Some("Todo"), false)]
#[case(None, None, false)]
fn a_name_is_padded_when_the_text_sent_is_not_the_text_kept(
    #[case] sent: Option<Value>,
    #[case] kept: Option<&str>,
    #[case] padded: bool,
) {
    let body: Map<String, Value> = sent.into_iter().map(|v| ("name".to_string(), v)).collect();
    assert_eq!(name_was_padded(&body, kept), padded);
}

// --- PrimaryKeyRelatedField ---------------------------------------------------------------------------------------------------

const KEY: &str = "5a1b3c4d-0000-4000-8000-000000000001";

/// What DRF's relation makes of each kind of value for a model keyed by a UUID: none, a key (shown as it was spelled), its own
/// refusal for a bool, and Django's `ValidationError` (which no field catches) for what the key column cannot read.
#[rstest]
#[case::null(json!(null), PkRead::Null)]
#[case::empty_text(json!(""), PkRead::Null)]
#[case::a_key(json!(KEY), PkRead::Key { id: KEY.parse().unwrap(), shown: KEY.into() })]
#[case::without_hyphens(json!("5a1b3c4d000040008000000000000001"), PkRead::Key { id: KEY.parse().unwrap(), shown: "5a1b3c4d000040008000000000000001".into() })]
#[case::in_braces(json!("{5A1B3C4D-0000-4000-8000-000000000001}"), PkRead::Key { id: KEY.parse().unwrap(), shown: "{5A1B3C4D-0000-4000-8000-000000000001}".into() })]
#[case::urn(json!("urn:uuid:5a1b3c4d-0000-4000-8000-000000000001"), PkRead::Key { id: KEY.parse().unwrap(), shown: "urn:uuid:5a1b3c4d-0000-4000-8000-000000000001".into() })]
#[case::an_integer(json!(7), PkRead::Key { id: Uuid::from_u128(7), shown: "7".into() })]
#[case::zero(json!(0), PkRead::Key { id: Uuid::nil(), shown: "0".into() })]
#[case::true_is_a_type_error(json!(true), PkRead::Refused("Incorrect type. Expected pk value, received bool.".into()))]
#[case::false_is_too(json!(false), PkRead::Refused("Incorrect type. Expected pk value, received bool.".into()))]
#[case::text_that_is_no_key(json!("nope"), PkRead::Raises)]
#[case::too_short(json!("5a1b3c4d"), PkRead::Raises)]
#[case::non_hex(json!("zzzzzzzz-0000-4000-8000-000000000001"), PkRead::Raises)]
#[case::a_negative_integer(json!(-1), PkRead::Raises)]
#[case::a_float(json!(1.5), PkRead::Raises)]
#[case::a_whole_float(json!(1.0), PkRead::Raises)]
#[case::a_list(json!([KEY]), PkRead::Raises)]
#[case::an_object(json!({"id": KEY}), PkRead::Raises)]
fn a_relation_reads_a_value_as_drfs_primary_key_field_does(#[case] v: Value, #[case] want: PkRead) {
    assert_eq!(pk_field(&v), want);
}

#[test]
fn a_key_that_names_no_row_is_quoted_as_it_was_spelled() {
    assert_eq!(no_such_pk_shown("{ABC}"), "Invalid pk \"{ABC}\" - object does not exist.");
    // the canonical form is the key's own text
    let id: Uuid = KEY.parse().unwrap();
    assert_eq!(no_such_pk(id), no_such_pk_shown(KEY));
}

// --- slugify ------------------------------------------------------------------------------------------------------------------

/// Django's `slugify` of what a state is called: the corpus records two (`Oracle Review` is `oracle-review`, and the renamed one
/// `oracle-review-2`). An accent is dropped, where Django folds it to its letter.
#[rstest]
#[case("Oracle Review", "oracle-review")]
#[case("Oracle Review 2", "oracle-review-2")]
#[case("In Progress", "in-progress")]
#[case("  Spaces   everywhere  ", "spaces-everywhere")]
#[case("a--b", "a-b")]
#[case("a - b", "a-b")]
#[case("a & b", "a-b")]
#[case("a-&-b", "a-b")]
#[case("-_x_-", "x")]
#[case("UPPER_case", "upper_case")]
#[case("Done!", "done")]
#[case("x_y", "x_y")]
#[case("123", "123")]
#[case("tab\tand\nnewline", "tab-and-newline")]
#[case("Café", "caf")]
#[case("日本", "")]
#[case("", "")]
#[case("---", "")]
#[case("_", "")]
fn a_name_is_slugified_as_django_does(#[case] name: &str, #[case] slug: &str) {
    assert_eq!(slugify(name), slug, "{name:?}");
}

proptest! {
    /// Whatever the name: a slug is lower-case letters, digits, underscores and single hyphens, begins and ends with a letter or a
    /// digit (or is empty), and slugifying it again changes nothing.
    #[test]
    fn prop_a_slug_is_clean_and_stable(name in any::<String>()) {
        let slug = slugify(&name);
        prop_assert!(slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'), "{}", slug);
        prop_assert!(!slug.contains("--"), "{}", slug);
        prop_assert!(!slug.starts_with(['-', '_']) && !slug.ends_with(['-', '_']), "{}", slug);
        prop_assert_eq!(slugify(&slug), slug);
    }
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
