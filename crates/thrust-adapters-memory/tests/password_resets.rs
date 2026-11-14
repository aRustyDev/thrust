//! The password-reset tokens in the memory adapter (migration 0017, F-A03, D-041): a mailed token's digest, the user it is for and
//! when it was made. Exhaustive over the finite domains (which token is asked for, how old the user's last token is against how long
//! the cooldown is). The Postgres adapter is held to the same answers by the generated sequences in
//! `thrust-adapters-selfhosted/tests/model` (`ResetInsert`, `ResetFind`, `ResetTake`, `ResetCooldown`, `ResetLife`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::{DateTime, TimeDelta};
use thrust_contracts::*;
use thrust_ports::*;

fn now() -> Timestamp {
    DateTime::from_timestamp(1_760_000_000, 0).unwrap()
}

fn secs(n: i64) -> TimeDelta {
    TimeDelta::seconds(n)
}

fn hash(n: u8) -> TokenHash {
    TokenHash([n; 32])
}

const OWNER: UserId = UserId(Uuid::from_u128(7));
const OTHER: UserId = UserId(Uuid::from_u128(8));

fn reset(n: u8, user: UserId, ttl: i64, created: Timestamp) -> NewPasswordReset {
    NewPasswordReset { token_hash: hash(n), user_id: user, created_at: created, expires_at: created + secs(ttl) }
}

/// What a stored token reads back as.
fn row(user: UserId, ttl: i64, created: Timestamp) -> PasswordReset {
    PasswordReset { user_id: user, expires_at: created + secs(ttl) }
}

/// A far-past cooldown start: no token blocks an insert made with it.
fn no_cooldown() -> Timestamp {
    now() + secs(1_000_000)
}

/// A stored token is found (and found again: finding takes nothing), is taken exactly once with what was stored, and is gone for the
/// next caller; a token that was never stored is neither found nor taken, and taking another leaves the stored one.
#[rstest]
fn a_token_is_found_without_being_taken_and_taken_exactly_once(
    #[values(1, 2)] stored: u8,
    #[values(1, 2, 3)] asked: u8,
) {
    let db = MemoryDatabase::new();
    assert!(block_on(db.insert_password_reset(&reset(stored, OWNER, 3600, now()), no_cooldown())).unwrap());
    let want = (asked == stored).then(|| row(OWNER, 3600, now()));
    assert_eq!(block_on(db.find_password_reset(&hash(asked))).unwrap(), want, "find, stored {stored}, asked {asked}");
    assert_eq!(block_on(db.find_password_reset(&hash(asked))).unwrap(), want, "a find takes nothing");
    assert_eq!(block_on(db.take_password_reset(&hash(asked))).unwrap(), want, "take, stored {stored}, asked {asked}");
    assert_eq!(block_on(db.take_password_reset(&hash(asked))).unwrap(), None, "a second take must find nothing");
    assert_eq!(block_on(db.find_password_reset(&hash(asked))).unwrap(), None, "and nothing is there to find");
    // taking another token leaves the stored one
    if asked != stored {
        assert_eq!(block_on(db.find_password_reset(&hash(stored))).unwrap(), Some(row(OWNER, 3600, now())));
    }
}

/// The cooldown: a token the user already has, made `age` s before this insert, refuses it when the cooldown is at least that long
/// (the boundary is inclusive: a token made exactly `cooldown` s ago still blocks). A refusal stores nothing and leaves the old token
/// working; an insert that is let through replaces it. Another user is never held back by this user's token.
#[rstest]
fn the_cooldown_is_inclusive_and_belongs_to_the_user(
    #[values(0, 1, 59, 60, 61, 3600)] age: i64,
    #[values(0, 1, 60)] cooldown: i64,
) {
    let db = MemoryDatabase::new();
    assert!(block_on(db.insert_password_reset(&reset(1, OWNER, 7200, now()), no_cooldown())).unwrap());
    let at = now() + secs(age);
    let blocked = cooldown >= age;

    // another user is never blocked, and the owner's token is not replaced by it
    assert!(
        block_on(db.insert_password_reset(&reset(9, OTHER, 7200, at), at - secs(cooldown))).unwrap(),
        "another user"
    );
    assert!(block_on(db.find_password_reset(&hash(1))).unwrap().is_some(), "the owner's token survives another user's");

    let stored = block_on(db.insert_password_reset(&reset(2, OWNER, 7200, at), at - secs(cooldown))).unwrap();
    assert_eq!(stored, !blocked, "age {age}, cooldown {cooldown}");
    let old = block_on(db.find_password_reset(&hash(1))).unwrap();
    let new = block_on(db.find_password_reset(&hash(2))).unwrap();
    if blocked {
        assert_eq!(
            (old, new),
            (Some(row(OWNER, 7200, now())), None),
            "a refusal stores nothing and leaves the old token working"
        );
    } else {
        assert_eq!((old, new), (None, Some(row(OWNER, 7200, at))), "an insert replaces the user's older token");
    }
    // the other user's token is untouched either way
    assert_eq!(block_on(db.find_password_reset(&hash(9))).unwrap(), Some(row(OTHER, 7200, at)));
}

/// An insert first drops the tokens that expired by its `created_at` (so an expired token neither blocks nor survives), and a digest
/// that is already stored is a `Conflict` that changes nothing.
#[test]
fn an_insert_drops_the_tokens_that_expired_before_it_and_refuses_a_digest_that_exists() {
    let db = MemoryDatabase::new();
    assert!(block_on(db.insert_password_reset(&reset(1, OWNER, 60, now()), no_cooldown())).unwrap());
    assert!(block_on(db.insert_password_reset(&reset(2, OTHER, 600, now()), no_cooldown())).unwrap());
    // 100 s later: the owner's token has expired, the other user's has not. A cooldown that would reach back to the owner's token does
    // not hold the owner back, because the expired token was dropped first.
    let later = now() + secs(100);
    assert!(block_on(db.insert_password_reset(&reset(3, OWNER, 600, later), now())).unwrap());
    assert_eq!(block_on(db.find_password_reset(&hash(1))).unwrap(), None, "the expired token was dropped");
    assert_eq!(block_on(db.find_password_reset(&hash(2))).unwrap(), Some(row(OTHER, 600, now())));
    assert_eq!(block_on(db.find_password_reset(&hash(3))).unwrap(), Some(row(OWNER, 600, later)));
    // a digest that exists is refused, and the token that holds it is as it was
    let dup = block_on(db.insert_password_reset(&reset(2, OTHER, 600, later + secs(1)), no_cooldown()));
    assert!(matches!(dup, Err(DbError::Conflict(_))), "{dup:?}");
    assert_eq!(block_on(db.find_password_reset(&hash(2))).unwrap(), Some(row(OTHER, 600, now())));
}
