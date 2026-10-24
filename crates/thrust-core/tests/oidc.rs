//! The OIDC authorization-code login (issue #38; ADR-0050), against a scripted provider: which values core stores and
//! sends, what it refuses before it talks to the provider at all, and who an accepted login may be.
//!
//! The spec, written out: `start` makes a `state`, a `nonce` and a PKCE verifier (256 random bits each), stores them
//! for ten minutes and hands them to the provider. `callback` first takes the stored login by the `state` it was
//! given (once: a replay finds nothing), refuses a state that is empty, unknown, replayed or expired WITHOUT calling
//! the provider, then asks the provider to exchange the code with the STORED verifier and nonce (the caller supplies
//! neither). What the provider vouches for must carry a verified email, or the login is refused. The account is the one
//! linked to `(issuer, sub)`; failing that, the active user with that email, who is then linked; failing that, nobody:
//! there is no just-in-time provisioning at P0 (Q-0050.1). An inactive user is refused.

mod common;

use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const LINKED: UserId = UserId(Uuid::from_u128(0xD1));
const BY_EMAIL: UserId = UserId(Uuid::from_u128(0xD2));
const INACTIVE: UserId = UserId(Uuid::from_u128(0xD3));

fn email_of(u: UserId) -> &'static str {
    match u {
        LINKED => "linked@example.test",
        BY_EMAIL => "byemail@example.test",
        _ => "inactive@example.test",
    }
}

/// Three users: one already linked to `(ISSUER, "sub-linked")`, one only known by email, one deactivated (with both).
fn oidc_world(cfg: CoreConfig, reply: OidcReply) -> (App, Arc<ScriptedOidc>, World) {
    let queue: Arc<dyn Queue> = Arc::new(RecordingQueue::default());
    let w = world_with(cfg, queue, Arc::new(DeferredBackground::default()));
    let provider = ScriptedOidc::new(reply);
    let mut app = w.app.clone();
    app.ports.oidc = Some(provider.clone());
    let login = |user_id, is_active| SeedLogin { user_id, email: email_of(user_id).into(), password: None, is_active };
    let link = |user_id, sub: &str| SeedAccount {
        user_id,
        provider: ScriptedOidc::ISSUER.into(),
        provider_account_id: sub.into(),
    };
    let fixture = SeedFixture {
        logins: vec![login(LINKED, true), login(BY_EMAIL, true), login(INACTIVE, false)],
        accounts: vec![link(LINKED, "sub-linked"), link(INACTIVE, "sub-inactive")],
        ..SeedFixture::default()
    };
    block_on(seed(&app, &fixture)).unwrap();
    (app, provider, w)
}

fn default_cfg() -> CoreConfig {
    CoreConfig { outbox_fastpath: false, ..CoreConfig::default() }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn start_stores_state_nonce_and_verifier_and_hands_them_to_the_provider() {
    let (app, provider, _w) = oidc_world(default_cfg(), OidcReply::Down);
    let url = block_on(oidc_start(&app)).unwrap();
    let url2 = block_on(oidc_start(&app)).unwrap();
    assert_ne!(url, url2, "every login has its own state");
    let started = provider.started.lock().unwrap().clone();
    assert_eq!(started.len(), 2);
    let (state, nonce, verifier) = &started[0];
    assert_eq!(url, format!("{}/authorize?state={state}", ScriptedOidc::ISSUER), "the provider's URL is what the caller gets");
    // 256 random bits each, and three different values (a PKCE verifier is 43 to 128 characters)
    assert!(is_hex64(state) && is_hex64(nonce) && is_hex64(verifier), "{started:?}");
    assert!(state != nonce && nonce != verifier && state != verifier);
    // the database holds what the callback will need, under the state's digest, for ten minutes, and not the state
    let now = app.ports.clock.now();
    let row = block_on(app.ports.db.take_oidc_login(&hash_token(state))).unwrap().expect("a stored login");
    assert_eq!((row.nonce.as_str(), row.code_verifier.as_str()), (nonce.as_str(), verifier.as_str()));
    assert_eq!(row.expires_at, now + thrust_contracts::chrono::TimeDelta::seconds(600));
    assert!(block_on(app.ports.db.take_oidc_login(&TokenHash([0; 32]))).unwrap().is_none());
}

#[derive(Debug, Clone, Copy)]
enum StateIs {
    Valid,
    /// One character changed.
    Tampered,
    Unknown,
    Empty,
    /// The same login's callback made twice: this is the second.
    Replayed,
    /// Started under a lifetime that had already run out.
    Expired,
}

#[derive(Debug, Clone, Copy)]
enum Vouched {
    /// A verified email, and the provider's subject is linked to a user.
    LinkedBySub,
    /// A verified email of a user who has no link yet.
    ByEmail,
    Unverified,
    NoEmail,
    /// Verified, but no user has that email and the subject is unknown: no JIT.
    NoSuchUser,
    /// The subject is linked to a deactivated user.
    InactiveUser,
    /// The provider refuses the exchange (a bad code, a failed PKCE check, an ID token that does not verify).
    ProviderRejects,
    ProviderDown,
}

impl Vouched {
    fn reply(self) -> OidcReply {
        let c = ScriptedOidc::claims;
        match self {
            Vouched::LinkedBySub => c("sub-linked", Some(email_of(LINKED)), true),
            Vouched::ByEmail => c("sub-new", Some(&format!("  {}  ", email_of(BY_EMAIL).to_uppercase())), true),
            Vouched::Unverified => c("sub-new", Some(email_of(BY_EMAIL)), false),
            Vouched::NoEmail => c("sub-new", None, true),
            Vouched::NoSuchUser => c("sub-stranger", Some("stranger@example.test"), true),
            Vouched::InactiveUser => c("sub-inactive", Some(email_of(INACTIVE)), true),
            Vouched::ProviderRejects => OidcReply::Rejected,
            Vouched::ProviderDown => OidcReply::Down,
        }
    }
}

#[rstest]
fn the_callback_follows_the_spec_for_every_state_and_every_answer(
    #[values(StateIs::Valid, StateIs::Tampered, StateIs::Unknown, StateIs::Empty, StateIs::Replayed, StateIs::Expired)]
    state_is: StateIs,
    #[values(
        Vouched::LinkedBySub,
        Vouched::ByEmail,
        Vouched::Unverified,
        Vouched::NoEmail,
        Vouched::NoSuchUser,
        Vouched::InactiveUser,
        Vouched::ProviderRejects,
        Vouched::ProviderDown
    )]
    vouched: Vouched,
) {
    let ttl = if matches!(state_is, StateIs::Expired) { -1 } else { 600 };
    let (app, provider, _w) = oidc_world(CoreConfig { oidc_login_ttl_secs: ttl, ..default_cfg() }, vouched.reply());
    block_on(oidc_start(&app)).unwrap();
    let (state, nonce, verifier) = provider.started.lock().unwrap()[0].clone();
    let sent_state = match state_is {
        StateIs::Valid | StateIs::Replayed | StateIs::Expired => state.clone(),
        StateIs::Tampered => format!("{}{}", &state[..63], if state.ends_with('0') { '1' } else { '0' }),
        StateIs::Unknown => "f".repeat(64),
        StateIs::Empty => String::new(),
    };
    if matches!(state_is, StateIs::Replayed) {
        // the first callback with the right state: whatever it came to, it used the login up
        let _ = block_on(oidc_callback(&app, "code-1", &state));
        provider.exchanged.lock().unwrap().clear();
    }
    let got = block_on(oidc_callback(&app, "the-code", &sent_state));
    let label = format!("{state_is:?} / {vouched:?}");

    let state_ok = matches!(state_is, StateIs::Valid);
    let want = if !state_ok {
        Outcome::Invalid
    } else {
        match vouched {
            Vouched::LinkedBySub | Vouched::ByEmail => Outcome::Ok,
            Vouched::Unverified | Vouched::NoEmail | Vouched::NoSuchUser | Vouched::InactiveUser => Outcome::Forbidden,
            Vouched::ProviderRejects => Outcome::Unauthenticated,
            Vouched::ProviderDown => Outcome::Other(CoreError::Upstream("scripted".into()).to_string()),
        }
    };
    let got_outcome = match &got {
        Err(CoreError::Upstream(_)) => Outcome::Other(CoreError::Upstream("scripted".into()).to_string()),
        other => outcome(other),
    };
    assert_eq!(got_outcome, want, "{label}");

    // a bad state never reaches the provider; a good one does, with the STORED verifier and nonce and the caller's code
    let exchanged = provider.exchanged.lock().unwrap().clone();
    if state_ok {
        assert_eq!(exchanged, [("the-code".to_string(), verifier, nonce)], "{label}");
    } else {
        assert!(exchanged.is_empty(), "{label}: the provider was called for a state that does not check out");
    }
    // only an accepted login makes a session, and it is the right user's
    let sessions = |u| block_on(sign_out_everywhere(&app, &Principal { user_id: u })).unwrap();
    let (linked, by_email, inactive) = (sessions(LINKED), sessions(BY_EMAIL), sessions(INACTIVE));
    // (a replay's FIRST callback was a valid one, so it made its session; the replay itself must have made none)
    let first_was_valid = matches!(state_is, StateIs::Valid | StateIs::Replayed);
    let expected = match (first_was_valid, vouched) {
        (true, Vouched::LinkedBySub) => (1, 0, 0),
        (true, Vouched::ByEmail) => (0, 1, 0),
        _ => (0, 0, 0),
    };
    assert_eq!((linked, by_email, inactive), expected, "{label}: sessions made");
    if let Ok(s) = &got {
        assert_eq!(s.user_id, if matches!(vouched, Vouched::LinkedBySub) { LINKED } else { BY_EMAIL }, "{label}");
    }
}

/// The first login of a user known only by email links the subject; from then on the subject decides, whatever email
/// the provider reports (a changed or someone else's email cannot take over the account).
#[test]
fn a_login_by_email_links_the_subject_and_the_link_then_decides() {
    let (app, provider, _w) = oidc_world(default_cfg(), Vouched::ByEmail.reply());
    let login = |app: &App| {
        block_on(oidc_start(app)).unwrap();
        let state = provider.started.lock().unwrap().last().unwrap().0.clone();
        block_on(oidc_callback(app, "c", &state))
    };
    assert!(block_on(app.ports.db.user_for_account(ScriptedOidc::ISSUER, "sub-new")).unwrap().is_none(), "not linked yet");
    assert_eq!(login(&app).unwrap().user_id, BY_EMAIL);
    let linked = block_on(app.ports.db.user_for_account(ScriptedOidc::ISSUER, "sub-new")).unwrap();
    assert_eq!(linked.map(|u| u.id), Some(BY_EMAIL), "the subject is linked after the first login");

    // the same subject now reports another user's verified email: it is still the linked user
    *provider.reply.lock().unwrap() = ScriptedOidc::claims("sub-new", Some(email_of(LINKED)), true);
    assert_eq!(login(&app).unwrap().user_id, BY_EMAIL, "the link beats the email");
    // control: a different subject with that email is a different login, and links to that user
    *provider.reply.lock().unwrap() = ScriptedOidc::claims("sub-other", Some(email_of(LINKED)), true);
    assert_eq!(login(&app).unwrap().user_id, LINKED);
}

/// With no provider configured the login routes do not exist, and nothing else is affected: a password or an API key
/// signs in exactly as before (ADR-0050 Confirmation: an API key works without OIDC).
#[test]
fn without_a_provider_the_login_is_not_found_and_a_key_still_works() {
    let queue: Arc<dyn Queue> = Arc::new(RecordingQueue::default());
    let w = world_with(default_cfg(), queue, Arc::new(DeferredBackground::default()));
    assert!(w.app.ports.oidc.is_none());
    assert_eq!(outcome(&block_on(oidc_start(&w.app))), Outcome::NotFound);
    assert_eq!(outcome(&block_on(oidc_callback(&w.app, "c", "s"))), Outcome::NotFound);
    let who = w.principal(Actor::MemberA);
    let key = block_on(create_api_key(&w.app, &who, CreateApiKeyRequest::default())).unwrap();
    let c = Credentials { api_key: Some(&key.token), ..Credentials::default() };
    assert_eq!(block_on(authenticate_request(&w.app, &c)).unwrap().principal, who);
}
