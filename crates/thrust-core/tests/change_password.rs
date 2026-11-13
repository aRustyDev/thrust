//! A person changes their own password (Q-F7, issue #46, D-039): `change_password`, against the memory adapters with the real
//! argon2id at a cheap cost and a hasher that counts its work.
//!
//! What must hold: a change swaps the password of that account and no other, ends every session of that account (the caller's
//! included) and no other's, makes one new session for the caller, and costs two units of hash work (the old password checked, the new
//! one hashed); a refusal changes nothing, is said in Plane's order (a wrong old password before a weak new one), and costs what the
//! order says (a guess longer than a password may be costs none, a wrong one costs one, however weak the new one is); the old
//! password's guesses are counted in the sign-in allowance of the account, before any hashing; and any sequence of sign-ups, sign-ins
//! and changes agrees with a reference that only knows who has which password and which sessions are alive.
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
/// Refused by `password_is_strong` (a common password once its decoration is gone), and so nobody's password in these tests.
const WEAK: &str = "password123";

/// A world whose sign-in limiter is out of the way, on a clock that never moves: what most of these tests check is not the limiter's.
fn fresh() -> World {
    limited(100_000)
}

fn limited(limit: u32) -> World {
    let w = world_config(CoreConfig {
        outbox_fastpath: false,
        sign_in_rate_limit: limit,
        sign_in_rate_window_secs: 60,
        ..CoreConfig::default()
    });
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

fn caller(user_id: UserId) -> Principal {
    Principal { user_id }
}

const PW_CHARS: &str = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789!#-_";

/// A password `password_is_strong` accepts: long, varied, and not a common one (a random string is none of them).
fn strong() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PW_CHARS.chars().collect::<Vec<_>>()), 12..32)
        .prop_map(|cs| cs.into_iter().collect::<String>())
        .prop_filter("at least 5 distinct characters", |p| p.chars().collect::<BTreeSet<_>>().len() >= 5)
}

fn email() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,8}@example\\.test"
}

// --- the properties --------------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(48))]

    /// A change swaps the password of that account, ends every session of it, makes one in their place, and touches no other
    /// account or session. `by` has the same password as the target, so a change that wrote the hash to the wrong row (or to every
    /// row) shows.
    #[test]
    fn prop_a_change_swaps_one_password_ends_that_accounts_sessions_and_makes_one_new(
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
            prop_assert_eq!(session_of(&w, t), Some(target.user_id), "control: alive before the change");
        }

        w.hasher.reset();
        let got = block_on(change_password(&w.app, &caller(target.user_id), &old, &new)).unwrap();
        let PasswordChange::Changed(fresh_session) = got else {
            return Err(TestCaseError::fail(format!("not changed: {got:?}")));
        };
        prop_assert_eq!(w.hasher.count(), 2, "one unit to check the old password, one to hash the new");
        prop_assert_eq!(fresh_session.user_id, target.user_id);

        prop_assert!(matches!(block_on(sign_in(&w.app, &email, &old)), Err(CoreError::Unauthenticated)));
        prop_assert_eq!(block_on(sign_in(&w.app, &email, &new)).unwrap().user_id, target.user_id);
        for t in &target_tokens {
            prop_assert_eq!(session_of(&w, t), None, "a session the account held before the change");
        }
        prop_assert_eq!(session_of(&w, &fresh_session.token), Some(target.user_id), "the session made by the change");
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

#[derive(Debug, Clone, Copy)]
enum Old {
    Right,
    Wrong,
    TooLong,
}

#[derive(Debug, Clone, Copy)]
enum New {
    Strong,
    Weak,
}

/// Every pair of faults, each answered as Plane answers it and costing what the order says: the old password is judged before the
/// new one is looked at, so a wrong old one is a wrong old one however weak the new one is; a refusal changes nothing, and a change
/// that happens costs one unit more than one that does not (it hashes). The finite domain is exhaustive.
#[rstest]
fn a_refusal_is_said_in_planes_order_changes_nothing_and_costs_what_the_order_says(
    #[values(Old::Right, Old::Wrong, Old::TooLong)] old: Old,
    #[values(New::Strong, New::Weak)] new: New,
) {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    w.hasher.reset();
    let long = "x".repeat(1025);
    let old_text = match old {
        Old::Right => PASSWORD,
        Old::Wrong => "not the password at all",
        Old::TooLong => long.as_str(),
    };
    let new_text = match new {
        New::Strong => NEW_PASSWORD,
        New::Weak => WEAK,
    };
    let got = block_on(change_password(&w.app, &caller(s.user_id), old_text, new_text)).unwrap();
    let (expected_units, changed) = match (old, new) {
        (Old::Right, New::Strong) => (2, true),
        (Old::Right, New::Weak) => (1, false),
        (Old::Wrong, _) => (1, false),
        (Old::TooLong, _) => (0, false),
    };
    match (old, new, &got) {
        (Old::Right, New::Strong, PasswordChange::Changed(_)) => {}
        (Old::Right, New::Weak, PasswordChange::TooWeak) => {}
        (Old::Wrong | Old::TooLong, _, PasswordChange::WrongOldPassword) => {}
        _ => panic!("{old:?} {new:?}: {got:?}"),
    }
    assert_eq!(w.hasher.count(), expected_units, "{old:?} {new:?}: hash work");
    if changed {
        assert_eq!(session_of(&w, &s.token), None, "{old:?} {new:?}: the session held before is over");
        assert_eq!(block_on(sign_in(&w.app, "a@example.test", NEW_PASSWORD)).unwrap().user_id, s.user_id);
    } else {
        assert_eq!(session_of(&w, &s.token), Some(s.user_id), "{old:?} {new:?}: the session stands");
        assert_eq!(
            block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id,
            s.user_id,
            "{old:?} {new:?}"
        );
    }
}

/// Changing to the password already held is not refused (Plane does not refuse it, measured), and it ends the sessions all the same.
#[test]
fn the_password_already_held_is_a_change_like_any_other() {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    let got = block_on(change_password(&w.app, &caller(s.user_id), PASSWORD, PASSWORD)).unwrap();
    assert!(matches!(got, PasswordChange::Changed(_)), "{got:?}");
    assert_eq!(session_of(&w, &s.token), None);
    assert_eq!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id, s.user_id);
}

/// An account that single sign-on made has no password to match. It is refused as a wrong old password, with the one unit of work
/// of any other refusal (so the cost does not tell which), and the operator's reset (D-038) is what gives it one.
#[test]
fn an_account_with_no_password_cannot_answer_for_one_and_the_operator_can_give_it_one() {
    let w = fresh();
    let id = UserId(Uuid::from_u128(0xABC));
    block_on(w.app.ports.db.insert_user(&UserRecord {
        id,
        email: Some("sso@example.test".into()),
        password_hash: None,
        is_active: true,
    }))
    .unwrap();
    for guess in ["", "anything", PASSWORD] {
        w.hasher.reset();
        let got = block_on(change_password(&w.app, &caller(id), guess, NEW_PASSWORD)).unwrap();
        assert_eq!(got, PasswordChange::WrongOldPassword, "{guess:?}");
        assert_eq!(w.hasher.count(), 1, "{guess:?}: as much work as any other wrong old password");
    }
    assert!(matches!(block_on(sign_in(&w.app, "sso@example.test", NEW_PASSWORD)), Err(CoreError::Unauthenticated)));
    block_on(admin_set_password(&w.app, "sso@example.test", PASSWORD)).unwrap();
    let got = block_on(change_password(&w.app, &caller(id), PASSWORD, NEW_PASSWORD)).unwrap();
    assert!(matches!(got, PasswordChange::Changed(_)), "{got:?}");
    assert_eq!(block_on(sign_in(&w.app, "sso@example.test", NEW_PASSWORD)).unwrap().user_id, id);
}

/// A deactivated account changes nothing (its sessions do not authenticate, so this is the window between that check and the read),
/// and a caller that is nobody is not a caller.
#[rstest]
fn a_caller_who_is_inactive_or_unknown_is_unauthenticated_and_changes_nothing(#[values(true, false)] unknown: bool) {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    let who = if unknown {
        caller(UserId(Uuid::from_u128(0xDEAD)))
    } else {
        assert!(block_on(set_user_active(&w.app, s.user_id, false)).unwrap());
        caller(s.user_id)
    };
    w.hasher.reset();
    let got = block_on(change_password(&w.app, &who, PASSWORD, NEW_PASSWORD));
    assert!(matches!(got, Err(CoreError::Unauthenticated)), "{got:?}");
    assert_eq!(w.hasher.count(), 0, "no work for a caller who is nobody");
    if !unknown {
        assert!(block_on(set_user_active(&w.app, s.user_id, true)).unwrap());
        assert_eq!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id, s.user_id);
    }
}

/// The old password's guesses are counted in the account's sign-in allowance, in both directions, before any hashing: a person has
/// one allowance for guessing the password, whichever door the guess comes in by.
#[rstest]
fn the_old_password_shares_the_sign_in_allowance_and_is_counted_before_any_hashing(
    #[values(true, false)] sign_ins_first: bool,
) {
    let w = limited(3);
    let s = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    w.hasher.reset();
    for _ in 0..3 {
        if sign_ins_first {
            assert!(matches!(
                block_on(sign_in(&w.app, "a@example.test", "wrong guess")),
                Err(CoreError::Unauthenticated)
            ));
        } else {
            let got = block_on(change_password(&w.app, &caller(s.user_id), "wrong guess", NEW_PASSWORD)).unwrap();
            assert_eq!(got, PasswordChange::WrongOldPassword);
        }
    }
    assert_eq!(w.hasher.count(), 3, "control: each allowed guess cost one unit");
    w.hasher.reset();
    if sign_ins_first {
        // the allowance is spent by sign-ins: the right old password is refused, and costs nothing
        let got = block_on(change_password(&w.app, &caller(s.user_id), PASSWORD, NEW_PASSWORD));
        assert!(matches!(got, Err(CoreError::RateLimited { .. })), "{got:?}");
    } else {
        let got = block_on(sign_in(&w.app, "a@example.test", PASSWORD));
        assert!(matches!(got, Err(CoreError::RateLimited { .. })), "{got:?}");
    }
    assert_eq!(w.hasher.count(), 0, "a limited attempt is not hashed");
    assert_eq!(session_of(&w, &s.token), Some(s.user_id), "nothing changed");
}

/// The salt comes from the Rng port: a source that cannot deliver must not leave a change half done (the old password and its
/// session stand).
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
    let got = block_on(change_password(&app, &caller(s.user_id), PASSWORD, NEW_PASSWORD));
    assert!(matches!(got, Err(CoreError::Internal(_))), "{got:?}");
    assert_eq!(session_of(&w, &s.token), Some(s.user_id), "the session stands");
    assert_eq!(block_on(sign_in(&w.app, "a@example.test", PASSWORD)).unwrap().user_id, s.user_id);
}

// --- a model ---------------------------------------------------------------------------------------------------------

/// The passwords the model picks from: three strong ones, then a weak one that nobody has (so as an old password it is wrong, and as a
/// new one it is refused).
const PWS: [&str; 4] =
    ["correct horse battery staple", "another long enough phrase", "third strong passphrase 42", WEAK];
const WEAK_AT: usize = 3;
const WHO: [&str; 3] = ["u0@example.test", "u1@example.test", "u2@example.test"];

#[derive(Debug, Clone)]
enum Op {
    SignUp {
        who: usize,
        pw: usize,
    },
    SignIn {
        who: usize,
        pw: usize,
    },
    Change {
        who: usize,
        old: usize,
        new: usize,
    },
    /// Is the `n`th session made so far (modulo how many there are) alive?
    Check {
        n: usize,
    },
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let who = 0..WHO.len();
    let strong_pw = 0..WEAK_AT;
    let any_pw = 0..PWS.len();
    prop::collection::vec(
        prop_oneof![
            3 => (who.clone(), strong_pw.clone()).prop_map(|(who, pw)| Op::SignUp { who, pw }),
            4 => (who.clone(), any_pw.clone()).prop_map(|(who, pw)| Op::SignIn { who, pw }),
            6 => (who, any_pw.clone(), any_pw).prop_map(|(who, old, new)| Op::Change { who, old, new }),
            3 => any::<usize>().prop_map(|n| Op::Check { n }),
        ],
        1..40,
    )
}

/// What a step was, so that a control can require the generator to reach every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    SignedUp,
    SignedIn,
    SignInRefused,
    Changed,
    /// A change that ended a session that was alive: the one that tells a change that ends sessions from one that does not.
    ChangedAndEndedASession,
    WrongOld,
    TooWeak,
    SessionLive,
    SessionEnded,
}
const ALL: [Class; 9] = [
    Class::SignedUp,
    Class::SignedIn,
    Class::SignInRefused,
    Class::Changed,
    Class::ChangedAndEndedASession,
    Class::WrongOld,
    Class::TooWeak,
    Class::SessionLive,
    Class::SessionEnded,
];

/// Run `ops` against a world and against a reference that knows only who has which password and which sessions are alive.
fn run(ops: &[Op]) -> Result<BTreeSet<Class>, TestCaseError> {
    let w = fresh();
    let mut password: [Option<usize>; 3] = [None; 3];
    let mut ids: [Option<UserId>; 3] = [None; 3];
    // (who, the session's token, its user, alive in the reference)
    let mut sessions: Vec<(usize, String, UserId, bool)> = Vec::new();
    let mut seen = BTreeSet::new();
    for op in ops {
        match *op {
            Op::SignUp { who, pw } => {
                if password[who].is_none() {
                    let s = block_on(sign_up(&w.app, WHO[who], PWS[pw]))
                        .map_err(|e| TestCaseError::fail(format!("{op:?}: {e:?}")))?;
                    password[who] = Some(pw);
                    ids[who] = Some(s.user_id);
                    sessions.push((who, s.token, s.user_id, true));
                    seen.insert(Class::SignedUp);
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
            Op::Change { who, old, new } => {
                // only an account that exists has a caller
                let (Some(id), Some(current)) = (ids[who], password[who]) else { continue };
                let got = block_on(change_password(&w.app, &caller(id), PWS[old], PWS[new]))
                    .map_err(|e| TestCaseError::fail(format!("{op:?}: {e:?}")))?;
                if old != current {
                    prop_assert_eq!(got, PasswordChange::WrongOldPassword, "{:?}", op);
                    seen.insert(Class::WrongOld);
                } else if new == WEAK_AT {
                    prop_assert_eq!(got, PasswordChange::TooWeak, "{:?}", op);
                    seen.insert(Class::TooWeak);
                } else {
                    let PasswordChange::Changed(s) = got else {
                        return Err(TestCaseError::fail(format!("{op:?}: {got:?}")));
                    };
                    password[who] = Some(new);
                    if sessions.iter().any(|x| x.0 == who && x.3) {
                        seen.insert(Class::ChangedAndEndedASession);
                    }
                    for x in sessions.iter_mut().filter(|x| x.0 == who) {
                        x.3 = false;
                    }
                    sessions.push((who, s.token, s.user_id, true));
                    seen.insert(Class::Changed);
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
    fn model_change_sequences_agree_with_a_reference_of_passwords_and_sessions(ops in ops()) {
        run(&ops)?;
    }
}

/// Control for the model: its generator reaches every kind of step, so no check of it is vacuous (a model whose strategy never
/// changes the password of an account that has a live session would pass whatever a change did to sessions).
#[test]
fn prop_the_change_model_generator_reaches_every_class() {
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
