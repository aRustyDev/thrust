//! The DRF-style validators of `fields.rs`, on the inputs the fixtures cannot reach: every spelling of a boolean, every form
//! of a UUID, the exact length limits, the kinds of value a text field takes, and which setting lands where. The fixtures
//! pin the messages the pin gave; these pin the rules behind them, so a validator that is wrong in a way no recording
//! shows (a spelling dropped, a bound off by one) fails here.

use proptest::prelude::*;
use serde_json::{json, Map, Value};
use thrust_contracts::*;
use thrust_ports::ProfilePatch;

use crate::fields::{django_bool, profile_patch, user_patch, FieldErrors};

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("an object")
}

fn profile(body: Value) -> Result<ProfilePatch, FieldErrors> {
    profile_patch(&obj(body))
}

fn user(body: Value) -> Result<ProfilePatch, FieldErrors> {
    user_patch(&obj(body))
}

/// The one message a refused field carries.
fn message(errors: &FieldErrors, field: &str) -> String {
    let list = errors.get(field).unwrap_or_else(|| panic!("no error for {field}: {errors:?}")).as_array().unwrap();
    assert_eq!(list.len(), 1, "{errors:?}");
    list[0].as_str().unwrap().to_string()
}

// --- booleans -----------------------------------------------------------------------------------------------------------

const TRUE: &[&str] = &["t", "T", "y", "Y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON", "1"];
const FALSE: &[&str] = &["f", "F", "n", "N", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF", "0"];

/// Every candidate worth asking: DRF's spellings, every case variant of each, near misses, and the JSON scalars.
fn candidates() -> Vec<Value> {
    let mut v: Vec<Value> = Vec::new();
    for s in TRUE.iter().chain(FALSE) {
        for form in [s.to_string(), s.to_uppercase(), s.to_lowercase(), format!(" {s}"), format!("{s} "), format!("{s}x")] {
            v.push(json!(form));
        }
    }
    v.extend([json!("maybe"), json!(""), json!("2"), json!("tRuE"), json!(true), json!(false), json!(1), json!(0), json!(1.0), json!(0.0), json!(2), json!(-1), json!(0.5), json!([]), json!({}), json!([true])]);
    v
}

/// What DRF's `BooleanField` says of a candidate: `Some(b)` for a boolean, `None` for a refusal. Written from DRF's sets.
fn drf_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) if TRUE.contains(&s.as_str()) => Some(true),
        Value::String(s) if FALSE.contains(&s.as_str()) => Some(false),
        Value::Number(n) if n.as_f64() == Some(1.0) => Some(true),
        Value::Number(n) if n.as_f64() == Some(0.0) => Some(false),
        _ => None,
    }
}

/// Each typed flag takes the booleans and refuses the rest, with the pin's message, and sets ITS field and no other.
#[test]
fn a_boolean_field_takes_exactly_drfs_spellings_and_sets_its_own_field() {
    for v in candidates() {
        for field in ["is_onboarded", "is_tour_completed", "has_marketing_email_consent", "is_app_rail_docked"] {
            let got = profile(json!({ field: v }));
            match (drf_bool(&v), got) {
                (Some(b), Ok(p)) => {
                    let (onboarded, tour) = (p.is_onboarded, p.is_tour_completed);
                    match field {
                        "is_onboarded" => assert_eq!((onboarded, tour, p.preferences.len()), (Some(b), None, 0), "{field} {v}"),
                        "is_tour_completed" => assert_eq!((onboarded, tour, p.preferences.len()), (None, Some(b), 0), "{field} {v}"),
                        _ => assert_eq!((onboarded, tour, p.preferences.get(field)), (None, None, Some(&json!(b))), "{field} {v}"),
                    }
                }
                (None, Err(e)) => assert_eq!(message(&e, field), "Must be a valid boolean.", "{field} {v}"),
                (want, got) => panic!("{field} {v}: expected {want:?}, got {got:?}"),
            }
        }
    }
}

#[test]
fn a_boolean_that_is_null_is_refused_as_null() {
    for field in ["is_onboarded", "is_tour_completed", "has_marketing_email_consent"] {
        assert_eq!(message(&profile(json!({ field: null })).unwrap_err(), field), "This field may not be null.", "{field}");
    }
}

/// Django's `BooleanField.to_python`, which the S27 and X04 routes go through by saving the raw body value.
#[test]
fn django_bool_takes_exactly_djangos_spellings() {
    const T: &[&str] = &["t", "True", "1"];
    const F: &[&str] = &["f", "False", "0"];
    for v in candidates() {
        let want = match &v {
            Value::Bool(b) => Ok(*b),
            Value::String(s) if T.contains(&s.as_str()) => Ok(true),
            Value::String(s) if F.contains(&s.as_str()) => Ok(false),
            Value::Number(n) if n.as_f64() == Some(1.0) => Ok(true),
            Value::Number(n) if n.as_f64() == Some(0.0) => Ok(false),
            _ => Err(()),
        };
        assert_eq!(django_bool(Some(&v)), want, "{v}");
    }
    assert_eq!(django_bool(None), Ok(false), "a missing key is false");
    assert_eq!(django_bool(Some(&Value::Null)), Err(()), "null is refused");
}

// --- UUIDs --------------------------------------------------------------------------------------------------------------

/// `last_workspace_id` takes a UUID however Python's `uuid.UUID(hex=...)` would after DRF strips `urn:`, `uuid:`, braces and
/// hyphens: 32 hex digits, hyphens anywhere, either case.
fn uuid_forms(u: Uuid, hyphens_at: &[usize]) -> Vec<String> {
    let hex = u.simple().to_string();
    let mut sprinkled = String::new();
    for (i, c) in hex.chars().enumerate() {
        if hyphens_at.contains(&i) {
            sprinkled.push('-');
        }
        sprinkled.push(c);
    }
    vec![
        u.hyphenated().to_string(),
        hex.clone(),
        hex.to_uppercase(),
        format!("{{{}}}", u.hyphenated()),
        format!("urn:uuid:{}", u.hyphenated()),
        sprinkled,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_a_uuid_in_any_of_its_forms_is_the_same_uuid(n in any::<u128>(), hyphens in prop::collection::vec(0usize..32, 0..6)) {
        let u = Uuid::from_u128(n);
        for form in uuid_forms(u, &hyphens) {
            let p = profile(json!({"last_workspace_id": form})).unwrap_or_else(|e| panic!("{form}: {e:?}"));
            prop_assert_eq!(p.last_workspace_id, Some(Some(WorkspaceId(u))), "{}", form);
        }
    }

    /// Control: one digit dropped or added, or one non-hex character, is not a UUID, wherever it is.
    #[test]
    fn prop_a_uuid_that_is_one_digit_off_is_refused(n in any::<u128>(), at in 0usize..32, junk in prop::sample::select(vec!['g', 'z', ' ', '/', '+'])) {
        let hex = Uuid::from_u128(n).simple().to_string();
        let mut short = hex.clone();
        short.remove(at);
        let mut long = hex.clone();
        long.insert(at, 'a');
        let mut bad = hex.clone();
        bad.replace_range(at..=at, &junk.to_string());
        for s in [short, long, bad] {
            let e = profile(json!({"last_workspace_id": s})).unwrap_err();
            prop_assert_eq!(message(&e, "last_workspace_id"), "Must be a valid UUID.", "{}", s);
        }
    }
}

#[test]
fn a_last_workspace_that_is_null_is_cleared_and_a_non_string_is_refused() {
    assert_eq!(profile(json!({"last_workspace_id": null})).unwrap().last_workspace_id, Some(None));
    for v in [json!(5), json!(true), json!([]), json!({}), json!("")] {
        let e = profile(json!({"last_workspace_id": v})).unwrap_err();
        assert_eq!(message(&e, "last_workspace_id"), "Must be a valid UUID.", "{v}");
    }
    assert_eq!(profile(json!({})).unwrap().last_workspace_id, None, "not named: not touched");
}

// --- text, lengths and trimming ------------------------------------------------------------------------------------------

#[test]
fn the_length_limits_are_exact_and_count_characters_after_trimming() {
    // (field, limit)
    for (field, limit) in [("first_name", 255usize), ("last_name", 255), ("display_name", 255)] {
        let at = |n: usize, c: &str| c.repeat(n);
        assert!(user(json!({ field: at(limit, "x") })).is_ok(), "{field} at the limit");
        assert!(user(json!({ field: format!("  {}  ", at(limit, "x")) })).is_ok(), "{field}: the limit is on the trimmed text");
        assert!(user(json!({ field: at(limit, "\u{e9}") })).is_ok(), "{field}: characters, not bytes");
        let e = user(json!({ field: at(limit + 1, "x") })).unwrap_err();
        assert_eq!(message(&e, field), format!("Ensure this field has no more than {limit} characters."), "{field}");
    }
    for (field, limit) in [("role", 300usize), ("billing_address_country", 255), ("company_name", 255), ("language", 255), ("background_color", 255)] {
        assert!(profile(json!({ field: "x".repeat(limit) })).is_ok(), "{field} at the limit");
        let e = profile(json!({ field: "x".repeat(limit + 1) })).unwrap_err();
        assert_eq!(message(&e, field), format!("Ensure this field has no more than {limit} characters."), "{field}");
    }
    // text without a limit takes a long value
    assert!(profile(json!({"use_case": "x".repeat(5000)})).is_ok());
    assert!(user(json!({"avatar": "x".repeat(5000)})).is_ok());
}

#[test]
fn a_text_field_trims_takes_numbers_as_text_and_refuses_the_rest() {
    let p = user(json!({"first_name": "  Pat \n"})).unwrap();
    assert_eq!(p.first_name.as_deref(), Some("Pat"));
    // whitespace alone is blank: fine where blank is allowed, refused where it is not
    assert_eq!(user(json!({"first_name": "   "})).unwrap().first_name.as_deref(), Some(""));
    assert_eq!(message(&user(json!({"display_name": "   "})).unwrap_err(), "display_name"), "This field may not be blank.");
    assert_eq!(message(&profile(json!({"language": "  "})).unwrap_err(), "language"), "This field may not be blank.");
    // a number is accepted as its text, a bool, list and object are not
    assert_eq!(user(json!({"first_name": 5})).unwrap().first_name.as_deref(), Some("5"));
    assert_eq!(profile(json!({"role": 5})).unwrap().preferences.get("role"), Some(&json!("5")));
    for v in [json!(true), json!(false), json!(["a"]), json!({"a": 1})] {
        assert_eq!(message(&user(json!({"first_name": v})).unwrap_err(), "first_name"), "Not a valid string.", "{v}");
    }
    // null: refused where the column is not null, stored as null where it is
    assert_eq!(message(&user(json!({"first_name": null})).unwrap_err(), "first_name"), "This field may not be null.");
    assert_eq!(profile(json!({"role": null})).unwrap().preferences.get("role"), Some(&Value::Null));
    assert_eq!(profile(json!({"use_case": null})).unwrap().preferences.get("use_case"), Some(&Value::Null));
    assert_eq!(profile(json!({"role": ""})).unwrap().preferences.get("role"), Some(&json!("")), "role may be blank");
}

#[test]
fn only_the_names_are_checked_for_urls_and_the_fields_in_a_body_are_checked_together() {
    // a display name may look like a URL: the pin has no such rule for it
    assert_eq!(user(json!({"display_name": "example.com"})).unwrap().display_name.as_deref(), Some("example.com"));
    // every refused field is reported, not just the first (DRF collects them)
    let e = user(json!({"first_name": "a.bc", "last_name": "http://x.test", "display_name": ""})).unwrap_err();
    assert_eq!(e.len(), 3, "{e:?}");
    assert_eq!(message(&e, "first_name"), "First name cannot contain a URL.");
    assert_eq!(message(&e, "last_name"), "Last name cannot contain a URL.");
    // fields the serializer does not know, or marks read only, are ignored
    let p = user(json!({"email": "x@y.test", "is_bot": true, "id": "z", "token": "t", "username": "u", "nonsense": 1})).unwrap();
    assert_eq!(p, ProfilePatch::default());
}

// --- the time zone ------------------------------------------------------------------------------------------------------

#[test]
fn a_time_zone_is_a_choice() {
    assert_eq!(user(json!({"user_timezone": "Asia/Kolkata"})).unwrap().timezone.as_deref(), Some("Asia/Kolkata"));
    assert_eq!(message(&user(json!({"user_timezone": "Mars/Olympus"})).unwrap_err(), "user_timezone"), "\"Mars/Olympus\" is not a valid choice.");
    assert_eq!(message(&user(json!({"user_timezone": null})).unwrap_err(), "user_timezone"), "This field may not be null.");
    // a non-string is quoted as Python would print it
    assert_eq!(message(&user(json!({"user_timezone": 5})).unwrap_err(), "user_timezone"), "\"5\" is not a valid choice.");
    assert_eq!(message(&user(json!({"user_timezone": true})).unwrap_err(), "user_timezone"), "\"True\" is not a valid choice.");
    assert_eq!(message(&user(json!({"user_timezone": ""})).unwrap_err(), "user_timezone"), "\"\" is not a valid choice.");
}

// --- the cover image ----------------------------------------------------------------------------------------------------

#[test]
fn a_cover_image_is_a_url_or_null() {
    for ok in ["https://example.test/c.png", "http://h", "ftp://h/x", "ftps://h?q=1", "https://h#frag"] {
        assert_eq!(user(json!({"cover_image": ok})).unwrap().cover_image, Some(Some(ok.to_string())), "{ok}");
    }
    assert_eq!(user(json!({"cover_image": null})).unwrap().cover_image, Some(None), "null clears it");
    for bad in ["https://", "example.test", "https:// spaced", "mailto:a@b.test", "https:///path"] {
        assert_eq!(message(&user(json!({"cover_image": bad})).unwrap_err(), "cover_image"), "Enter a valid URL.", "{bad}");
    }
    assert_eq!(message(&user(json!({"cover_image": ""})).unwrap_err(), "cover_image"), "This field may not be blank.");
    let at = |n: usize| format!("https://h/{}", "a".repeat(n - "https://h/".len()));
    assert!(user(json!({"cover_image": at(800)})).is_ok(), "800 is the longest");
    assert_eq!(message(&user(json!({"cover_image": at(801)})).unwrap_err(), "cover_image"), "Ensure this field has no more than 800 characters.");
}

// --- the profile settings ---------------------------------------------------------------------------------------------

#[test]
fn every_profile_setting_lands_under_its_own_name_and_an_unknown_one_is_ignored() {
    let body = json!({
        "theme": {"t": 1}, "is_app_rail_docked": false, "use_case": "u", "role": "r", "billing_address_country": "FR",
        "billing_address": {"a": 1}, "has_billing_address": true, "company_name": "c", "notification_view_mode": "compact",
        "is_smooth_cursor_enabled": true, "is_mobile_onboarded": true, "mobile_onboarding_step": {"m": true},
        "mobile_timezone_auto_set": true, "language": "fr", "start_of_the_week": 3, "goals": {"g": 1},
        "background_color": "#abcdef", "is_navigation_tour_completed": true, "has_marketing_email_consent": true,
        "is_subscribed_to_changelog": true, "product_tour": {"p": true}, "nonsense": 1, "user": "x", "id": "y",
    });
    let p = profile(body.clone()).unwrap();
    let mut want = body.as_object().unwrap().clone();
    for ignored in ["nonsense", "user", "id"] {
        want.remove(ignored);
    }
    assert_eq!(p.preferences, want, "each setting under its own name, the unknown and read-only ones not at all");
    assert_eq!((p.is_onboarded, p.is_tour_completed, p.last_workspace_id, p.onboarding_step), (None, None, None, None));
    // JSON settings take any JSON but null, except `billing_address`, which may be null
    for field in ["theme", "mobile_onboarding_step", "goals", "product_tour"] {
        assert_eq!(message(&profile(json!({ field: null })).unwrap_err(), field), "This field may not be null.", "{field}");
        assert!(profile(json!({ field: [1, 2] })).is_ok(), "{field} takes a list");
    }
    assert_eq!(profile(json!({"billing_address": null})).unwrap().preferences.get("billing_address"), Some(&Value::Null));
}

#[test]
fn the_choices_are_the_pins() {
    for (field, good, bad) in [("notification_view_mode", ["full", "compact"], ["huge", "FULL"]), ("start_of_the_week", ["0", "6"], ["7", "-1"])] {
        for v in good {
            assert!(profile(json!({ field: v })).is_ok(), "{field} {v}");
        }
        for v in bad {
            assert_eq!(message(&profile(json!({ field: v })).unwrap_err(), field), format!("\"{v}\" is not a valid choice."), "{field} {v}");
        }
    }
    // a day of the week is stored as the number it is, whether the client sent text or a number
    for sent in [json!(2), json!("2")] {
        assert_eq!(profile(json!({"start_of_the_week": sent})).unwrap().preferences.get("start_of_the_week"), Some(&json!(2)));
    }
    // and every day is accepted, nothing past Saturday
    for day in 0..=6 {
        assert!(profile(json!({"start_of_the_week": day})).is_ok(), "{day}");
    }
    assert!(profile(json!({"start_of_the_week": 7})).is_err());
    assert_eq!(message(&profile(json!({"start_of_the_week": null})).unwrap_err(), "start_of_the_week"), "This field may not be null.");
}

#[test]
fn the_onboarding_steps_are_a_json_object() {
    assert_eq!(profile(json!({"onboarding_step": {"profile_complete": true}})).unwrap().onboarding_step, Some(json!({"profile_complete": true})));
    assert_eq!(profile(json!({"onboarding_step": {}})).unwrap().onboarding_step, Some(json!({})));
    assert_eq!(message(&profile(json!({"onboarding_step": null})).unwrap_err(), "onboarding_step"), "This field may not be null.");
    for bad in [json!([]), json!("x"), json!(1), json!(true)] {
        assert_eq!(message(&profile(json!({"onboarding_step": bad})).unwrap_err(), "onboarding_step"), "Must be a JSON object.");
    }
}

#[test]
fn every_refused_field_in_a_profile_body_is_reported_together() {
    let e = profile(json!({"is_onboarded": "maybe", "start_of_the_week": 9, "language": "", "role": "ok"})).unwrap_err();
    assert_eq!(e.keys().cloned().collect::<std::collections::BTreeSet<_>>(), ["is_onboarded", "language", "start_of_the_week"].map(String::from).into());
}
