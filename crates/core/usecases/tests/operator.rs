//! The operator's way in (Q-F7, issue #46): `admin_create_user` and `admin_set_password`, against the memory adapters with the
//! real argon2id at a cheap cost and a hasher that counts its work.
//!
//! What must hold: an account the operator makes signs in with its password and with no other, and making it again, in any
//! spelling of the email, is a conflict that costs nothing; a reset swaps the password of that account and no other, ends every
//! session of that account and no other, costs one unit of hash work, and leaves the account as active or as inactive as it
//! was; a refused reset changes nothing and costs none; and any sequence of makes, resets and sign-ins agrees with a reference
//! that only knows who has which password.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const PASSWORD: &str = "correct horse battery staple";
const NEW_PASSWORD: &str = "a different strong passphrase";

/// A world whose sign-in limiter is out of the way: these tests sign in far more often than a person would in a minute, on a clock
/// that never moves, and what they check is not the limiter's.
fn fresh() -> World {
    let w = world_config(CoreConfig { outbox_fastpath: false, sign_in_rate_limit: 100_000, ..CoreConfig::default() });
    w.hasher.reset();
    w
}

fn bearer(token: &str) -> Credentials<'_> {
    Credentials { bearer: Some(token), ..Credentials::default() }
}

/// Whether `token` is a live session, and whose.
fn session_of(w: &World, token: &str) -> Option<UserId> {
    block_on(authenticate_request(&w.app, &bearer(token))).ok().map(|a| a.principal.user_id)
}

const PW_CHARS: &str = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789!#-_";

/// A password `password_is_strong` accepts: long, varied, and not a common one (a random string is none of them).
fn strong() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PW_CHARS.chars().collect::<Vec<_>>()), 12..32)
        .prop_map(|cs| cs.into_iter().collect::<String>())
        .prop_filter("at least 5 distinct characters", |p| {
            p.chars().collect::<BTreeSet<_>>().len() >= MIN_DISTINCT_FOR_TESTS
        })
}
const MIN_DISTINCT_FOR_TESTS: usize = 5;

fn email() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,8}@example\\.test"
}

// --- the properties --------------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(48))]

    /// Made by the operator: the one password signs in, no other does, and the same email in another spelling is a conflict.
    #[test]
    fn prop_an_account_the_operator_makes_signs_in_with_its_password_and_only_that_one(
        email in email(),
        pw in strong(),
        other in strong(),
    ) {
        prop_assume!(pw != other);
        let w = fresh();
        let id = block_on(admin_create_user(&w.app, &format!(" {} ", email.to_uppercase()), &pw)).unwrap();
        prop_assert_eq!(w.hasher.count(), 1, "one unit of hash work");
        let s = block_on(sign_in(&w.app, &email, &pw)).unwrap();
        prop_assert_eq!(s.user_id, id);
        prop_assert!(matches!(block_on(sign_in(&w.app, &email, &other)), Err(CoreError::Unauthenticated)));
        // control: a second make is refused, and refusing it costs nothing
        w.hasher.reset();
        let again = block_on(admin_create_user(&w.app, &email, &other));
        prop_assert!(matches!(again, Err(CoreError::Conflict(_))), "{:?}", again);
        prop_assert_eq!(w.hasher.count(), 0);
    }

    /// A reset swaps the password of that account, ends every session of it, and touches no other account or session. `by`
    /// has the same password as the target, so a reset that wrote the hash to the wrong row (or to every row) shows.
    #[test]
    fn prop_a_reset_swaps_one_password_and_ends_that_users_sessions_and_no_ones_else(
        email in email(),
        by in email(),
        old in strong(),
        new in strong(),
        extra_sessions in 0usize..4,
        bystander_sessions in 0usize..3,
    ) {
        prop_assume!(old != new && email != by);
        let w = fresh();
        let target = block_on(sign_up(&w.app, &email, &old)).unwrap();
        let mut target_tokens = vec![target.token.clone()];
        for _ in 0..extra_sessions {
            target_tokens.push(block_on(sign_in(&w.app, &email, &old)).unwrap().token);
        }
        let bystander = block_on(sign_up(&w.app, &by, &old)).unwrap();
        let mut bystander_tokens = vec![bystander.token.clone()];
        for _ in 0..bystander_sessions {
            bystander_tokens.push(block_on(sign_in(&w.app, &by, &old)).unwrap().token);
        }
        for t in &target_tokens {
            prop_assert_eq!(session_of(&w, t), Some(target.user_id), "control: alive before the reset");
        }

        w.hasher.reset();
        let id = block_on(admin_set_password(&w.app, &format!(" {} ", email.to_uppercase()), &new)).unwrap();
        prop_assert_eq!(id, target.user_id);
        prop_assert_eq!(w.hasher.count(), 1, "one unit of hash work");

        prop_assert!(matches!(block_on(sign_in(&w.app, &email, &old)), Err(CoreError::Unauthenticated)));
        prop_assert_eq!(block_on(sign_in(&w.app, &email, &new)).unwrap().user_id, id);
        for t in &target_tokens {
            prop_assert_eq!(session_of(&w, t), None, "a session of the reset user");
        }
        for t in &bystander_tokens {
            prop_assert_eq!(session_of(&w, t), Some(bystander.user_id), "a session of someone else");
        }
        prop_assert_eq!(
            block_on(sign_in(&w.app, &by, &old)).unwrap().user_id,
            bystander.user_id,
            "someone else's password"
        );
    }
}

// --- the cases the properties do not choose --------------------------------------------------------------------------

/// Resetting a password does not change whether the account may sign in, either way: a deactivated user stays deactivated (and a
/// reactivated one finds the new password in place), an active one stays active.
#[rstest]
fn a_reset_leaves_the_account_as_active_as_it_was(#[values(true, false)] active: bool) {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    if !active {
        assert!(block_on(set_user_active(&w.app, s.user_id, false)).unwrap());
    }
    assert_eq!(block_on(admin_set_password(&w.app, "a@example.test", NEW_PASSWORD)).unwrap(), s.user_id);
    let got = block_on(sign_in(&w.app, "a@example.test", NEW_PASSWORD));
    if active {
        assert_eq!(got.unwrap().user_id, s.user_id);
    } else {
        assert!(matches!(got, Err(CoreError::Unauthenticated)), "a password makes nobody active: {got:?}");
        // control: the new password is the one in place, so it works once the account is active again
        assert!(block_on(set_user_active(&w.app, s.user_id, true)).unwrap());
        assert_eq!(block_on(sign_in(&w.app, "a@example.test", NEW_PASSWORD)).unwrap().user_id, s.user_id);
    }
    assert!(matches!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)), Err(CoreError::Unauthenticated)));
}

#[derive(Debug, Clone, Copy)]
enum Refusal {
    WeakPassword,
    WeakPasswordOfAnUnknownEmail,
    UnknownEmail,
    NotAnEmail,
}

/// A refused reset changes nothing and does no hash work: the old password still signs in, the session still stands. A weak
/// password is refused before the account is looked at, so it does not say whether the account exists.
#[rstest]
fn a_refused_reset_changes_nothing_and_does_no_hash_work(
    #[values(Refusal::WeakPassword, Refusal::WeakPasswordOfAnUnknownEmail, Refusal::UnknownEmail, Refusal::NotAnEmail)]
    why: Refusal,
) {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    w.hasher.reset();
    let (email, password, invalid) = match why {
        Refusal::WeakPassword => ("a@example.test", "password123", true),
        Refusal::WeakPasswordOfAnUnknownEmail => ("nobody@example.test", "short", true),
        Refusal::UnknownEmail => ("nobody@example.test", NEW_PASSWORD, false),
        Refusal::NotAnEmail => ("not-an-email", NEW_PASSWORD, false),
    };
    let got = block_on(admin_set_password(&w.app, email, password));
    match (&got, invalid) {
        (Err(CoreError::Invalid(_)), true) | (Err(CoreError::NotFound), false) => {}
        _ => panic!("{why:?}: {got:?}"),
    }
    assert_eq!(w.hasher.count(), 0, "{why:?}: a refusal costs no hash work");
    assert_eq!(session_of(&w, &s.token), Some(s.user_id), "{why:?}: the session stands");
    assert_eq!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id, s.user_id, "{why:?}");
    // nothing was made for an email that had no account
    assert!(!block_on(email_exists(&w.app, "nobody@example.test")).unwrap(), "{why:?}");
}

/// An account that single sign-on made has no password. A reset gives it one, which is the way in for a person whose provider is
/// unreachable (and the operator's way to give an SSO account a fallback).
#[test]
fn a_reset_gives_an_account_with_no_password_one() {
    let w = fresh();
    let id = UserId(Uuid::from_u128(0xABC));
    block_on(w.app.ports.db.insert_user(&UserRecord {
        id,
        email: Some("sso@example.test".into()),
        password_hash: None,
        is_active: true,
    }))
    .unwrap();
    assert!(matches!(block_on(sign_in(&w.app, "sso@example.test", PASSWORD)), Err(CoreError::Unauthenticated)));
    assert_eq!(block_on(admin_set_password(&w.app, "sso@example.test", PASSWORD)).unwrap(), id);
    assert_eq!(block_on(sign_in(&w.app, "sso@example.test", PASSWORD)).unwrap().user_id, id);
}

/// The salt comes from the Rng port: a source that cannot deliver must neither make an account with a predictable hash nor leave a
/// reset half done (the old password and its session stand).
#[test]
fn a_dead_random_source_is_an_internal_error_and_changes_nothing() {
    struct Dead;
    impl thrust_ports::Rng for Dead {
        fn fill(&self, _buf: &mut [u8]) -> Result<(), thrust_ports::RngError> {
            Err(thrust_ports::RngError("down (test)".into()))
        }
    }
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    let mut app = w.app.clone();
    app.ports.rng = Arc::new(Dead);

    assert!(matches!(block_on(admin_create_user(&app, "new@example.test", PASSWORD)), Err(CoreError::Internal(_))));
    assert!(!block_on(email_exists(&w.app, "new@example.test")).unwrap());

    assert!(matches!(block_on(admin_set_password(&app, "a@example.test", NEW_PASSWORD)), Err(CoreError::Internal(_))));
    assert_eq!(session_of(&w, &s.token), Some(s.user_id), "the session stands");
    assert_eq!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id, s.user_id);
}

// --- a model ---------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Create {
        who: usize,
        pw: usize,
    },
    Reset {
        who: usize,
        pw: usize,
    },
    SignIn {
        who: usize,
        pw: usize,
    },
    /// Is the `n`th session made so far (modulo how many there are) alive?
    Check {
        n: usize,
    },
}

const WHO: [&str; 3] = ["u0@example.test", "u1@example.test", "u2@example.test"];
const PWS: [&str; 3] = ["correct horse battery staple", "another long enough phrase", "third strong passphrase 42"];

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let who = 0..WHO.len();
    let pw = 0..PWS.len();
    prop::collection::vec(
        prop_oneof![
            3 => (who.clone(), pw.clone()).prop_map(|(who, pw)| Op::Create { who, pw }),
            3 => (who.clone(), pw.clone()).prop_map(|(who, pw)| Op::Reset { who, pw }),
            4 => (who, pw).prop_map(|(who, pw)| Op::SignIn { who, pw }),
            3 => any::<usize>().prop_map(|n| Op::Check { n }),
        ],
        1..40,
    )
}

/// What a step was, so that a control can require the generator to reach every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Created,
    CreateConflict,
    Reset,
    ResetNoAccount,
    SignedIn,
    SignInRefused,
    SessionLive,
    SessionEnded,
}
const ALL: [Class; 8] = [
    Class::Created,
    Class::CreateConflict,
    Class::Reset,
    Class::ResetNoAccount,
    Class::SignedIn,
    Class::SignInRefused,
    Class::SessionLive,
    Class::SessionEnded,
];

/// Run `ops` against a world and against a reference that knows only who has which password and which sessions are alive.
fn run(ops: &[Op]) -> Result<BTreeSet<Class>, TestCaseError> {
    let w = fresh();
    let mut password: [Option<usize>; 3] = [None; 3];
    // (who, the session's token, its user, alive in the reference)
    let mut sessions: Vec<(usize, String, UserId, bool)> = Vec::new();
    let mut seen = BTreeSet::new();
    for op in ops {
        match *op {
            Op::Create { who, pw } => {
                let got = block_on(admin_create_user(&w.app, WHO[who], PWS[pw]));
                if password[who].is_none() {
                    prop_assert!(got.is_ok(), "{:?}: {:?}", op, got);
                    password[who] = Some(pw);
                    seen.insert(Class::Created);
                } else {
                    prop_assert!(matches!(got, Err(CoreError::Conflict(_))), "{:?}: {:?}", op, got);
                    seen.insert(Class::CreateConflict);
                }
            }
            Op::Reset { who, pw } => {
                let got = block_on(admin_set_password(&w.app, WHO[who], PWS[pw]));
                if password[who].is_some() {
                    prop_assert!(got.is_ok(), "{:?}: {:?}", op, got);
                    password[who] = Some(pw);
                    for s in sessions.iter_mut().filter(|s| s.0 == who) {
                        s.3 = false;
                    }
                    seen.insert(Class::Reset);
                } else {
                    prop_assert!(matches!(got, Err(CoreError::NotFound)), "{:?}: {:?}", op, got);
                    seen.insert(Class::ResetNoAccount);
                }
            }
            Op::SignIn { who, pw } => {
                let got = block_on(sign_in(&w.app, WHO[who], PWS[pw]));
                if password[who] == Some(pw) {
                    let s = got.map_err(|e| TestCaseError::fail(format!("{op:?}: {e:?}")))?;
                    sessions.push((who, s.token, s.user_id, true));
                    seen.insert(Class::SignedIn);
                } else {
                    prop_assert!(matches!(got, Err(CoreError::Unauthenticated)), "{:?}: {:?}", op, got);
                    seen.insert(Class::SignInRefused);
                }
            }
            Op::Check { n } => {
                if sessions.is_empty() {
                    continue;
                }
                let (_, token, user, alive) = &sessions[n % sessions.len()];
                let got = session_of(&w, token);
                if *alive {
                    prop_assert_eq!(got, Some(*user), "{:?}", op);
                    seen.insert(Class::SessionLive);
                } else {
                    prop_assert_eq!(got, None, "{:?}", op);
                    seen.insert(Class::SessionEnded);
                }
            }
        }
    }
    Ok(seen)
}

proptest! {
    #![proptest_config(proptest_config(64))]

    #[test]
    fn model_operator_sequences_agree_with_a_reference_of_passwords_and_sessions(ops in ops()) {
        run(&ops)?;
    }
}

/// Control for the model: its generator reaches every kind of step, so no check of it is vacuous (a model whose strategy never
/// resets an account that has a session would pass whatever a reset did to sessions).
#[test]
fn prop_the_operator_model_generator_reaches_every_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let strategy = ops();
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..300 {
        let sequence = strategy.new_tree(&mut runner).unwrap().current();
        seen.extend(run(&sequence).unwrap());
    }
    let missing: Vec<_> = ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never reached: {missing:?}");
}
