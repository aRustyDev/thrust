//! The pin cookie: sealed pins open to themselves until they expire, and nothing else opens (src/pin.rs).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions, not the helpers beside them"
)]
mod common;

use common::{proptest_config, OTHER_PIN_KEY, PIN_KEY};
use proptest::prelude::*;
use rstest::rstest;
use thrust_edge::pin::{self, Kind, Pin, PinError, COOKIE_NAME};

fn kind() -> impl Strategy<Value = Kind> {
    prop_oneof![Just(Kind::Human), Just(Kind::Ci)]
}

/// A valid pin: the grammar of each field, boundaries included (1-character and maximum-length values).
fn pin_strategy() -> impl Strategy<Value = Pin> {
    (
        prop_oneof!["[!-~]{1,254}", Just("a".to_string()), Just("x".repeat(254))],
        kind(),
        prop_oneof!["[a-z]{1,16}", Just("z".repeat(16))],
        prop_oneof!["[a-z0-9][a-z0-9.-]{0,56}", Just("v0.3.0".to_string()), Just("9".repeat(57))],
        1u64..u64::MAX,
    )
        .prop_map(|(principal, kind, audience, requested, exp)| Pin { principal, kind, audience, requested, exp })
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Every valid pin opens to itself before it expires.
    #[test]
    fn prop_a_sealed_pin_opens_to_itself(p in pin_strategy(), before in 1u64..1_000_000) {
        let now = p.exp.saturating_sub(before);
        prop_assume!(now < p.exp);
        let value = pin::seal(&p, PIN_KEY).unwrap();
        prop_assert!(value.len() <= pin::MAX_VALUE_LEN);
        prop_assert_eq!(pin::open(&value, PIN_KEY, now), Ok(p));
    }

    /// At and after its expiry a pin is refused as expired, never accepted.
    #[test]
    fn prop_a_pin_is_refused_from_its_expiry_on(p in pin_strategy(), after in 0u64..1_000_000) {
        let value = pin::seal(&p, PIN_KEY).unwrap();
        prop_assert_eq!(pin::open(&value, PIN_KEY, p.exp.saturating_add(after)), Err(PinError::Expired));
    }

    /// Changing any one byte of a sealed value makes it refused (and not expired: the MAC is checked first).
    #[test]
    fn prop_any_changed_byte_is_refused(p in pin_strategy(), at in any::<prop::sample::Index>(), to in 0x21u8..0x7f) {
        let value = pin::seal(&p, PIN_KEY).unwrap();
        let mut bytes = value.clone().into_bytes();
        let i = at.index(bytes.len());
        prop_assume!(bytes[i] != to);
        bytes[i] = to;
        let tampered = String::from_utf8(bytes).unwrap();
        let got = pin::open(&tampered, PIN_KEY, 0);
        prop_assert!(matches!(got, Err(PinError::BadSignature | PinError::Malformed)), "{tampered}: {got:?}");
    }

    /// A pin sealed with another key is a forgery.
    #[test]
    fn prop_another_key_is_a_forgery(p in pin_strategy()) {
        let value = pin::seal(&p, OTHER_PIN_KEY).unwrap();
        prop_assert_eq!(pin::open(&value, PIN_KEY, 0), Err(PinError::BadSignature));
    }

    /// Whatever text arrives, open answers without panicking, and accepts nothing it did not seal.
    #[test]
    fn prop_arbitrary_text_is_never_accepted(s in ".{0,1100}") {
        prop_assert!(pin::open(&s, PIN_KEY, 0).is_err());
    }

    /// A Cookie header with exactly one pin cookie among others yields its value; with two, none.
    #[test]
    fn prop_the_cookie_header_yields_exactly_one_pin(
        others in prop::collection::vec(("[A-Za-z_][A-Za-z0-9_]{0,12}", "[A-Za-z0-9._-]{0,20}"), 0..6),
        value in "[A-Za-z0-9._-]{1,40}",
        at in any::<prop::sample::Index>(),
        twice in any::<bool>(),
    ) {
        let mut pairs: Vec<String> = others.iter().filter(|(n, _)| n != COOKIE_NAME).map(|(n, v)| format!("{n}={v}")).collect();
        let pos = at.index(pairs.len() + 1);
        pairs.insert(pos, format!("{COOKIE_NAME}={value}"));
        if twice {
            pairs.push(format!("{COOKIE_NAME}=other"));
        }
        let header = pairs.join("; ");
        let want = if twice { None } else { Some(value.as_str()) };
        prop_assert_eq!(pin::from_cookie_header(&header), want);
    }
}

#[derive(Clone, Copy, Debug)]
enum Field {
    Principal,
    Audience,
    Requested,
}

/// The spec of each field's grammar, written here independently of src/pin.rs (ADR-0111): a principal is 1 to 254 visible ASCII
/// characters; an audience 1 to 16 lowercase letters; a name 1 to 57 characters, a lowercase letter or digit first, then lowercase
/// letters, digits, `.` or `-`.
fn spec_valid(field: Field, s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    match field {
        Field::Principal => (1..=254).contains(&chars.len()) && chars.iter().all(|c| ('!'..='~').contains(c)),
        Field::Audience => (1..=16).contains(&chars.len()) && chars.iter().all(|c| c.is_ascii_lowercase()),
        Field::Requested => {
            (1..=57).contains(&chars.len())
                && chars.first().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && chars.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '.' || *c == '-')
        }
    }
}

fn implemented_valid(field: Field, s: &str) -> bool {
    match field {
        Field::Principal => pin::valid_principal(s),
        Field::Audience => pin::valid_audience(s),
        Field::Requested => pin::valid_name(s),
    }
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// The validators agree with the spec on arbitrary text, weighted toward each grammar's alphabet and length boundaries.
    #[test]
    fn prop_the_grammars_match_their_spec(
        s in prop_oneof![".{0,300}", "[a-z0-9.-]{0,60}", "[a-z]{0,20}", "[!-~]{250,258}", "[ -~]{0,20}"],
    ) {
        for field in [Field::Principal, Field::Audience, Field::Requested] {
            prop_assert_eq!(implemented_valid(field, &s), spec_valid(field, &s), "{:?} {:?}", field, s);
        }
    }
}

/// One value per class on each side of every grammar: a pin with a field outside its grammar is not sealed (and a sealed value
/// cannot smuggle one in: the payload is under the MAC, so the only way in is through `seal`).
#[rstest]
fn each_field_outside_its_grammar_is_refused(
    #[values(Field::Principal, Field::Audience, Field::Requested)] field: Field,
    #[values("", "with space", "UPPER", "a_b", "tab\t", "é", "x/y", "..", "-lead", "toolong")] bad: &str,
) {
    let mut p = Pin {
        principal: "ci.access".into(),
        kind: Kind::Ci,
        audience: "rc".into(),
        requested: "v0.3.0".into(),
        exp: 10,
    };
    let value = match (bad, field) {
        ("toolong", Field::Principal) => "a".repeat(255),
        ("toolong", Field::Audience) => "a".repeat(17),
        ("toolong", Field::Requested) => "a".repeat(58),
        _ => bad.to_string(),
    };
    let want_ok = spec_valid(field, &value);
    match field {
        Field::Principal => p.principal = value,
        Field::Audience => p.audience = value,
        Field::Requested => p.requested = value,
    }
    let got = pin::seal(&p, PIN_KEY);
    if want_ok {
        assert!(got.is_ok(), "{field:?} {bad:?} is in its grammar, so it seals");
    } else {
        assert_eq!(got, Err(PinError::Invalid), "{field:?} {bad:?}");
    }
}

/// The grammars themselves, at their boundaries (ADR-0111 names `v0.3.0`, `staging`, `dev-*`, `rc`, `dev`).
#[rstest]
#[case::spec_release("v0.3.0", true)]
#[case::spec_alias("staging", true)]
#[case::spec_dev("dev-login-page", true)]
#[case::longest(&"a".repeat(57), true)]
#[case::too_long(&"a".repeat(58), false)]
#[case::leading_dot(".v1", false)]
#[case::leading_dash("-v1", false)]
#[case::underscore("a_b", false)]
#[case::empty("", false)]
fn names(#[case] name: &str, #[case] ok: bool) {
    assert_eq!(pin::valid_name(name), ok);
}

#[test]
fn a_key_shorter_than_32_bytes_is_refused_both_ways() {
    let p = Pin { principal: "a".into(), kind: Kind::Human, audience: "rc".into(), requested: "x".into(), exp: 10 };
    assert_eq!(pin::seal(&p, &PIN_KEY[..31]), Err(PinError::KeyTooShort));
    let value = pin::seal(&p, PIN_KEY).unwrap();
    assert_eq!(pin::open(&value, &PIN_KEY[..31], 0), Err(PinError::KeyTooShort));
}

#[test]
fn an_oversized_value_is_refused_before_anything_else() {
    let value = "v1.".to_string() + &"A".repeat(pin::MAX_VALUE_LEN);
    assert_eq!(pin::open(&value, PIN_KEY, 0), Err(PinError::TooLong));
}

/// A correctly signed payload that is not exactly a pin (an extra field, a missing one, a wrong kind) is refused: signing it
/// needs the key, so this stands for a key that signed something else.
#[rstest]
#[case::extra_field(r#"{"p":"a","k":"ci","a":"rc","r":"x","e":10,"admin":true}"#)]
#[case::missing_field(r#"{"p":"a","k":"ci","a":"rc","r":"x"}"#)]
#[case::unknown_kind(r#"{"p":"a","k":"root","a":"rc","r":"x","e":10}"#)]
#[case::not_json("not json")]
fn a_signed_payload_that_is_not_a_pin_is_refused(#[case] payload: &str) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use hmac::{Hmac, Mac};
    let signed = format!("v1.{}", URL_SAFE_NO_PAD.encode(payload));
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(PIN_KEY).unwrap();
    m.update(signed.as_bytes());
    let value = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(m.finalize().into_bytes()));
    assert_eq!(pin::open(&value, PIN_KEY, 0), Err(PinError::Invalid));
}

#[test]
fn the_set_cookie_headers_carry_the_host_prefix_attributes() {
    let set = pin::set_cookie("v", 60);
    let clear = pin::clear_cookie();
    for h in [&set, &clear] {
        assert!(h.starts_with(&format!("{COOKIE_NAME}=")));
        for attr in ["Path=/", "Secure", "HttpOnly", "SameSite=Lax"] {
            assert!(h.split("; ").any(|a| a == attr), "{h} lacks {attr}");
        }
        assert!(!h.contains("Domain"), "a __Host- cookie has no Domain: {h}");
    }
    assert!(set.ends_with("Max-Age=60"));
    assert!(clear.ends_with("Max-Age=0"));
}

/// The size limit is the longest value read: exactly `MAX_VALUE_LEN` bytes is read (and refused for what it is), one more is not.
#[rstest]
fn the_size_limit_is_inclusive(#[values(-1i64, 0, 1)] delta: i64) {
    let len = usize::try_from(pin::MAX_VALUE_LEN as i64 + delta).unwrap();
    let got = pin::open(&"A".repeat(len), PIN_KEY, 0);
    assert_eq!(got == Err(PinError::TooLong), delta > 0, "{len} bytes: {got:?}");
}

/// Values signed with the right key but not in the v1 shape (another version, an empty payload, an empty MAC) are malformed: the
/// shape is checked before the MAC, so a valid MAC cannot make another format readable.
#[rstest]
#[case::another_version("v2", r#"{"p":"a","k":"ci","a":"rc","r":"x","e":10}"#, true)]
#[case::empty_payload("v1", "", true)]
#[case::empty_mac("v1", r#"{"p":"a","k":"ci","a":"rc","r":"x","e":10}"#, false)]
fn a_signed_value_not_in_the_v1_shape_is_malformed(
    #[case] version: &str,
    #[case] payload: &str,
    #[case] with_mac: bool,
) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use hmac::{Hmac, Mac};
    let signed = format!("{version}.{}", URL_SAFE_NO_PAD.encode(payload));
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(PIN_KEY).unwrap();
    m.update(signed.as_bytes());
    let mac = if with_mac { URL_SAFE_NO_PAD.encode(m.finalize().into_bytes()) } else { String::new() };
    assert_eq!(pin::open(&format!("{signed}.{mac}"), PIN_KEY, 0), Err(PinError::Malformed));
}
