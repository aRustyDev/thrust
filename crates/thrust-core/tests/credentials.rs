//! Which credential a request is authenticated with, and when a cookie-authenticated mutation needs the CSRF token
//! (issue #38; ADR-0052). The rules, written out as the spec the tests own:
//!
//! * a bearer token, when the request has one, is the only credential considered: valid is a session, anything else
//!   is `Unauthenticated`, and the cookie and the CSRF token are not looked at (a script that sets `Authorization`
//!   is not riding an ambient cookie, so there is nothing to forge);
//! * otherwise the session cookie counts: an unknown, revoked or expired one is `Unauthenticated` whatever else the
//!   request carries; a valid one authenticates a safe method outright, and a mutation only when the CSRF header
//!   equals the CSRF cookie (both present, non-empty, constant-time compared), while enforcement is on;
//! * with enforcement off (`CoreConfig::csrf_enforce`, Q-0050.8: off in the composition roots while the stock frontend
//!   is in use) the token is not consulted: a valid cookie authenticates a mutation whatever the CSRF header and cookie
//!   say, and nothing else in this list changes (a dead cookie is still `Unauthenticated`, the bearer token is still
//!   the only credential considered when there is one);
//! * no credential at all is `Unauthenticated`.

mod common;

use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;

const LIVE: &str = "live-session-token";
const REVOKED: &str = "revoked-session-token";
const UNKNOWN: &str = "never-issued";

/// The world the credential tests run in, with CSRF enforcement `enforce`.
fn credential_world(enforce: bool) -> World {
    let w = world_config(CoreConfig { outbox_fastpath: false, csrf_enforce: enforce, ..CoreConfig::default() });
    let who = w.principal(Actor::MemberA).user_id;
    let fixture = SeedFixture {
        sessions: vec![
            SeedSession { token: LIVE.into(), user_id: who },
            SeedSession { token: REVOKED.into(), user_id: who },
        ],
        ..SeedFixture::default()
    };
    block_on(seed(&w.app, &fixture)).unwrap();
    block_on(revoke_session(&w.app, REVOKED)).unwrap();
    w
}

#[derive(Debug, Clone, Copy)]
enum Cred {
    Nothing,
    BearerLive,
    BearerUnknown,
    CookieLive,
    CookieRevoked,
    CookieUnknown,
    /// A valid bearer next to an unusable cookie: the bearer is the credential.
    BearerLiveCookieUnknown,
    /// An unusable bearer next to a valid cookie: the bearer is still the credential, and it fails.
    BearerUnknownCookieLive,
}

impl Cred {
    fn bearer(self) -> Option<&'static str> {
        match self {
            Cred::BearerLive | Cred::BearerLiveCookieUnknown => Some(LIVE),
            Cred::BearerUnknown | Cred::BearerUnknownCookieLive => Some(UNKNOWN),
            _ => None,
        }
    }
    fn cookie(self) -> Option<&'static str> {
        match self {
            Cred::CookieLive | Cred::BearerUnknownCookieLive => Some(LIVE),
            Cred::CookieRevoked => Some(REVOKED),
            Cred::CookieUnknown | Cred::BearerLiveCookieUnknown => Some(UNKNOWN),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Csrf {
    Absent,
    Matching,
    Mismatching,
    CookieOnly,
    HeaderOnly,
    BothEmpty,
}

impl Csrf {
    fn cookie(self) -> Option<&'static str> {
        match self {
            Csrf::Matching | Csrf::CookieOnly => Some("t0ken"),
            Csrf::Mismatching => Some("t0ken"),
            Csrf::BothEmpty => Some(""),
            _ => None,
        }
    }
    fn header(self) -> Option<&'static str> {
        match self {
            Csrf::Matching | Csrf::HeaderOnly => Some("t0ken"),
            Csrf::Mismatching => Some("other"),
            Csrf::BothEmpty => Some(""),
            _ => None,
        }
    }
    /// The spec: the token is accepted only when both halves are there, are not empty, and are equal.
    fn accepted(self) -> bool {
        matches!(self, Csrf::Matching)
    }
}

#[rstest]
fn the_credential_and_the_csrf_rule_decide_the_outcome(
    #[values(
        Cred::Nothing,
        Cred::BearerLive,
        Cred::BearerUnknown,
        Cred::CookieLive,
        Cred::CookieRevoked,
        Cred::CookieUnknown,
        Cred::BearerLiveCookieUnknown,
        Cred::BearerUnknownCookieLive
    )]
    cred: Cred,
    #[values(false, true)] mutating: bool,
    #[values(Csrf::Absent, Csrf::Matching, Csrf::Mismatching, Csrf::CookieOnly, Csrf::HeaderOnly, Csrf::BothEmpty)]
    csrf: Csrf,
    #[values(true, false)] enforce: bool,
) {
    let w = credential_world(enforce);
    let c = Credentials {
        bearer: cred.bearer(),
        api_key: None,
        session_cookie: cred.cookie(),
        csrf_cookie: csrf.cookie(),
        csrf_header: csrf.header(),
        mutating,
    };
    let got = block_on(authenticate_request(&w.app, &c));
    let want = match (cred.bearer(), cred.cookie()) {
        (Some(t), _) => (t == LIVE).then_some(t).ok_or(Outcome::Unauthenticated),
        (None, Some(t)) if t == LIVE => {
            if mutating && enforce && !csrf.accepted() {
                Err(Outcome::Other("csrf token missing or wrong".into()))
            } else {
                Ok(t)
            }
        }
        _ => Err(Outcome::Unauthenticated),
    };
    let label = format!("{cred:?} mutating={mutating} {csrf:?} enforce={enforce}");
    match (got, want) {
        (Ok(a), Ok(token)) => {
            assert_eq!(a.principal, w.principal(Actor::MemberA), "{label}");
            assert_eq!(a.via, Via::Session(hash_token(token)), "{label}");
        }
        (got, Err(want)) => assert_eq!(outcome(&got), want, "{label}"),
        (got, Ok(_)) => panic!("{label}: got {got:?}, want a session"),
    }
}

// --- the token comparison -----------------------------------------------------------------------------------------

#[rstest]
fn csrf_ok_requires_both_halves_non_empty_and_equal(
    #[values(None, Some(""), Some("a"), Some("b"), Some("aa"))] cookie: Option<&str>,
    #[values(None, Some(""), Some("a"), Some("b"), Some("aa"))] header: Option<&str>,
) {
    let want = matches!((cookie, header), (Some(c), Some(h)) if !c.is_empty() && c == h);
    assert_eq!(csrf_ok(cookie, header), want, "cookie={cookie:?} header={header:?}");
}

/// One byte flipped anywhere, one byte more or one byte fewer are all different; equal is equal.
fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    prop::collection::vec(any::<u8>(), 0..64).prop_flat_map(|a| {
        let len = a.len();
        let same = Just((a.clone(), a.clone()));
        let flipped = (0..len.max(1), 1..=255u8).prop_map({
            let a = a.clone();
            move |(i, x)| {
                let mut b = a.clone();
                if let Some(slot) = b.get_mut(i) {
                    *slot ^= x;
                }
                (a.clone(), b)
            }
        });
        let longer = Just({
            let mut b = a.clone();
            b.push(0);
            (a.clone(), b)
        });
        let shorter = Just((a.clone(), a[..len.saturating_sub(1)].to_vec()));
        let random = prop::collection::vec(any::<u8>(), 0..64).prop_map(move |b| (a.clone(), b));
        prop_oneof![2 => same, 2 => flipped, 1 => longer, 1 => shorter, 1 => random]
    })
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_ct_eq_agrees_with_eq((a, b) in pair()) {
        prop_assert_eq!(ct_eq(&a, &b), a == b);
    }
}

/// The rule as ADR-0052 wrote it is the core's default; the composition roots turn it off (and say so in `/_info`).
#[test]
fn enforcement_is_on_unless_a_root_turns_it_off() {
    assert!(CoreConfig::default().csrf_enforce);
}

/// What turning enforcement off does not touch: the token endpoint's token, the comparison, and the two credentials that
/// never needed it. A token that is presented is still validated by `csrf_ok` (it is only not asked for), and a bearer
/// token or an API key is exactly as before.
#[rstest]
fn turning_enforcement_off_changes_nothing_but_the_cookie_mutation(#[values(true, false)] enforce: bool) {
    let w = credential_world(enforce);
    let c = |bearer, cookie| Credentials {
        bearer,
        api_key: None,
        session_cookie: cookie,
        csrf_cookie: None,
        csrf_header: None,
        mutating: true,
    };
    // a bearer token, with no token: accepted either way; a dead bearer: refused either way
    assert!(block_on(authenticate_request(&w.app, &c(Some(LIVE), None))).is_ok());
    assert_eq!(outcome(&block_on(authenticate_request(&w.app, &c(Some(UNKNOWN), None)))), Outcome::Unauthenticated);
    // a dead cookie is refused either way, whatever it carries
    assert_eq!(outcome(&block_on(authenticate_request(&w.app, &c(None, Some(UNKNOWN))))), Outcome::Unauthenticated);
    // the cookie mutation with no token is the one thing that differs
    let got = block_on(authenticate_request(&w.app, &c(None, Some(LIVE))));
    assert_eq!(got.is_ok(), !enforce, "enforce={enforce}: {got:?}");
    // the token's own code does not look at the switch
    assert!(csrf_ok(Some("t0ken"), Some("t0ken")));
    assert!(!csrf_ok(Some("t0ken"), Some("other")));
    assert!(new_csrf_token(&w.app).is_ok());
}

#[test]
fn a_csrf_token_is_128_random_bits_and_different_every_time() {
    let w = credential_world(true);
    let a = new_csrf_token(&w.app).unwrap();
    let b = new_csrf_token(&w.app).unwrap();
    assert_ne!(a, b);
    assert_eq!((a.len(), b.len()), (32, 32), "16 bytes, hex");
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    let mut app = w.app.clone();
    app.ports.rng = Arc::new(FailingRng);
    assert!(matches!(new_csrf_token(&app), Err(CoreError::Internal(_))));
}
