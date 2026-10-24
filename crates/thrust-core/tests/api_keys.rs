//! API keys (issue #38; ADR-0052 K1, ADR-0033): a high-entropy token with a visible prefix, shown once, stored only as
//! its SHA-256, accepted as `X-Api-Key`. Statuses reproduce the oracle (ADR-0033, round 27): only a MISSING key is
//! `Unauthenticated` (401); an unknown, revoked, expired or inactive-user key is `InvalidApiKey` (403), and the lookup
//! does not say which. Nothing here involves OIDC: the `oidc` port is absent from these worlds, and a key works.

mod common;

use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const OWNER: Actor = Actor::MemberA;

fn secs(n: i64) -> TimeDelta {
    TimeDelta::seconds(n)
}

/// The default world panics if anything is spawned; a key's first use spawns its `last_used_at` write, so these tests use
/// a background that only collects the work (the stamp has a test of its own).
fn quiet_world() -> World {
    let queue: Arc<dyn Queue> = Arc::new(RecordingQueue::default());
    world_with(CoreConfig { outbox_fastpath: false, ..CoreConfig::default() }, queue, Arc::new(DeferredBackground::default()))
}

fn key_credentials(key: Option<&str>) -> Credentials<'_> {
    Credentials { api_key: key, ..Credentials::default() }
}

#[derive(Debug, Clone, Copy)]
enum State {
    Live,
    NoExpiry,
    /// `expires_at` is the fixed clock's instant: a key is live only while `expires_at > now`.
    ExpiresNow,
    Expired,
    Revoked,
    OwnerInactive,
}

/// A world with one seeded key in `state` for the owner, plus a second live key of another user.
fn key_world(state: State) -> (World, &'static str) {
    const TOKEN: &str = "thrk_seeded-key-under-test";
    let w = quiet_world();
    let now = w.app.ports.clock.now();
    let expires_at = match state {
        State::Live | State::Revoked | State::OwnerInactive => Some(now + secs(3600)),
        State::NoExpiry => None,
        State::ExpiresNow => Some(now),
        State::Expired => Some(now - secs(3600)),
    };
    let key = |token: &str, actor: Actor, expires_at, revoked| SeedApiKey {
        token: token.into(),
        user_id: w.principal(actor).user_id,
        label: "seeded".into(),
        expires_at,
        revoked,
    };
    let fixture = SeedFixture {
        api_keys: vec![
            key(TOKEN, OWNER, expires_at, matches!(state, State::Revoked)),
            key("thrk_somebody-elses-key", Actor::GuestA, None, false),
        ],
        ..SeedFixture::default()
    };
    block_on(seed(&w.app, &fixture)).unwrap();
    if matches!(state, State::OwnerInactive) {
        assert!(block_on(set_user_active(&w.app, w.principal(OWNER).user_id, false)).unwrap());
    }
    (w, TOKEN)
}

#[rstest]
fn only_a_live_key_of_an_active_user_authenticates_and_every_other_state_is_403_not_401(
    #[values(State::Live, State::NoExpiry, State::ExpiresNow, State::Expired, State::Revoked, State::OwnerInactive)]
    state: State,
    #[values(true, false)] known: bool,
) {
    let (w, token) = key_world(state);
    let asked = if known { token } else { "thrk_never-issued" };
    let got = block_on(authenticate_request(&w.app, &key_credentials(Some(asked))));
    let live = known && matches!(state, State::Live | State::NoExpiry);
    let label = format!("{state:?} known={known}");
    if live {
        let a = got.unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(a.principal, w.principal(OWNER), "{label}");
        assert!(matches!(a.via, Via::ApiKey(_)), "{label}");
    } else {
        assert!(matches!(got, Err(CoreError::InvalidApiKey)), "{label}: {got:?}");
    }
}

#[test]
fn a_missing_key_is_the_only_401() {
    let (w, _) = key_world(State::Live);
    for credentials in [key_credentials(None), key_credentials(Some(""))] {
        let got = block_on(authenticate_request(&w.app, &credentials));
        assert_eq!(outcome(&got), Outcome::Unauthenticated, "{credentials:?}");
    }
    // control: a present, unknown key is not that
    let got = block_on(authenticate_request(&w.app, &key_credentials(Some("x"))));
    assert!(matches!(got, Err(CoreError::InvalidApiKey)));
}

/// Which credential counts when several arrive: a bearer token, then the key, then the cookie. The key is never
/// combined with a CSRF token's rules (it is not an ambient credential), and a key that fails is not rescued by a valid
/// cookie next to it.
#[test]
fn credential_precedence_is_bearer_then_key_then_cookie() {
    let (w, token) = key_world(State::Live);
    let who = w.principal(Actor::AdminA);
    let fixture = SeedFixture {
        sessions: vec![SeedSession { token: "sess".into(), user_id: who.user_id }],
        ..SeedFixture::default()
    };
    block_on(seed(&w.app, &fixture)).unwrap();
    let via = |c: Credentials<'_>| block_on(authenticate_request(&w.app, &c));
    let both = via(Credentials { bearer: Some("sess"), api_key: Some(token), ..Credentials::default() }).unwrap();
    assert_eq!((both.principal.clone(), matches!(both.via, Via::Session(_))), (who.clone(), true), "the bearer wins");
    let key_and_cookie =
        via(Credentials { api_key: Some(token), session_cookie: Some("sess"), mutating: true, ..Credentials::default() })
            .unwrap();
    assert_eq!(key_and_cookie.principal, w.principal(OWNER), "the key wins over the cookie, and a mutation needs no CSRF token");
    assert!(matches!(key_and_cookie.via, Via::ApiKey(_)));
    let bad_key_good_cookie =
        via(Credentials { api_key: Some("nope"), session_cookie: Some("sess"), ..Credentials::default() });
    assert!(matches!(bad_key_good_cookie, Err(CoreError::InvalidApiKey)), "{bad_key_good_cookie:?}");
}

// --- creating, listing, revoking --------------------------------------------------------------------------------------

#[test]
fn a_new_key_is_256_random_bits_with_a_visible_prefix_and_only_its_digest_is_stored() {
    let w = quiet_world();
    let who = w.principal(OWNER);
    let a = block_on(create_api_key(&w.app, &who, CreateApiKeyRequest { label: Some("ci".into()), expires_at: None })).unwrap();
    let b = block_on(create_api_key(&w.app, &who, CreateApiKeyRequest::default())).unwrap();
    assert_ne!(a.token, b.token);
    assert_ne!(a.id, b.id);
    // `thrk_` and 64 hex characters: 256 bits
    for k in [&a, &b] {
        let secret = k.token.strip_prefix("thrk_").expect("the visible marker");
        assert!(secret.len() == 64 && secret.bytes().all(|c| c.is_ascii_hexdigit()), "{}", k.token);
        assert_eq!(k.prefix, &k.token[..9], "the prefix is the marker and the first four characters, nothing more");
    }
    assert_eq!((a.label.as_str(), b.label.as_str()), ("ci", ""));
    // the database knows the token by its digest, and the prefix is not enough to find it
    let now = w.app.ports.clock.now();
    assert_eq!(block_on(w.db.api_key(&hash_token(&a.token), now)).unwrap().unwrap().id, a.id);
    assert!(block_on(w.db.api_key(&hash_token(&a.prefix), now)).unwrap().is_none());
    // the key works as an X-Api-Key at once
    let got = block_on(authenticate_request(&w.app, &key_credentials(Some(&a.token)))).unwrap();
    assert_eq!(got.principal, who);
    assert_eq!(got.via, Via::ApiKey(a.id));
    // listing shows what was created, without the token: nothing in it can be turned back into a credential
    let listed = block_on(list_api_keys(&w.app, &who)).unwrap();
    assert_eq!(listed.iter().map(|k| k.id).collect::<Vec<_>>(), [a.id, b.id]);
    let json = serde_json::to_string(&listed).unwrap();
    assert!(!json.contains(&a.token) && !json.contains(a.token.strip_prefix("thrk_").unwrap()), "{json}");
    assert!(json.contains(&a.prefix));
    // control: another user lists none of them
    assert!(block_on(list_api_keys(&w.app, &w.principal(Actor::GuestA))).unwrap().is_empty());
}

#[rstest]
fn creating_a_key_validates_its_label_and_expiry(
    #[values(None, Some("ok"), Some("x"), Some(""))] label: Option<&str>,
    #[values(-1, 0, 1, 3600)] expires_in: i64,
    #[values(true, false)] long_label: bool,
) {
    let w = quiet_world();
    let now = w.app.ports.clock.now();
    let label = if long_label { Some("y".repeat(101)) } else { label.map(str::to_string) };
    let got = block_on(create_api_key(
        &w.app,
        &w.principal(OWNER),
        CreateApiKeyRequest { label: label.clone(), expires_at: Some(now + secs(expires_in)) },
    ));
    // the expiry must be in the future; a label is at most 100 characters
    let want = if expires_in > 0 && !long_label { Outcome::Ok } else { Outcome::Invalid };
    assert_eq!(outcome(&got), want, "label={label:?} expires_in={expires_in}");
}

#[test]
fn revoking_is_by_owner_idempotent_and_takes_effect_on_the_next_request() {
    let w = quiet_world();
    let (owner, other) = (w.principal(OWNER), w.principal(Actor::GuestA));
    let a = block_on(create_api_key(&w.app, &owner, CreateApiKeyRequest::default())).unwrap();
    let b = block_on(create_api_key(&w.app, &owner, CreateApiKeyRequest::default())).unwrap();
    let live = |token: &str| block_on(authenticate_request(&w.app, &key_credentials(Some(token)))).is_ok();
    assert!(live(&a.token) && live(&b.token));

    // another user cannot revoke it, learns nothing but "not found", and the key lives on
    assert_eq!(outcome(&block_on(revoke_api_key(&w.app, &other, a.id))), Outcome::NotFound);
    assert!(live(&a.token), "someone else's revoke must not end the key");
    // an unknown id is not found either
    assert_eq!(outcome(&block_on(revoke_api_key(&w.app, &owner, ApiKeyId(Uuid::from_u128(9))))), Outcome::NotFound);

    block_on(revoke_api_key(&w.app, &owner, a.id)).unwrap();
    assert!(!live(&a.token), "a revoked key is rejected on the next request");
    assert!(live(&b.token), "control: the unrevoked key still works");
    // idempotent, and the listing says so
    block_on(revoke_api_key(&w.app, &owner, a.id)).unwrap();
    let listed = block_on(list_api_keys(&w.app, &owner)).unwrap();
    assert_eq!((listed[0].revoked_at.is_some(), listed[1].revoked_at.is_some()), (true, false));
}

/// `last_used_at` is written after the response, through `Background`, and not on every request.
#[test]
fn last_used_is_stamped_in_the_background_and_not_on_every_request() {
    let background = Arc::new(DeferredBackground::default());
    let queue: Arc<dyn Queue> = Arc::new(RecordingQueue::default());
    let w = world_with(CoreConfig { outbox_fastpath: false, ..CoreConfig::default() }, queue, background.clone());
    let who = w.principal(OWNER);
    let k = block_on(create_api_key(&w.app, &who, CreateApiKeyRequest::default())).unwrap();
    let used = || block_on(list_api_keys(&w.app, &who)).unwrap()[0].last_used_at;
    assert_eq!(used(), None, "never used");

    block_on(authenticate_request(&w.app, &key_credentials(Some(&k.token)))).unwrap();
    assert_eq!(used(), None, "the write is not part of the request");
    assert_eq!(background.run_all(), 1, "one background write");
    assert_eq!(used(), Some(w.app.ports.clock.now()));

    // a second use at the same instant: the stamp is fresh, so nothing is spawned
    block_on(authenticate_request(&w.app, &key_credentials(Some(&k.token)))).unwrap();
    assert_eq!(background.run_all(), 0, "no write per request");
    // a rejected key stamps nothing and spawns nothing
    let _ = block_on(authenticate_request(&w.app, &key_credentials(Some("thrk_nope"))));
    assert_eq!(background.run_all(), 0);
}

#[test]
fn seed_hashes_the_token_and_marks_a_revoked_key() {
    let (w, token) = key_world(State::Revoked);
    let now = w.app.ports.clock.now();
    assert!(block_on(w.db.api_key(&hash_token(token), now)).unwrap().is_none(), "revoked at seed");
    let keys = block_on(list_api_keys(&w.app, &w.principal(OWNER))).unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].revoked_at.is_some());
    assert_eq!(keys[0].prefix, &token[..9]);
    let _: &dyn Database = &w.db;
}
