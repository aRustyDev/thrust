//! The Access token on an opt-in path: only an RS256 token signed by the team, for this path's AUD, in its validity window, with a
//! principal, is accepted (src/access.rs).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions, not the helpers beside them"
)]
mod common;

use common::{b64, jwks_doc, proptest_config, rsa_keys, token};
use hmac::{Hmac, Mac};
use proptest::prelude::*;
use rsa::traits::PublicKeyParts as _;
use rstest::rstest;
use serde_json::json;
use thrust_edge::access::{self, AccessError, Expect, Identity};
use thrust_edge::pin::Kind;

const ISS: &str = "https://team.cloudflareaccess.com";
const AUD: &str = "523f788ad6fe18e01ad8fbb6b85f9613323faa23b0d570fd418e782cd3e6afc3";
const OTHER_AUD: &str = "0000788ad6fe18e01ad8fbb6b85f9613323faa23b0d570fd418e782cd3e60000";
const KID: &str = "k1";
const NOW: u64 = 1_800_000_000;

fn jwks() -> access::Jwks {
    access::parse_jwks(jwks_doc(KID, &rsa_keys().0).as_bytes()).unwrap()
}

fn expect() -> Expect<'static> {
    Expect { issuer: ISS, audience: AUD, now: NOW }
}

fn header() -> serde_json::Value {
    json!({ "alg": "RS256", "kid": KID, "typ": "JWT" })
}

/// A person's token as Access issues it (type, email, country and identity_nonce beside the registered claims).
fn human_claims() -> serde_json::Value {
    json!({ "aud": [AUD], "iss": ISS, "exp": NOW + 600, "iat": NOW - 10, "nbf": NOW - 10, "email": "dev@example.org",
            "type": "app", "sub": "8f2c", "country": "US", "identity_nonce": "n" })
}

/// A service token's, as the spike recorded it (research/0016): `common_name`, an empty `sub`, no email.
fn ci_claims() -> serde_json::Value {
    json!({ "aud": [AUD], "iss": ISS, "exp": NOW + 600, "iat": NOW - 10, "nbf": NOW - 10,
            "common_name": "0123456789abcdef.access", "type": "app", "sub": "" })
}

fn with(mut claims: serde_json::Value, key: &str, value: serde_json::Value) -> serde_json::Value {
    claims[key] = value;
    claims
}

fn without(mut claims: serde_json::Value, key: &str) -> serde_json::Value {
    claims.as_object_mut().unwrap().remove(key);
    claims
}

#[test]
fn a_persons_token_is_a_human_and_a_service_tokens_is_ci() {
    let (team, _) = rsa_keys();
    assert_eq!(
        access::verify(&token(&header(), &human_claims(), Some(team)), &jwks(), &expect()),
        Ok(Identity { principal: "dev@example.org".into(), kind: Kind::Human })
    );
    assert_eq!(
        access::verify(&token(&header(), &ci_claims(), Some(team)), &jwks(), &expect()),
        Ok(Identity { principal: "0123456789abcdef.access".into(), kind: Kind::Ci })
    );
    // `aud` may also be a single string
    let one = with(ci_claims(), "aud", json!(AUD));
    assert!(access::verify(&token(&header(), &one, Some(team)), &jwks(), &expect()).is_ok());
}

/// The kinds of forged or wrong tokens, each refused for its own reason. A fixed list because each row is a different attack,
/// not a point in a domain; every row's token is otherwise the valid CI token.
#[derive(Clone, Copy, Debug)]
enum Bad {
    WrongAudience,
    WrongIssuer,
    Expired,
    ExpiresNow,
    NotYetValid,
    SignedByAnotherKey,
    AlgNone,
    AlgHs256WithThePublicKey,
    AlgRs512,
    Crit,
    NoKid,
    UnknownKid,
    TwoParts,
    FourParts,
    NotBase64,
    NoPrincipal,
    EmailNotVisibleAscii,
    CommonNameNotVisibleAscii,
}

#[rstest]
fn every_bad_token_is_refused_for_its_reason(
    #[values(
        Bad::WrongAudience,
        Bad::WrongIssuer,
        Bad::Expired,
        Bad::ExpiresNow,
        Bad::NotYetValid,
        Bad::SignedByAnotherKey,
        Bad::AlgNone,
        Bad::AlgHs256WithThePublicKey,
        Bad::AlgRs512,
        Bad::Crit,
        Bad::NoKid,
        Bad::UnknownKid,
        Bad::TwoParts,
        Bad::FourParts,
        Bad::NotBase64,
        Bad::NoPrincipal,
        Bad::EmailNotVisibleAscii,
        Bad::CommonNameNotVisibleAscii
    )]
    bad: Bad,
) {
    let (team, attacker) = rsa_keys();
    let good = ci_claims();
    let (t, want) = match bad {
        Bad::WrongAudience => {
            (token(&header(), &with(good, "aud", json!([OTHER_AUD])), Some(team)), AccessError::Audience)
        }
        Bad::WrongIssuer => (
            token(&header(), &with(good, "iss", json!("https://other.cloudflareaccess.com")), Some(team)),
            AccessError::Issuer,
        ),
        Bad::Expired => (token(&header(), &with(good, "exp", json!(NOW - 1)), Some(team)), AccessError::Expired),
        Bad::ExpiresNow => (token(&header(), &with(good, "exp", json!(NOW)), Some(team)), AccessError::Expired),
        Bad::NotYetValid => (
            token(&header(), &with(good, "nbf", json!(NOW + access::NBF_LEEWAY + 1)), Some(team)),
            AccessError::NotYetValid,
        ),
        Bad::SignedByAnotherKey => (token(&header(), &good, Some(attacker)), AccessError::BadSignature),
        Bad::AlgNone => (token(&json!({ "alg": "none", "kid": KID }), &good, None), AccessError::Algorithm),
        Bad::AlgHs256WithThePublicKey => {
            // the classic confusion: an HMAC keyed with the public key's bytes, under the team's kid
            let h = json!({ "alg": "HS256", "kid": KID });
            let signed = format!("{}.{}", b64(h.to_string().as_bytes()), b64(good.to_string().as_bytes()));
            let mut m = Hmac::<sha2::Sha256>::new_from_slice(&team.to_public_key().n().to_bytes_be()).unwrap();
            m.update(signed.as_bytes());
            (format!("{signed}.{}", b64(&m.finalize().into_bytes())), AccessError::Algorithm)
        }
        Bad::AlgRs512 => (token(&json!({ "alg": "RS512", "kid": KID }), &good, Some(team)), AccessError::Algorithm),
        Bad::Crit => {
            (token(&json!({ "alg": "RS256", "kid": KID, "crit": ["exp"] }), &good, Some(team)), AccessError::Algorithm)
        }
        Bad::NoKid => (token(&json!({ "alg": "RS256" }), &good, Some(team)), AccessError::UnknownKey),
        Bad::UnknownKid => (token(&json!({ "alg": "RS256", "kid": "k2" }), &good, Some(team)), AccessError::UnknownKey),
        Bad::TwoParts => {
            let t = token(&header(), &good, Some(team));
            (t.rsplit_once('.').unwrap().0.to_string(), AccessError::Malformed)
        }
        Bad::FourParts => (token(&header(), &good, Some(team)) + ".x", AccessError::Malformed),
        Bad::NotBase64 => ("eyJ!.eyJ.sig".to_string(), AccessError::Malformed),
        Bad::NoPrincipal => (token(&header(), &without(good, "common_name"), Some(team)), AccessError::NoPrincipal),
        Bad::EmailNotVisibleAscii => {
            (token(&header(), &with(good, "email", json!("a b@example.org")), Some(team)), AccessError::NoPrincipal)
        }
        Bad::CommonNameNotVisibleAscii => {
            (token(&header(), &with(good, "common_name", json!("ci token")), Some(team)), AccessError::NoPrincipal)
        }
    };
    assert_eq!(access::verify(&t, &jwks(), &expect()), Err(want), "{bad:?}");
}

#[test]
fn an_oversized_token_is_refused_before_it_is_parsed() {
    let t = "a".repeat(access::MAX_TOKEN_LEN + 1);
    assert_eq!(access::verify(&t, &jwks(), &expect()), Err(AccessError::TooLong));
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Over generated validity windows, a team-signed token is accepted exactly when now is before `exp` and not more than the
    /// leeway before `nbf`, and its principal comes back.
    #[test]
    fn prop_the_validity_window(
        // the boundaries are weighted in: exp at now, nbf at exactly the leeway
        exp_off in prop_oneof![Just(0i64), Just(1), Just(-1), -1000i64..1000],
        nbf_off in prop::option::of(prop_oneof![
            Just(access::NBF_LEEWAY as i64), Just(access::NBF_LEEWAY as i64 + 1), Just(access::NBF_LEEWAY as i64 - 1), -1000i64..1000,
        ]),
        human in any::<bool>(),
    ) {
        let (team, _) = rsa_keys();
        let exp = NOW.checked_add_signed(exp_off).unwrap();
        let mut claims = with(if human { human_claims() } else { ci_claims() }, "exp", json!(exp));
        claims = match nbf_off {
            Some(off) => with(claims, "nbf", json!(NOW.checked_add_signed(off).unwrap())),
            None => without(claims, "nbf"),
        };
        let got = access::verify(&token(&header(), &claims, Some(team)), &jwks(), &expect());
        let nbf_ok = nbf_off.is_none_or(|off| off <= access::NBF_LEEWAY as i64);
        if exp > NOW && nbf_ok {
            prop_assert_eq!(got.map(|i| i.kind), Ok(if human { Kind::Human } else { Kind::Ci }));
        } else {
            prop_assert!(got.is_err());
        }
    }

    /// Changing any one byte of a valid token gets it refused (a changed signature, header or payload breaks the RS256 check, or
    /// the shape), never accepted as somebody else.
    #[test]
    fn prop_any_changed_byte_is_refused(at in any::<prop::sample::Index>(), to in 0x21u8..0x7f) {
        let (team, _) = rsa_keys();
        let t = token(&header(), &ci_claims(), Some(team));
        let mut bytes = t.clone().into_bytes();
        let i = at.index(bytes.len());
        prop_assume!(bytes[i] != to);
        bytes[i] = to;
        let tampered = String::from_utf8(bytes).unwrap();
        // base64url's last character can carry unused bits: a change confined to them decodes to the same bytes. Then the token
        // is the same token, and accepting it is right.
        let same = tampered.split('.').map(|p| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, p).ok()).collect::<Vec<_>>()
            == t.split('.').map(|p| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, p).ok()).collect::<Vec<_>>();
        let got = access::verify(&tampered, &jwks(), &expect());
        if same {
            prop_assert!(got.is_ok());
        } else {
            prop_assert!(got.is_err(), "{}", tampered);
        }
    }

    /// Whatever text arrives, verify answers without panicking and accepts nothing.
    #[test]
    fn prop_arbitrary_text_is_never_accepted(s in prop_oneof![".{0,400}", "[A-Za-z0-9_-]{0,80}\\.[A-Za-z0-9_-]{0,80}\\.[A-Za-z0-9_-]{0,80}"]) {
        prop_assert!(access::verify(&s, &jwks(), &expect()).is_err());
    }
}

/// The JWKS parser keeps exactly the usable RSA-2048..4096 signing keys, whatever else the document holds.
#[rstest]
#[case::cloudflares_shape(None, 1)]
#[case::not_rsa(Some(("kty", json!("EC"))), 0)]
#[case::another_alg(Some(("alg", json!("RS512"))), 0)]
#[case::encryption_key(Some(("use", json!("enc"))), 0)]
#[case::no_kid(Some(("kid", serde_json::Value::Null)), 0)]
#[case::empty_kid(Some(("kid", json!(""))), 0)]
#[case::no_modulus(Some(("n", serde_json::Value::Null)), 0)]
#[case::modulus_not_base64(Some(("n", json!("!!!"))), 0)]
#[case::small_modulus(Some(("n", json!(b64(&[0xffu8; 128])))), 0)]
fn jwks_keys(#[case] change: Option<(&str, serde_json::Value)>, #[case] usable: usize) {
    let mut doc: serde_json::Value = serde_json::from_str(&jwks_doc(KID, &rsa_keys().0)).unwrap();
    if let Some((field, value)) = change {
        let key = &mut doc["keys"][0];
        if value.is_null() {
            key.as_object_mut().unwrap().remove(field);
        } else {
            key[field] = value;
        }
    }
    match access::parse_jwks(doc.to_string().as_bytes()) {
        Ok(keys) => assert_eq!(keys.len(), usable),
        Err(e) => {
            assert_eq!(usable, 0);
            assert_eq!(e, AccessError::NoKeys);
        }
    }
}

#[test]
fn a_jwks_document_that_is_not_one_is_refused() {
    assert_eq!(access::parse_jwks(b"not json").err(), Some(AccessError::Malformed));
    assert_eq!(access::parse_jwks(&vec![b' '; access::MAX_JWKS_LEN + 1]).err(), Some(AccessError::TooLong));
}

/// A key the team rotated in is found by its kid among several.
#[test]
fn the_key_named_by_kid_is_used_among_several() {
    let (team, attacker) = rsa_keys();
    let mut doc: serde_json::Value = serde_json::from_str(&jwks_doc("old", attacker)).unwrap();
    let new: serde_json::Value = serde_json::from_str(&jwks_doc(KID, team)).unwrap();
    doc["keys"].as_array_mut().unwrap().push(new["keys"][0].clone());
    let keys = access::parse_jwks(doc.to_string().as_bytes()).unwrap();
    assert_eq!(keys.len(), 2);
    assert!(access::verify(&token(&header(), &ci_claims(), Some(team)), &keys, &expect()).is_ok());
    // and a token claiming the old kid but signed by the team key is refused: the kid picks the key
    let t = token(&json!({ "alg": "RS256", "kid": "old" }), &ci_claims(), Some(team));
    assert_eq!(access::verify(&t, &keys, &expect()), Err(AccessError::BadSignature));
}

/// The size limits are the longest input read: exactly the limit is read (and refused for what it is), one byte more is not.
#[rstest]
fn the_size_limits_are_inclusive(#[values(-1i64, 0, 1)] delta: i64) {
    let token_len = usize::try_from(access::MAX_TOKEN_LEN as i64 + delta).unwrap();
    let got = access::verify(&"a".repeat(token_len), &jwks(), &expect());
    assert_eq!(got == Err(AccessError::TooLong), delta > 0, "token of {token_len} bytes: {got:?}");
    let doc_len = usize::try_from(access::MAX_JWKS_LEN as i64 + delta).unwrap();
    let got = access::parse_jwks(&vec![b' '; doc_len]).err();
    assert_eq!(got == Some(AccessError::TooLong), delta > 0, "JWKS of {doc_len} bytes: {got:?}");
}
