//! Sign-in and the session lifecycle (issue #38), against the memory adapters with the real argon2id at a cheap cost
//! and a hasher that counts its work.
//!
//! What must hold: a sign-in succeeds exactly for a known, active user with a password and that password, and every
//! other attempt gets the same answer; every attempt costs exactly one unit of hash work, so the time a sign-in takes
//! says nothing about whether the email exists (counted, not timed); every sign-in makes a fresh high-entropy session
//! whose token the database never sees; sign-out reaches one session and sign-out-everywhere all of a user's; the cost
//! parameters are OWASP's minimum unless configured, and a hash encodes them.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const PASSWORD: &str = "correct horse battery staple";
const ACTIVE: UserId = UserId(Uuid::from_u128(0xC1));
const INACTIVE: UserId = UserId(Uuid::from_u128(0xC2));
const NO_PASSWORD: UserId = UserId(Uuid::from_u128(0xC3));
const OTHER: UserId = UserId(Uuid::from_u128(0xC4));

/// The email each seeded user is stored under after normalisation (the fixture spells two of them differently).
fn email_of(u: UserId) -> &'static str {
    match u {
        ACTIVE => "active@example.test",
        INACTIVE => "inactive@example.test",
        NO_PASSWORD => "nopw@example.test",
        _ => "other@example.test",
    }
}

/// A world with four login users: one active with a password, one deactivated with one, one active without, and a
/// second active user for the "somebody else" cases. The hash counter is zero when it returns.
fn auth_world() -> World {
    let w = world();
    let login = |user_id, email: &str, password: Option<&str>, is_active| SeedLogin {
        user_id,
        email: email.into(),
        password: password.map(str::to_string),
        is_active,
    };
    let fixture = SeedFixture {
        logins: vec![
            // seeded with a capital and padding: the core stores it normalised
            login(ACTIVE, "  Active@Example.TEST ", Some(PASSWORD), true),
            login(INACTIVE, "inactive@example.test", Some(PASSWORD), false),
            login(NO_PASSWORD, "nopw@example.test", None, true),
            login(OTHER, "other@example.test", Some("another password"), true),
        ],
        ..SeedFixture::default()
    };
    block_on(seed(&w.app, &fixture)).unwrap();
    w.hasher.reset();
    w
}

#[derive(Debug, Clone, Copy)]
enum Email {
    Known,
    OtherCase,
    Padded,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
enum Pw {
    Right,
    Wrong,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Active,
    Inactive,
    NoPassword,
}

impl Who {
    fn id(self) -> UserId {
        match self {
            Who::Active => ACTIVE,
            Who::Inactive => INACTIVE,
            Who::NoPassword => NO_PASSWORD,
        }
    }
}

/// The spec, written out: only a known, active user with a password, asked for with that password, signs in; an empty
/// password is a client error before anything is hashed; everything else is the one answer for a failed sign-in.
#[rstest]
fn sign_in_succeeds_exactly_for_an_active_user_with_the_right_password_and_costs_one_hash_either_way(
    #[values(Email::Known, Email::OtherCase, Email::Padded, Email::Unknown)] email: Email,
    #[values(Pw::Right, Pw::Wrong, Pw::Empty)] pw: Pw,
    #[values(Who::Active, Who::Inactive, Who::NoPassword)] who: Who,
) {
    let w = auth_world();
    let base = email_of(who.id());
    let asked = match email {
        Email::Known => base.to_string(),
        Email::OtherCase => base.to_uppercase(),
        Email::Padded => format!("  {base}\t"),
        Email::Unknown => "nobody@example.test".to_string(),
    };
    let password = match pw {
        Pw::Right => PASSWORD,
        Pw::Wrong => "not the password",
        Pw::Empty => "",
    };
    let got = block_on(sign_in(&w.app, &asked, password));
    let label = format!("{email:?} {pw:?} {who:?}");
    let known = !matches!(email, Email::Unknown);
    let want = match pw {
        Pw::Empty => Outcome::Invalid,
        _ if known && matches!(pw, Pw::Right) && who == Who::Active => Outcome::Ok,
        _ => Outcome::Unauthenticated,
    };
    assert_eq!(outcome(&got), want, "{label}");
    let work = if matches!(pw, Pw::Empty) { 0 } else { 1 };
    assert_eq!(w.hasher.count(), work, "{label}: units of hash work (an unknown email must cost what a wrong password does)");
    if let Ok(s) = got {
        assert_eq!(s.user_id, ACTIVE, "{label}");
    }
}

/// Every failure is the same value, not merely the same status: nothing in the answer tells the cases apart.
#[test]
fn every_failed_sign_in_is_the_same_error() {
    let w = auth_world();
    let attempts = [
        ("active@example.test", "wrong"),
        ("nobody@example.test", PASSWORD),
        ("inactive@example.test", PASSWORD),
        ("nopw@example.test", PASSWORD),
    ];
    let errors: BTreeSet<String> =
        attempts.iter().map(|(e, p)| format!("{:?}", block_on(sign_in(&w.app, e, p)).unwrap_err())).collect();
    assert_eq!(errors.len(), 1, "failed sign-ins differ: {errors:?}");
}

/// A sign-in that is refused for input before any hashing is a client error for known and unknown emails alike.
#[rstest]
fn malformed_input_is_refused_before_hashing(
    #[values("", "   ")] email: &str,
    #[values(true, false)] too_long_password: bool,
) {
    let w = auth_world();
    let password = if too_long_password { "x".repeat(1025) } else { PASSWORD.to_string() };
    let got = block_on(sign_in(&w.app, email, &password));
    assert_eq!(outcome(&got), Outcome::Invalid);
    assert_eq!(w.hasher.count(), 0);
    // control: the same password with a real email is hashed (it is the email that was refused)
    if !too_long_password {
        assert_eq!(outcome(&block_on(sign_in(&w.app, "active@example.test", &password))), Outcome::Ok);
        assert_eq!(w.hasher.count(), 1);
    }
    // control: an over-long password is refused whoever asks
    if too_long_password {
        assert_eq!(outcome(&block_on(sign_in(&w.app, "active@example.test", &password))), Outcome::Invalid);
        assert_eq!(w.hasher.count(), 0);
    }
}

#[test]
fn a_sign_in_mints_a_fresh_session_whose_token_the_database_never_sees() {
    let w = auth_world();
    let a = block_on(sign_in(&w.app, "active@example.test", PASSWORD)).unwrap();
    let b = block_on(sign_in(&w.app, "active@example.test", PASSWORD)).unwrap();
    // fresh each time: no session fixation, and the two tokens and CSRF tokens differ
    assert_ne!(a.token, b.token);
    assert_ne!(a.csrf_token, b.csrf_token);
    // at least 128 bits of CSPRNG output (#36's requirement); the session token is 256, hex-encoded, and the CSRF token
    // is 128
    let is_hex = |s: &str| s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    assert_eq!((a.token.len(), a.csrf_token.len()), (64, 32));
    assert!(is_hex(&a.token) && is_hex(&a.csrf_token));
    assert_ne!(a.token, a.csrf_token);
    // the row is there under the token's digest, and live, for 30 days from now
    let now = w.app.ports.clock.now();
    assert_eq!(a.expires_at, now + TimeDelta::seconds(w.app.cfg.session_ttl_secs));
    assert_eq!(block_on(authenticate(&w.app, &a.token)).unwrap(), Principal { user_id: ACTIVE });
    let live = block_on(w.db.session(&hash_token(&a.token), now)).unwrap().expect("a row under the digest");
    assert_eq!(live.user_id, ACTIVE);
    // control: the token itself is not a key: its raw bytes are not a session
    let raw = TokenHash(<[u8; 32]>::try_from(a.token.as_bytes()[..32].to_vec()).unwrap());
    assert!(block_on(w.db.session(&raw, now)).unwrap().is_none());
}

#[test]
fn a_random_source_that_fails_makes_no_session_and_is_not_an_authentication_answer() {
    let w = auth_world();
    let mut app = w.app.clone();
    app.ports.rng = Arc::new(FailingRng);
    let got = block_on(sign_in(&app, "active@example.test", PASSWORD));
    assert!(matches!(got, Err(CoreError::Internal(_))), "{got:?}");
    // control: with the real stream the same sign-in works, so the failure above was the source
    assert!(block_on(sign_in(&w.app, "active@example.test", PASSWORD)).is_ok());
}

#[test]
fn sign_out_reaches_one_session_and_sign_out_everywhere_all_of_a_users_and_nobody_elses() {
    let w = auth_world();
    let login = |email: &str, pw: &str| block_on(sign_in(&w.app, email, pw)).unwrap();
    let (a1, a2, a3) = (login("active@example.test", PASSWORD), login("active@example.test", PASSWORD), login("active@example.test", PASSWORD));
    let o1 = login("other@example.test", "another password");
    let live = |s: &SignedIn| block_on(authenticate(&w.app, &s.token)).is_ok();
    assert!([&a1, &a2, &a3, &o1].into_iter().all(live), "all four are live to start with");

    block_on(revoke_session(&w.app, &a1.token)).unwrap();
    assert_eq!([live(&a1), live(&a2), live(&a3), live(&o1)], [false, true, true, true], "sign-out is one session");
    // signing out twice, or a token that was never issued, changes nothing
    block_on(revoke_session(&w.app, &a1.token)).unwrap();
    block_on(revoke_session(&w.app, "never-issued")).unwrap();
    assert_eq!([live(&a2), live(&a3), live(&o1)], [true, true, true]);

    let revoked = block_on(sign_out_everywhere(&w.app, &Principal { user_id: ACTIVE })).unwrap();
    assert_eq!(revoked, 2, "the two sessions that were still live");
    assert_eq!([live(&a1), live(&a2), live(&a3), live(&o1)], [false, false, false, true], "the other user is untouched");
    assert_eq!(block_on(sign_out_everywhere(&w.app, &Principal { user_id: ACTIVE })).unwrap(), 0);
    // control: signing in again after sign-out-everywhere works, and the new session is live
    assert!(live(&login("active@example.test", PASSWORD)));
}

#[test]
fn deactivating_the_user_ends_their_sessions_at_once_and_reactivating_brings_them_back() {
    let w = auth_world();
    let s = block_on(sign_in(&w.app, "active@example.test", PASSWORD)).unwrap();
    let live = || block_on(authenticate(&w.app, &s.token)).is_ok();
    assert!(live());
    assert!(block_on(set_user_active(&w.app, ACTIVE, false)).unwrap());
    assert!(!live(), "a deactivated user's session is rejected");
    // and they cannot sign in again
    assert_eq!(outcome(&block_on(sign_in(&w.app, "active@example.test", PASSWORD))), Outcome::Unauthenticated);
    assert!(block_on(set_user_active(&w.app, ACTIVE, true)).unwrap());
    assert!(live(), "reactivated");
    assert!(!block_on(set_user_active(&w.app, UserId(Uuid::from_u128(0xDEAD)), false)).unwrap(), "no such user");
}

// --- the cost parameters ------------------------------------------------------------------------------------------

/// The default cost is OWASP's minimum (m = 19 MiB, t = 2, p = 1), and that is what a hash made under it encodes: the
/// one test that runs the real cost. (A pinned spec value: OWASP Password Storage Cheat Sheet.)
#[test]
fn the_default_cost_is_owasps_minimum_and_the_hash_encodes_it() {
    assert_eq!(CoreConfig::default().argon2, Argon2Params { m_cost_kib: 19_456, t_cost: 2, p_cost: 1 });
    let hasher = Argon2idHasher::new(CoreConfig::default().argon2);
    let phc = hasher.hash("pw", &[7u8; 16]).unwrap();
    assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
    assert!(hasher.verify("pw", &phc));
    assert!(!hasher.verify("pw2", &phc));
}

/// A configured cost is what new hashes carry, and a hash made under another cost still verifies (the cost is read
/// from the string), so raising the default later does not lock anyone out.
#[rstest]
fn a_hash_carries_the_cost_it_was_made_under_and_verifies_under_any_configured_cost(
    #[values(8, 32, 64)] m: u32,
    #[values(1, 2)] t: u32,
    #[values(1, 2)] p: u32,
) {
    let made = Argon2idHasher::new(Argon2Params { m_cost_kib: m.max(8 * p), t_cost: t, p_cost: p });
    let phc = made.hash("pw", &[1u8; 16]).unwrap();
    assert!(phc.contains(&format!("m={},t={},p={}$", m.max(8 * p), t, p)), "{phc}");
    let other = Argon2idHasher::new(Argon2Params { m_cost_kib: 64, t_cost: 3, p_cost: 1 });
    assert!(other.verify("pw", &phc));
    assert!(!other.verify("not pw", &phc));
}

proptest! {
    #![proptest_config(proptest_config(32))]

    /// Any password round-trips through any salt, and only that password does.
    #[test]
    fn prop_a_hash_verifies_its_password_and_no_other(
        pw in "\\PC{1,40}",
        other in "\\PC{1,40}",
        salt in prop::collection::vec(any::<u8>(), 8..32),
    ) {
        let h = CountingHasher::new();
        let phc = h.hash(&pw, &salt).unwrap();
        prop_assert!(h.verify(&pw, &phc));
        prop_assert_eq!(h.verify(&other, &phc), other == pw);
    }

    /// A string that is not a PHC hash never matches, and never panics.
    #[test]
    fn prop_a_string_that_is_not_a_hash_never_verifies(pw in "\\PC{0,20}", junk in "\\PC{0,80}") {
        prop_assume!(!junk.starts_with("$argon2"));
        prop_assert!(!CountingHasher::new().verify(&pw, &junk));
    }
}

// --- rate limiting -------------------------------------------------------------------------------------------------

/// The limiter is per email and runs before any hashing: past the limit even the right password is refused, at no hash
/// cost, and another email is not affected. (A fixed clock: one window, deterministic.)
#[rstest]
fn sign_in_is_limited_per_email_before_any_hashing(#[values(true, false)] known: bool) {
    let w = auth_world();
    let limit = w.app.cfg.sign_in_rate_limit;
    let email = if known { "active@example.test" } else { "nobody@example.test" };
    for i in 0..limit {
        let got = block_on(sign_in(&w.app, email, "wrong"));
        assert_eq!(outcome(&got), Outcome::Unauthenticated, "attempt {i} of {limit} is answered, not limited");
    }
    assert_eq!(w.hasher.count(), u64::from(limit));
    let got = block_on(sign_in(&w.app, email, PASSWORD));
    assert!(matches!(got, Err(CoreError::RateLimited { .. })), "attempt {} must be limited: {got:?}", limit + 1);
    assert_eq!(w.hasher.count(), u64::from(limit), "a refused attempt costs no hash work");
    // the key ignores case and padding, so the limit cannot be dodged by respelling the email
    let got = block_on(sign_in(&w.app, &format!(" {} ", email.to_uppercase()), PASSWORD));
    assert!(matches!(got, Err(CoreError::RateLimited { .. })), "{got:?}");
    // control: another email has its own count
    assert_eq!(outcome(&block_on(sign_in(&w.app, "other@example.test", "another password"))), Outcome::Ok);
}

// --- seed ------------------------------------------------------------------------------------------------------------

/// The fixture carries plaintext passwords; the database gets a PHC hash made under the configured cost.
#[test]
fn seed_stores_a_hash_not_the_password_and_the_email_normalised() {
    let w = auth_world();
    let u = block_on(w.db.user_by_email("active@example.test")).unwrap().expect("normalised email");
    let hash = u.password_hash.expect("a hash");
    assert!(hash.starts_with("$argon2id$v=19$m=32,t=1,p=1$"), "{hash}");
    assert!(!hash.contains(PASSWORD));
    assert!(u.is_active);
    assert!(block_on(w.db.user_by_email("  Active@Example.TEST ")).unwrap().is_none(), "the adapter compares exactly");
    let none = block_on(w.db.user_by_email("nopw@example.test")).unwrap().unwrap();
    assert_eq!(none.password_hash, None);
    assert!(!block_on(w.db.user_by_email("inactive@example.test")).unwrap().unwrap().is_active);
}
